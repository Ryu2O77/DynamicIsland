//! Captura de pantalla y grabador de pantalla con APIs de Windows.
//!
//! - Captura: GDI (`BitBlt`) y PNG con el codificador de Windows Imaging Component
//!   expuesto por WinRT (`BitmapEncoder`).
//! - Grabación: GDI a 30 fps y MP4/H.264 con Media Foundation (`IMFSinkWriter`).
//!
//! No usa dependencias externas ni programas de terceros.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapEncoder, BitmapPixelFormat};
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
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
pub fn primary_screen() -> (i32, i32, i32, i32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN) & !1;
        let h = GetSystemMetrics(SM_CYSCREEN) & !1;
        (0, 0, w.max(16), h.max(16))
    }
}

/// Captura un rectángulo de la pantalla en un búfer reutilizable.
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

pub fn videos_dir() -> PathBuf {
    home_dir("Videos")
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

// ------------------------------------------------------------ grabador (Media Foundation)

struct RecorderInner {
    stop: Arc<AtomicBool>,
    err: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

/// Grabador activo. Al soltarlo sin llamar a `stop` también se finaliza el MP4.
pub struct Recorder {
    inner: RecorderInner,
    path: PathBuf,
    started: Instant,
}

impl Recorder {
    /// Arranca la grabación del monitor principal. Devuelve error si Media
    /// Foundation o el codificador H.264 no están disponibles.
    pub fn start(path: PathBuf, fps: u32) -> Result<Recorder, String> {
        let fps = fps.clamp(5, 60);
        if let Some(dir) = path.parent() {
            ensure_dir(dir)?;
        }
        let (x, y, w, h) = primary_screen();
        if w < 16 || h < 16 {
            return Err("no se pudo medir la pantalla".into());
        }

        let stop = Arc::new(AtomicBool::new(false));
        let err = Arc::new(Mutex::new(None));
        let (tx, rx) = channel::<Result<(), String>>();
        let path2 = path.clone();
        let (stop2, err2) = (stop.clone(), err.clone());
        let handle = std::thread::spawn(move || {
            let mut ready = false;
            let res = record_loop(x, y, w, h, fps, &path2, &stop2, || {
                ready = true;
                let _ = tx.send(Ok(()));
            });
            if !ready {
                let msg = res.err().unwrap_or_else(|| "no se pudo iniciar la grabación".into());
                let _ = tx.send(Err(msg.clone()));
                *err2.lock().unwrap() = Some(msg);
            } else if let Err(e) = res {
                *err2.lock().unwrap() = Some(e);
            }
        });

        match rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok(Recorder {
                inner: RecorderInner { stop, err, handle: Some(handle) },
                path,
                started: Instant::now(),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::Release);
                Err("Media Foundation no respondió (¿paquete de códecs instalado?)".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err("el grabador se cerró inesperadamente".into()),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Error ocurrido durante la grabación, si lo hubo.
    pub fn error(&self) -> Option<String> {
        self.inner.err.lock().ok().and_then(|e| e.clone())
    }

    /// Detiene la grabación, espera a que se cierre el MP4 y devuelve la ruta.
    pub fn stop(mut self) -> Result<PathBuf, String> {
        self.stop_inner();
        let e = self.inner.err.lock().ok().and_then(|mut e| e.take());
        match e {
            Some(e) => Err(e),
            None => Ok(self.path.clone()),
        }
    }

    fn stop_inner(&mut self) {
        self.inner.stop.store(true, Ordering::Release);
        if let Some(h) = self.inner.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn record_loop(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    fps: u32,
    path: &Path,
    stop: &AtomicBool,
    ready: impl FnOnce(),
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| format!("Media Foundation: {e}"))?;
    }

    let writer = match create_writer(path, w as u32, h as u32, fps) {
        Ok(w) => w,
        Err(e) => {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
            return Err(e);
        }
    };
    ready();

    let frame = Duration::from_secs_f64(1.0 / fps as f64);
    let frame_ns = 10_000_000i64 / fps as i64;
    let start = Instant::now();
    let mut next = start;
    let mut last_t = -1i64;
    let mut px: Vec<u8> = Vec::new();
    let mut result = Ok(());

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        if let Err(e) = grab_into(x, y, w, h, &mut px) {
            result = Err(e);
            break;
        }
        let t = ((start.elapsed().as_secs_f64() * 1e7) as i64).max(last_t + 1);
        last_t = t;
        if let Err(e) = write_frame(&writer, &px, t, frame_ns) {
            result = Err(e);
            break;
        }
        next += frame;
        let after = Instant::now();
        if next < after {
            next = after;
        }
    }

    if result.is_ok() {
        if let Err(e) = unsafe { writer.Finalize() } {
            result = Err(format!("cerrar MP4: {e}"));
        }
    }
    drop(writer);
    unsafe {
        let _ = MFShutdown();
        CoUninitialize();
    }
    result
}

fn create_writer(path: &Path, w: u32, h: u32, fps: u32) -> Result<IMFSinkWriter, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, None)
            .map_err(|e| format!("crear MP4: {e}"))?;

        let bitrate = ((w as u64 * h as u64 * fps as u64) / 8).clamp(2_000_000, 24_000_000) as u32;
        let size = ((w as u64) << 32) | h as u64;
        let rate = ((fps as u64) << 32) | 1;

        let out = MFCreateMediaType().map_err(|e| format!("tipo de salida: {e}"))?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|e| e.to_string())?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate).map_err(|e| e.to_string())?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| e.to_string())?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(|e| e.to_string())?;
        out.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(|e| e.to_string())?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1).map_err(|e| e.to_string())?;
        let stream = writer.AddStream(&out).map_err(|e| format!("pista de vídeo: {e}"))?;

        let input = MFCreateMediaType().map_err(|e| format!("tipo de entrada: {e}"))?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(|e| e.to_string())?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| e.to_string())?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(|e| e.to_string())?;
        input.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(|e| e.to_string())?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1).map_err(|e| e.to_string())?;
        // Positivo = imagen de arriba a abajo (GDI ya entrega las filas así).
        input.SetUINT32(&MF_MT_DEFAULT_STRIDE, (w * 4) as u32).map_err(|e| e.to_string())?;
        writer
            .SetInputMediaType(stream, &input, None)
            .map_err(|e| format!("entrada H.264: {e}"))?;
        writer.BeginWriting().map_err(|e| format!("iniciar MP4: {e}"))?;
        Ok(writer)
    }
}

fn write_frame(writer: &IMFSinkWriter, px: &[u8], time: i64, dur: i64) -> Result<(), String> {
    unsafe {
        let buf = MFCreateMemoryBuffer(px.len() as u32).map_err(|e| format!("búfer de vídeo: {e}"))?;
        let mut p: *mut u8 = std::ptr::null_mut();
        buf.Lock(&mut p, None, None).map_err(|e| format!("bloquear búfer: {e}"))?;
        std::ptr::copy_nonoverlapping(px.as_ptr(), p, px.len());
        buf.Unlock().map_err(|e| format!("desbloquear búfer: {e}"))?;
        buf.SetCurrentLength(px.len() as u32).map_err(|e| e.to_string())?;

        let sample = MFCreateSample().map_err(|e| format!("muestra de vídeo: {e}"))?;
        sample.AddBuffer(&buf).map_err(|e| e.to_string())?;
        sample.SetSampleTime(time).map_err(|e| e.to_string())?;
        sample.SetSampleDuration(dur).map_err(|e| e.to_string())?;
        writer.WriteSample(0, &sample).map_err(|e| format!("escribir fotograma: {e}"))?;
    }
    Ok(())
}

/// Ruta de un PNG nuevo dentro de `Pictures\DynamicIsland`.
pub fn new_shot_path() -> Result<PathBuf, String> {
    let dir = pictures_dir();
    ensure_dir(&dir)?;
    Ok(dir.join(format!("Captura_{}.png", stamp())))
}

/// Ruta de un MP4 nuevo dentro de `Videos\DynamicIsland`.
pub fn new_video_path() -> Result<PathBuf, String> {
    let dir = videos_dir();
    ensure_dir(&dir)?;
    Ok(dir.join(format!("Grabacion_{}.mp4", stamp())))
}
