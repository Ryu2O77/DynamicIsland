//! Captura de pantalla con APIs de Windows.
//!
//! - Captura: GDI (`BitBlt`) y PNG con el codificador de Windows Imaging Component
//!   expuesto por WinRT (`BitmapEncoder`).
//!
//! No usa dependencias externas ni programas de terceros.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapEncoder, BitmapPixelFormat};
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Fotograma capturado, BGRA y de arriba a abajo.
pub struct Shot {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// Rectángulo de toda la pantalla virtual (todos los monitores).
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Rectángulo del monitor principal, con lados pares (H.264 lo necesita).
pub fn grab_into(x: i32, y: i32, w: i32, h: i32, out: &mut Vec<u8>) -> Result<(), String> {
    if w <= 0 || h <= 0 {
        return Err("tamaño de captura no válido".into());
    }
    let len = (w as usize) * (h as usize) * 4;
    out.resize(len, 0);
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC falló".into());
        }
        let mem = CreateCompatibleDC(screen);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return Err(format!("CreateDIBSection: {e}"));
            }
        };
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        let ok = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        if ok.is_ok() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(bits as *const u8, out.as_mut_ptr(), len);
        }
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        ok.map_err(|e| format!("BitBlt: {e}"))?;
    }
    Ok(())
}

pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Result<Shot, String> {
    let mut px = Vec::new();
    grab_into(x, y, w, h, &mut px)?;
    Ok(Shot { w: w as u32, h: h as u32, px })
}

/// Codifica el fotograma a PNG (WIC a través de WinRT).
pub fn save_png(shot: &Shot, path: &Path) -> Result<(), String> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let stream = InMemoryRandomAccessStream::new().map_err(|e| format!("flujo de memoria: {e}"))?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::PngEncoderId().map_err(|e| e.to_string())?, &stream)
            .map_err(|e| format!("codificador PNG: {e}"))?
            .get()
            .map_err(|e| format!("codificador PNG: {e}"))?;
        encoder
            .SetPixelData(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Ignore,
                shot.w,
                shot.h,
                96.0,
                96.0,
                &shot.px,
            )
            .map_err(|e| format!("píxeles PNG: {e}"))?;
        encoder.FlushAsync().map_err(|e| format!("PNG: {e}"))?.get().map_err(|e| format!("PNG: {e}"))?;
        let size = stream.Size().map_err(|e| format!("PNG: {e}"))? as usize;
        stream.Seek(0).map_err(|e| format!("PNG: {e}"))?;
        let reader = DataReader::CreateDataReader(&stream).map_err(|e| format!("PNG: {e}"))?;
        reader.LoadAsync(size as u32).map_err(|e| format!("PNG: {e}"))?.get().map_err(|e| format!("PNG: {e}"))?;
        let mut buf = vec![0u8; size];
        reader.ReadBytes(&mut buf).map_err(|e| format!("PNG: {e}"))?;
        std::fs::write(path, buf).map_err(|e| format!("escribir {}: {e}", path.display()))?;
    }
    Ok(())
}

fn home_dir(sub: &str) -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    base.join(sub).join("DynamicIsland")
}

pub fn pictures_dir() -> PathBuf {
    home_dir("Pictures")
}

pub fn ensure_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("crear {}: {e}", p.display()))
}

pub fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Ruta de un PNG nuevo dentro de `Pictures\DynamicIsland`.
pub fn new_shot_path() -> Result<PathBuf, String> {
    let dir = pictures_dir();
    ensure_dir(&dir)?;
    Ok(dir.join(format!("Captura_{}.png", stamp())))
}
