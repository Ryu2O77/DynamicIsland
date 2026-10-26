#![windows_subsystem = "windows"]

//! Dynamic Island para Windows (v0.4)
//! Paginas (Musica, Reloj, Timer, Crono, Calculadora, Ajustes), configuracion
//! con interruptores guardada en %APPDATA%\DynamicIsland\config.ini.
//! Render por software + texto GDI; los timers solo corren cuando hace falta.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, Interface, PCWSTR};
use windows::Foundation::TimeSpan;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
    ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProps,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlayStatus,
};
use windows::Storage::Streams::IRandomAccessStream;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimeAsFileTime};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, SetFocus, TrackMouseEvent, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, TME_LEAVE,
    TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NOTIFYICONDATAW, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::*;

// ------------------------------------------------------------ constantes

const CW: f32 = 500.0; // lienzo (logico)
const CH: f32 = 350.0;
const TOP: f32 = 8.0; // separacion con el borde superior
const CS: usize = 96; // lado de la caratula decodificada

const WM_APP_MEDIA: u32 = 0x8000 + 1;
const WM_TRAY: u32 = 0x8000 + 2;
const WM_APP_FRAME: u32 = 0x8000 + 3;
const WM_MOUSELEAVE_: u32 = 0x02A3;
const MA_NOACTIVATE_: isize = 3;

const T_CLOCK: usize = 2;
const T_VIS: usize = 3;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ANIM_ON: AtomicBool = AtomicBool::new(false);
static FRAME_PENDING: AtomicBool = AtomicBool::new(false);
static ANIM_THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
static MUSIC_ON: AtomicBool = AtomicBool::new(true);
static COVER_ON: AtomicBool = AtomicBool::new(true);

type Rgb = (f32, f32, f32);
const WHITE: Rgb = (255.0, 255.0, 255.0);
const BLACK: Rgb = (0.0, 0.0, 0.0);
const ORANGE: Rgb = (255.0, 159.0, 10.0);
const GREEN: Rgb = (48.0, 209.0, 88.0);

// funciones que se pueden activar/desactivar
const F_MUSIC: usize = 0;
const F_COVER: usize = 1;
const F_VIS: usize = 2;
const F_CLOCK: usize = 3;
const F_TIMER: usize = 4;
const F_STOPWATCH: usize = 5;
const F_CALC: usize = 6;
const F_BATT: usize = 7;
const F_FULLSCREEN: usize = 8;
const F_VOLUME: usize = 9;
const F_SHADOW: usize = 10;
const F_COUNT: usize = 11;

const FEATURES: [(&str, &str); F_COUNT] = [
    ("music", "Música"),
    ("cover", "Carátula"),
    ("visualizer", "Visualizador"),
    ("clock", "Reloj"),
    ("timer", "Temporizador"),
    ("stopwatch", "Cronómetro"),
    ("calculator", "Calculadora"),
    ("battery", "Aviso de batería"),
    ("fullscreen", "Ocultar en fullscreen"),
    ("volume", "Volumen con la rueda"),
    ("shadow", "Sombra"),
];

// teclas de la calculadora: (etiqueta, estilo 0=numero 1=funcion 2=operador)
const KEYS: [(&str, u8); 20] = [
    ("AC", 1), ("(", 1), (")", 1), ("÷", 2),
    ("7", 0), ("8", 0), ("9", 0), ("×", 2),
    ("4", 0), ("5", 0), ("6", 0), ("−", 2),
    ("1", 0), ("2", 0), ("3", 0), ("+", 2),
    ("←", 1), ("0", 0), (".", 0), ("=", 2),
];

// ------------------------------------------------------------ datos

struct Cover {
    px: Vec<u8>, // BGRA, CS x CS
    accent: Rgb,
}

#[derive(Clone, Default)]
struct Media {
    has: bool,
    playing: bool,
    title: String,
    artist: String,
    pos: f64, // segundos, medido en `stamp`
    dur: f64,
    stamp: Option<Instant>,
    cover: Option<Arc<Cover>>,
}

impl Media {
    fn cur_pos(&self) -> f64 {
        let mut p = self.pos;
        if self.playing {
            if let Some(st) = self.stamp {
                p += st.elapsed().as_secs_f64();
            }
        }
        if self.dur > 0.0 {
            p.min(self.dur)
        } else {
            p
        }
    }
}

fn differs(a: &Media, b: &Media) -> bool {
    a.has != b.has
        || a.playing != b.playing
        || a.title != b.title
        || a.artist != b.artist
        || (a.dur - b.dur).abs() > 0.5
        || (!b.playing && (a.pos - b.pos).abs() > 0.8)
        || a.cover.is_some() != b.cover.is_some()
        || match (&a.cover, &b.cover) {
            (Some(x), Some(y)) => !Arc::ptr_eq(x, y),
            _ => false,
        }
}

#[derive(Clone, Copy, Default)]
struct Batt {
    has: bool,
    pct: u8,
    ac: bool,
}

fn read_batt() -> Batt {
    unsafe {
        let mut p = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut p).is_err() {
            return Batt::default();
        }
        Batt {
            has: p.BatteryFlag & 128 == 0 && p.BatteryFlag != 255 && p.BatteryLifePercent <= 100,
            pct: p.BatteryLifePercent.min(100),
            ac: p.ACLineStatus == 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Collapsed,
    Expanded,
    Toast,
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Music,
    Clock,
    Timer,
    Stopwatch,
    Calc,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Prev,
    Next,
    Play,
    Pause,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Tab(Page),
    Pin,
    Prev,
    Toggle,
    Next,
    Seek(f32),
    Preset(u64),
    TimerAdd,
    TimerToggle,
    TimerReset,
    SwToggle,
    SwLap,
    SwReset,
    Key(usize),
    Feature(usize),
    Autostart,
}

#[derive(Clone, Copy)]
enum WKind {
    Tab,
    Pin,
    Pill,
    Key(u8),
    Toggle,
    Icon(Icon),
    Big(Icon),
    Bar,
}

#[derive(Clone, Copy)]
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: WKind,
    act: Act,
    label: &'static str,
    on: bool,
}

struct Toast {
    battery: bool, // icono: bateria o reloj
    text: String,
    until: Instant,
}

#[derive(Clone, Copy)]
enum Cmd {
    Prev,
    Toggle,
    Next,
    Seek(f64),
}

struct App {
    hwnd: HWND,
    mem: HDC,
    bits: *mut u32,
    pw: i32,
    ph: i32,
    pos: POINT,
    s: f32,
    font_main: HFONT,
    font_small: HFONT,
    font_big: HFONT,
    cur: [f32; 2],
    vel: [f32; 2],
    mode: Mode,
    page: Page,
    prev_view: Option<(Mode, Page)>,
    layout_key: (Mode, Page, bool, bool),
    fade: f32,
    hover: bool,
    pinned: bool,
    hover_act: Option<Act>,
    tracking: bool,
    animating: bool,
    vis_on: bool,
    hidden: bool,
    last: Instant,
    t0: Instant,
    collapsed_at: Instant,
    media: Arc<Mutex<Media>>,
    cov: Vec<u8>,
    sh: Vec<u8>,
    last_minute: u16,
    toast: Option<Toast>,
    batt: Batt,
    prev_ac: Option<bool>,
    low_warned: bool,
    features: [bool; F_COUNT],
    autostart: bool,
    timer_set: u64,
    timer_end: Option<Instant>,
    timer_paused: Option<f32>,
    sw_start: Option<Instant>,
    sw_acc: f64,
    laps: Vec<f64>,
    calc_expr: String,
    calc_prev: String,
    calc_done: bool,
    calc_err: bool,
    prev_fg: isize,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<F: FnOnce(&mut App)>(f: F) {
    APP.with(|c| {
        if let Ok(mut b) = c.try_borrow_mut() {
            if let Some(a) = b.as_mut() {
                f(a);
            }
        }
    });
}

// ------------------------------------------------------------ configuracion

fn config_path() -> Option<std::path::PathBuf> {
    let d = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(d).join("DynamicIsland").join("config.ini"))
}

fn load_features() -> [bool; F_COUNT] {
    let mut f = [true; F_COUNT];
    if let Some(p) = config_path() {
        if let Ok(txt) = std::fs::read_to_string(p) {
            for line in txt.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    for (i, (key, _)) in FEATURES.iter().enumerate() {
                        if *key == k.trim() {
                            f[i] = v.trim() != "0";
                        }
                    }
                }
            }
        }
    }
    f
}

fn save_features(f: &[bool; F_COUNT]) {
    if let Some(p) = config_path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut s = String::new();
        for (i, (k, _)) in FEATURES.iter().enumerate() {
            s.push_str(&format!("{}={}\n", k, if f[i] { 1 } else { 0 }));
        }
        let _ = std::fs::write(p, s);
    }
}

// ------------------------------------------------------------ calculadora (motor)

fn parse_expr(c: &[char], i: &mut usize) -> Option<f64> {
    let mut v = parse_term(c, i)?;
    while *i < c.len() && (c[*i] == '+' || c[*i] == '-') {
        let op = c[*i];
        *i += 1;
        let r = parse_term(c, i)?;
        if op == '+' {
            v += r;
        } else {
            v -= r;
        }
    }
    Some(v)
}

fn parse_term(c: &[char], i: &mut usize) -> Option<f64> {
    let mut v = parse_factor(c, i)?;
    while *i < c.len() && (c[*i] == '*' || c[*i] == '/') {
        let op = c[*i];
        *i += 1;
        let r = parse_factor(c, i)?;
        if op == '*' {
            v *= r;
        } else {
            if r == 0.0 {
                return None;
            }
            v /= r;
        }
    }
    Some(v)
}

fn parse_factor(c: &[char], i: &mut usize) -> Option<f64> {
    if *i >= c.len() {
        return None;
    }
    if c[*i] == '-' {
        *i += 1;
        return parse_factor(c, i).map(|x| -x);
    }
    if c[*i] == '(' {
        *i += 1;
        let v = parse_expr(c, i)?;
        if *i < c.len() && c[*i] == ')' {
            *i += 1;
            return Some(v);
        }
        return None;
    }
    let start = *i;
    while *i < c.len() && (c[*i].is_ascii_digit() || c[*i] == '.') {
        *i += 1;
    }
    if start == *i {
        return None;
    }
    let s: String = c[start..*i].iter().collect();
    s.parse::<f64>().ok()
}

/// Evalua cerrando los parentesis que falten.
fn eval_loose(expr: &str) -> Option<f64> {
    let open = expr.chars().filter(|&c| c == '(').count() as i32;
    let close = expr.chars().filter(|&c| c == ')').count() as i32;
    if close > open || expr.is_empty() {
        return None;
    }
    let mut e = expr.to_string();
    for _ in 0..(open - close) {
        e.push(')');
    }
    let c: Vec<char> = e.chars().collect();
    let mut i = 0;
    let v = parse_expr(&c, &mut i)?;
    if i != c.len() || !v.is_finite() {
        return None;
    }
    Some(v)
}

fn fmt_num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if v.abs() >= 1e15 || v.abs() < 1e-9 {
        return format!("{:e}", v);
    }
    let mut s = format!("{:.10}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

fn disp_expr(e: &str) -> String {
    e.chars()
        .map(|c| match c {
            '*' => '×',
            '/' => '÷',
            '-' => '−',
            c => c,
        })
        .collect()
}

fn tail_chars(s: &str, n: usize) -> String {
    let v: Vec<char> = s.chars().collect();
    if v.len() <= n {
        s.to_string()
    } else {
        let mut o = String::from("…");
        o.extend(v[v.len() - (n - 1)..].iter());
        o
    }
}

fn key_char(k: &str) -> Option<char> {
    match k {
        "÷" => Some('/'),
        "×" => Some('*'),
        "−" => Some('-'),
        "+" | "." | "(" | ")" => k.chars().next(),
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => d.chars().next(),
        _ => None,
    }
}

// ------------------------------------------------------------ medios (SMTC)

fn now_ft() -> i64 {
    unsafe {
        let ft = GetSystemTimeAsFileTime();
        ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
    }
}

fn fetch_cover(props: &MediaProps) -> windows::core::Result<Cover> {
    let thumb = props.Thumbnail()?;
    let stream = thumb.OpenReadAsync()?.get()?;
    let stream: IRandomAccessStream = stream.cast()?;
    let dec = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let tf = BitmapTransform::new()?;
    tf.SetScaledWidth(CS as u32)?;
    tf.SetScaledHeight(CS as u32)?;
    let pd = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Straight,
            &tf,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .get()?;
    let px: Vec<u8> = pd.DetachPixelData()?.to_vec();
    if px.len() < CS * CS * 4 {
        return Err(windows::core::Error::from(E_FAIL));
    }
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for i in (0..CS * CS).step_by(7) {
        b += px[i * 4] as u64;
        g += px[i * 4 + 1] as u64;
        r += px[i * 4 + 2] as u64;
        n += 1;
    }
    let (r, g, b) = (r as f32 / n as f32, g as f32 / n as f32, b as f32 / n as f32);
    let m = r.max(g).max(b);
    let accent = if m < 8.0 {
        WHITE
    } else {
        let k = 215.0 / m;
        ((r * k).min(255.0), (g * k).min(255.0), (b * k).min(255.0))
    };
    Ok(Cover { px, accent })
}

struct Poller {
    mgr: Manager,
    key: (String, String),
    cover: Option<Arc<Cover>>,
    tries: u32,
}

impl Poller {
    fn poll(&mut self, cover_on: bool) -> windows::core::Result<Media> {
        let s = self.mgr.GetCurrentSession()?;
        let p = s.TryGetMediaPropertiesAsync()?.get()?;
        let title = p.Title()?.to_string();
        let artist = p.Artist()?.to_string();
        let key = (title.clone(), artist.clone());
        if key != self.key {
            self.key = key;
            self.cover = None;
            self.tries = 0;
        }
        if !cover_on {
            self.cover = None;
            self.tries = 0;
        } else if self.cover.is_none() && self.tries < 6 {
            self.tries += 1;
            self.cover = fetch_cover(&p).ok().map(Arc::new);
        }
        let playing = s.GetPlaybackInfo()?.PlaybackStatus()? == PlayStatus::Playing;

        let (mut pos, mut dur) = (0.0f64, 0.0f64);
        if let Ok(tl) = s.GetTimelineProperties() {
            let g = |r: windows::core::Result<TimeSpan>| r.map(|t| t.Duration).unwrap_or(0);
            let start = g(tl.StartTime());
            let end = g(tl.EndTime());
            let mut position = g(tl.Position());
            if playing {
                let last = tl.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                let now = now_ft();
                if last > 0 && now > last {
                    position += now - last;
                }
            }
            dur = ((end - start).max(0)) as f64 / 1e7;
            pos = ((position - start).max(0)) as f64 / 1e7;
            if dur > 0.0 {
                pos = pos.min(dur);
            }
        }
        Ok(Media {
            has: true,
            playing,
            title,
            artist,
            pos,
            dur,
            stamp: Some(Instant::now()),
            cover: self.cover.clone(),
        })
    }
}

fn media_thread(hwnd: isize, media: Arc<Mutex<Media>>) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let mgr = loop {
        match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => break m,
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    let mut poller = Poller { mgr, key: (String::new(), String::new()), cover: None, tries: 0 };
    loop {
        let music = MUSIC_ON.load(Ordering::Relaxed);
        let new = if music {
            poller.poll(COVER_ON.load(Ordering::Relaxed)).unwrap_or_default()
        } else {
            Media::default()
        };
        let changed = {
            let mut m = media.lock().unwrap();
            let c = differs(&m, &new);
            *m = new;
            c
        };
        if changed {
            unsafe {
                let _ = PostMessageW(HWND(hwnd as _), WM_APP_MEDIA, WPARAM(0), LPARAM(0));
            }
        }
        std::thread::sleep(Duration::from_millis(if music { 700 } else { 1500 }));
    }
}

fn media_cmd(cmd: Cmd) {
    std::thread::spawn(move || unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let run = || -> windows::core::Result<()> {
            let mgr = Manager::RequestAsync()?.get()?;
            let s = mgr.GetCurrentSession()?;
            match cmd {
                Cmd::Prev => {
                    s.TrySkipPreviousAsync()?.get()?;
                }
                Cmd::Toggle => {
                    s.TryTogglePlayPauseAsync()?.get()?;
                }
                Cmd::Next => {
                    s.TrySkipNextAsync()?.get()?;
                }
                Cmd::Seek(secs) => {
                    s.TryChangePlaybackPositionAsync((secs * 1e7) as i64)?.get()?;
                }
            }
            Ok(())
        };
        let _ = run();
    });
}

// ------------------------------------------------------------ sistema

fn is_fullscreen() -> bool {
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(s) => s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
            Err(_) => false,
        }
    }
}

fn autostart_enabled() -> bool {
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            0,
            KEY_READ,
            &mut k,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let r = RegQueryValueExW(k, w!("DynamicIsland"), None, None, None, None);
        let _ = RegCloseKey(k);
        r == ERROR_SUCCESS
    }
}

fn set_autostart(on: bool) {
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            0,
            KEY_SET_VALUE,
            &mut k,
        ) != ERROR_SUCCESS
        {
            return;
        }
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let s = format!("\"{}\"", exe.display());
                let mut v: Vec<u16> = s.encode_utf16().collect();
                v.push(0);
                let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
                let _ = RegSetValueExW(k, w!("DynamicIsland"), 0, REG_SZ, Some(bytes));
            }
        } else {
            let _ = RegDeleteValueW(k, w!("DynamicIsland"));
        }
        let _ = RegCloseKey(k);
    }
}

fn key_tap(vk: u8, times: i32) {
    unsafe {
        for _ in 0..times {
            keybd_event(vk, 0, KEYBD_EVENT_FLAGS(0), 0);
            keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
        }
    }
}

// ------------------------------------------------------------ dibujo (software)

#[inline]
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn sdf_rr(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    let (ax, ay) = (qx.max(0.0), qy.max(0.0));
    let outer = if ax > 0.0 && ay > 0.0 { (ax * ax + ay * ay).sqrt() } else { ax + ay };
    outer + qx.max(qy).min(0.0) - r
}

#[inline]
fn blend(dst: &mut u32, c: Rgb, a: f32) {
    let r = ((*dst >> 16) & 255) as f32;
    let g = ((*dst >> 8) & 255) as f32;
    let b = (*dst & 255) as f32;
    let nr = (r + (c.0 - r) * a + 0.5) as u32;
    let ng = (g + (c.1 - g) * a + 0.5) as u32;
    let nb = (b + (c.2 - b) * a + 0.5) as u32;
    *dst = (nr.min(255) << 16) | (ng.min(255) << 8) | nb.min(255);
}

struct Canvas<'a> {
    b: &'a mut [u32],
    pw: usize,
    ph: usize,
}

impl<'a> Canvas<'a> {
    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, alpha: f32) {
        if alpha <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = r.min(w / 2.0).min(h / 2.0);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
        let xa = (x.floor() as i32 - 1).max(0) as usize;
        let xb = ((x + w).ceil() as i32 + 1).clamp(0, self.pw as i32) as usize;
        let ya = (y.floor() as i32 - 1).max(0) as usize;
        let yb = ((y + h).ceil() as i32 + 1).clamp(0, self.ph as i32) as usize;
        for yy in ya..yb {
            for xx in xa..xb {
                let d = sdf_rr(xx as f32 + 0.5, yy as f32 + 0.5, cx, cy, hx, hy, r);
                let a = (0.5 - d).clamp(0.0, 1.0) * alpha;
                if a > 0.0 {
                    blend(&mut self.b[yy * self.pw + xx], c, a);
                }
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, alpha: f32) {
        self.rrect(cx - r, cy - r, r * 2.0, r * 2.0, r, c, alpha);
    }

    /// Poligono convexo con borde suave.
    fn poly(&mut self, pts: &[(f32, f32)], c: Rgb, alpha: f32) {
        let n = pts.len();
        if n < 3 || n > 8 || alpha <= 0.0 {
            return;
        }
        let mut area = 0.0;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        let sg = if area > 0.0 { 1.0 } else { -1.0 };
        let mut edges = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); 8];
        let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (ex, ey) = (b.0 - a.0, b.1 - a.1);
            let l = ex.hypot(ey).max(1e-6);
            edges[i] = (a.0, a.1, sg * ey / l, -sg * ex / l);
            minx = minx.min(a.0);
            maxx = maxx.max(a.0);
            miny = miny.min(a.1);
            maxy = maxy.max(a.1);
        }
        let xa = (minx.floor() as i32 - 1).max(0) as usize;
        let xb = (maxx.ceil() as i32 + 1).clamp(0, self.pw as i32) as usize;
        let ya = (miny.floor() as i32 - 1).max(0) as usize;
        let yb = (maxy.ceil() as i32 + 1).clamp(0, self.ph as i32) as usize;
        for yy in ya..yb {
            for xx in xa..xb {
                let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                let mut d = f32::MIN;
                for e in edges.iter().take(n) {
                    d = d.max(e.2 * (px - e.0) + e.3 * (py - e.1));
                }
                let a = (0.5 - d).clamp(0.0, 1.0) * alpha;
                if a > 0.0 {
                    blend(&mut self.b[yy * self.pw + xx], c, a);
                }
            }
        }
    }

    fn cover(&mut self, cv: &Cover, x: f32, y: f32, size: f32, r: f32, alpha: f32) {
        let (cx, cy) = (x + size / 2.0, y + size / 2.0);
        let (hx, hy) = (size / 2.0 - r, size / 2.0 - r);
        let xa = (x.floor() as i32 - 1).max(0) as usize;
        let xb = ((x + size).ceil() as i32 + 1).clamp(0, self.pw as i32) as usize;
        let ya = (y.floor() as i32 - 1).max(0) as usize;
        let yb = ((y + size).ceil() as i32 + 1).clamp(0, self.ph as i32) as usize;
        let k = CS as f32 / size;
        for yy in ya..yb {
            for xx in xa..xb {
                let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                let a = (0.5 - sdf_rr(px, py, cx, cy, hx, hy, r)).clamp(0.0, 1.0) * alpha;
                if a <= 0.0 {
                    continue;
                }
                let u = ((px - x) * k - 0.5).clamp(0.0, CS as f32 - 1.001);
                let v = ((py - y) * k - 0.5).clamp(0.0, CS as f32 - 1.001);
                let (ix, iy) = (u as usize, v as usize);
                let (fx, fy) = (u - ix as f32, v - iy as f32);
                let mut c = [0.0f32; 3];
                for (dy, wy) in [(0usize, 1.0 - fy), (1, fy)] {
                    for (dx, wx) in [(0usize, 1.0 - fx), (1, fx)] {
                        let i = ((iy + dy) * CS + ix + dx) * 4;
                        let wgt = wx * wy;
                        c[0] += cv.px[i + 2] as f32 * wgt;
                        c[1] += cv.px[i + 1] as f32 * wgt;
                        c[2] += cv.px[i] as f32 * wgt;
                    }
                }
                blend(&mut self.b[yy * self.pw + xx], (c[0], c[1], c[2]), a);
            }
        }
    }

    fn bars(&mut self, x: f32, yc: f32, s: f32, t: f32, playing: bool, c: Rgb, f: f32) {
        for i in 0..4 {
            let fi = i as f32;
            let amp = if playing {
                0.3 + 0.7 * ((t * (2.3 + 0.7 * fi) + fi * 1.9).sin() * 0.5 + 0.5)
            } else {
                0.2
            };
            let hh = 16.0 * s * amp;
            self.rrect(x + fi * 5.5 * s, yc - hh / 2.0, 3.0 * s, hh, 1.5 * s, c, f);
        }
    }

    fn battery(&mut self, x: f32, y: f32, s: f32, b: Batt, f: f32) {
        let body = (210.0, 210.0, 214.0);
        self.rrect(x, y, 24.0 * s, 12.0 * s, 3.5 * s, body, f);
        self.rrect(x + 1.6 * s, y + 1.6 * s, 20.8 * s, 8.8 * s, 2.2 * s, BLACK, f);
        let col = if b.ac {
            GREEN
        } else if b.pct <= 20 {
            (255.0, 69.0, 58.0)
        } else {
            (235.0, 235.0, 240.0)
        };
        let len = (18.0 * b.pct as f32 / 100.0).max(1.0);
        self.rrect(x + 3.0 * s, y + 3.0 * s, len * s, 6.0 * s, 1.2 * s, col, f);
        self.rrect(x + 24.8 * s, y + 3.8 * s, 2.2 * s, 4.4 * s, 1.1 * s, body, f);
    }

    fn clock_icon(&mut self, cx: f32, cy: f32, r: f32, s: f32, c: Rgb, f: f32) {
        self.circle(cx, cy, r, c, f);
        self.circle(cx, cy, r - 1.7 * s, BLACK, f);
        self.rrect(cx - 0.8 * s, cy - r * 0.55, 1.6 * s, r * 0.55 + 0.8 * s, 0.8 * s, c, f);
        self.rrect(cx - 0.8 * s, cy - 0.8 * s, r * 0.5 + 0.8 * s, 1.6 * s, 0.8 * s, c, f);
    }

    fn icon(&mut self, ic: Icon, cx: f32, cy: f32, s: f32, c: Rgb, f: f32) {
        match ic {
            Icon::Prev => {
                self.rrect(cx - 9.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, 1.0 * s, c, f);
                self.poly(&[(cx + 9.0 * s, cy - 8.5 * s), (cx + 9.0 * s, cy + 8.5 * s), (cx - 5.0 * s, cy)], c, f);
            }
            Icon::Next => {
                self.rrect(cx + 6.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, 1.0 * s, c, f);
                self.poly(&[(cx - 9.0 * s, cy - 8.5 * s), (cx - 9.0 * s, cy + 8.5 * s), (cx + 5.0 * s, cy)], c, f);
            }
            Icon::Play => {
                self.poly(&[(cx - 5.0 * s, cy - 8.0 * s), (cx - 5.0 * s, cy + 8.0 * s), (cx + 8.5 * s, cy)], c, f);
            }
            Icon::Pause => {
                self.rrect(cx - 6.0 * s, cy - 7.0 * s, 4.2 * s, 14.0 * s, 1.2 * s, c, f);
                self.rrect(cx + 1.8 * s, cy - 7.0 * s, 4.2 * s, 14.0 * s, 1.2 * s, c, f);
            }
        }
    }

    fn switch(&mut self, x: f32, y: f32, s: f32, on: bool, f: f32) {
        let (w, h) = (36.0 * s, 20.0 * s);
        let col = if on { GREEN } else { (72.0, 72.0, 80.0) };
        self.rrect(x, y, w, h, h / 2.0, col, f);
        let kx = if on { x + w - h / 2.0 } else { x + h / 2.0 };
        self.circle(kx, y + h / 2.0, 8.0 * s, WHITE, f);
    }
}

/// Un pixel del borde de la pildora: cobertura + brillo del borde interior.
#[inline]
#[allow(clippy::too_many_arguments)]
fn pill_pixel(x: usize, y: usize, pw: usize, cx: f32, cy: f32, hx: f32, hy: f32, r: f32, s: f32, cov: &mut [u8], buf: &mut [u32]) {
    let d = sdf_rr(x as f32 + 0.5, y as f32 + 0.5, cx, cy, hx, hy, r);
    let a = (0.5 - d).clamp(0.0, 1.0);
    if a > 0.0 {
        let i = y * pw + x;
        cov[i] = (a * 255.0) as u8;
        let depth = (-d).max(0.0);
        let ring = (1.0 - depth / (1.2 * s)).max(0.0) * 0.14;
        let g = (ring * 255.0) as u32;
        buf[i] = (g << 16) | (g << 8) | g;
    }
}

fn make_font(height: i32, weight: i32) -> HFONT {
    let mut face = [0u16; 32];
    for (i, c) in "Segoe UI".encode_utf16().enumerate() {
        face[i] = c;
    }
    let lf = LOGFONTW {
        lfHeight: -height,
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: ANTIALIASED_QUALITY,
        lfFaceName: face,
        ..Default::default()
    };
    unsafe { CreateFontIndirectW(&lf) }
}

fn fmt_time(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn fmt_timer(secs: f32) -> String {
    let t = secs.max(0.0).ceil() as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{:02}:{:02}", t / 60, t % 60)
    }
}

fn fmt_sw(secs: f64) -> String {
    let tenths = (secs.max(0.0) * 10.0) as u64;
    let s = tenths / 10;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}.{}", s / 60, s % 60, tenths % 10)
    }
}

// ------------------------------------------------------------ app: logica

impl App {
    fn feat(&self, i: usize) -> bool {
        self.features[i]
    }

    fn page_enabled(&self, p: Page) -> bool {
        match p {
            Page::Music => self.feat(F_MUSIC),
            Page::Clock => self.feat(F_CLOCK),
            Page::Timer => self.feat(F_TIMER),
            Page::Stopwatch => self.feat(F_STOPWATCH),
            Page::Calc => self.feat(F_CALC),
            Page::Settings => true,
        }
    }

    fn home_page(&self, m: &Media) -> Page {
        if self.feat(F_MUSIC) && m.has {
            return Page::Music;
        }
        for p in [Page::Clock, Page::Music, Page::Timer, Page::Stopwatch, Page::Calc] {
            if self.page_enabled(p) {
                return p;
            }
        }
        Page::Settings
    }

    fn ensure_page(&mut self) {
        if !self.page_enabled(self.page) {
            let m = self.media.lock().unwrap().clone();
            self.page = self.home_page(&m);
        }
    }

    fn timer_active(&self) -> bool {
        self.timer_end.is_some() || self.timer_paused.is_some()
    }

    fn timer_secs(&self) -> f32 {
        match self.timer_end {
            Some(e) => e.saturating_duration_since(Instant::now()).as_secs_f32(),
            None => self.timer_paused.unwrap_or(self.timer_set as f32),
        }
    }

    fn sw_elapsed(&self) -> f64 {
        self.sw_acc + self.sw_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    /// 0 temporizador, 1 cronometro, 2 musica, 3 reloj, 4 vacio
    fn collapsed_view(&self, m: &Media) -> u8 {
        if self.timer_active() {
            0
        } else if self.sw_start.is_some() {
            1
        } else if m.has {
            2
        } else if self.feat(F_CLOCK) {
            3
        } else {
            4
        }
    }

    /// Hay algo en pantalla que cambia varias veces por segundo.
    fn live_visible(&self) -> bool {
        if self.hidden {
            return false;
        }
        let m = self.media.lock().unwrap();
        let vis = m.has && m.playing && self.feat(F_VIS);
        let sw = self.sw_start.is_some();
        match self.mode {
            Mode::Collapsed => {
                if self.timer_active() {
                    false
                } else if sw {
                    true
                } else {
                    m.has && vis
                }
            }
            Mode::Expanded => match self.page {
                Page::Music => vis,
                Page::Stopwatch => sw,
                _ => false,
            },
            Mode::Toast => false,
        }
    }

    fn target(&self, m: &Media) -> (f32, f32) {
        match self.mode {
            Mode::Toast => (280.0, 46.0),
            Mode::Expanded => match self.page {
                Page::Music => {
                    if m.has {
                        (420.0, 158.0)
                    } else {
                        (420.0, 100.0)
                    }
                }
                Page::Clock => (420.0, 128.0),
                Page::Timer => (420.0, 156.0),
                Page::Stopwatch => (420.0, 150.0),
                Page::Calc => (420.0, 304.0),
                Page::Settings => (420.0, 224.0),
            },
            Mode::Collapsed => match self.collapsed_view(m) {
                0 | 1 => (132.0, 34.0),
                2 => (226.0, 34.0),
                3 => (120.0, 34.0),
                _ => (96.0, 30.0),
            },
        }
    }

    fn pill_rect(&self) -> (f32, f32, f32, f32) {
        let s = self.s;
        let w = (self.cur[0] * s).clamp(1.0, self.pw as f32);
        let h = (self.cur[1] * s).clamp(1.0, self.ph as f32 - TOP * s);
        ((self.pw as f32 - w) / 2.0, TOP * s, w, h)
    }

    fn start_anim(&mut self) {
        if !self.animating && !self.hidden {
            self.animating = true;
            self.last = Instant::now();
            ANIM_ON.store(true, Ordering::Release);
            if let Some(t) = ANIM_THREAD.get() {
                t.unpark();
            }
        }
    }

    fn refresh_mode(&mut self) {
        let new = if self.toast.is_some() {
            Mode::Toast
        } else if self.hover || self.pinned {
            Mode::Expanded
        } else {
            Mode::Collapsed
        };
        let kp = if new == Mode::Expanded { self.page } else { Page::Clock };
        let has = self.media.lock().unwrap().has;
        let active = self.timer_active() || self.sw_start.is_some();
        let key = (new, kp, has, active);
        let (om, op) = (self.layout_key.0, self.layout_key.1);
        self.mode = new;
        if key != self.layout_key {
            self.layout_key = key;
            self.fade = 0.0;
            if (om, op) != (new, kp) {
                self.prev_view = Some((om, op));
            }
        }
        if new != Mode::Expanded {
            self.hover_act = None;
        }
        self.start_anim();
    }

    fn changed(&mut self) {
        self.ensure_page();
        self.refresh_mode();
        self.update_vis_timer();
        if !self.animating {
            self.render();
        }
    }

    fn show_toast(&mut self, battery: bool, text: String, secs: u64) {
        self.toast = Some(Toast { battery, text, until: Instant::now() + Duration::from_secs(secs) });
        self.refresh_mode();
    }

    fn update_vis_timer(&mut self) {
        let need = self.live_visible();
        unsafe {
            if need && !self.vis_on {
                SetTimer(self.hwnd, T_VIS, 80, None);
                self.vis_on = true;
            } else if !need && self.vis_on {
                let _ = KillTimer(self.hwnd, T_VIS);
                self.vis_on = false;
            }
        }
    }

    fn on_media(&mut self) {
        self.update_vis_timer();
        self.refresh_mode();
        if !self.hidden {
            self.render();
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.05);
        self.last = now;
        let m = self.media.lock().unwrap().clone();
        let (tw, th) = self.target(&m);
        let t = [tw, th];
        let (k, c) = (260.0f32, 26.0f32);
        let mut settled = true;
        let mut rem = dt;
        while rem > 0.0 {
            let hs = rem.min(1.0 / 240.0);
            for i in 0..2 {
                let a = k * (t[i] - self.cur[i]) - c * self.vel[i];
                self.vel[i] += a * hs;
                self.cur[i] += self.vel[i] * hs;
            }
            rem -= hs;
        }
        for i in 0..2 {
            if (t[i] - self.cur[i]).abs() > 0.15 || self.vel[i].abs() > 0.5 {
                settled = false;
            }
        }
        self.fade = (self.fade + dt / 0.25).min(1.0);
        if self.fade < 1.0 {
            settled = false;
        }
        if settled {
            self.cur = t;
            self.vel = [0.0, 0.0];
            self.animating = false;
            ANIM_ON.store(false, Ordering::Release);
        }
        self.render();
    }

    fn vis_tick(&mut self) {
        if !self.animating && self.live_visible() {
            self.render();
        } else if !self.live_visible() {
            self.update_vis_timer();
        }
    }

    fn clock_tick(&mut self) {
        let fs = self.feat(F_FULLSCREEN) && is_fullscreen();
        if fs != self.hidden {
            self.hidden = fs;
            unsafe {
                let _ = ShowWindow(self.hwnd, if fs { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            self.update_vis_timer();
            if !fs {
                self.refresh_mode();
                self.render();
            }
        }
        if self.hidden {
            return;
        }
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let now = Instant::now();
        let mut redraw = false;

        // bateria
        let b = read_batt();
        self.batt = b;
        if self.feat(F_BATT) && b.has {
            if let Some(prev) = self.prev_ac {
                if prev != b.ac {
                    let t = if b.ac {
                        format!("Cargando · {}%", b.pct)
                    } else {
                        format!("Con batería · {}%", b.pct)
                    };
                    self.show_toast(true, t, 3);
                }
            }
            self.prev_ac = Some(b.ac);
            if !b.ac && b.pct <= 20 && !self.low_warned {
                self.low_warned = true;
                self.show_toast(true, format!("Batería baja · {}%", b.pct), 5);
            }
            if b.ac || b.pct > 25 {
                self.low_warned = false;
            }
        } else if b.has {
            self.prev_ac = Some(b.ac);
        }

        // temporizador
        if let Some(end) = self.timer_end {
            if now >= end {
                self.timer_end = None;
                self.timer_paused = None;
                unsafe {
                    let _ = MessageBeep(MB_OK);
                }
                self.show_toast(false, "¡Tiempo!".to_string(), 5);
                self.update_vis_timer();
            } else if self.mode == Mode::Collapsed || (self.mode == Mode::Expanded && self.page == Page::Timer) {
                redraw = true;
            }
        }

        // fin de aviso
        if let Some(t) = &self.toast {
            if now >= t.until {
                self.toast = None;
                self.refresh_mode();
            }
        }

        // reloj / progreso
        let st = unsafe { GetLocalTime() };
        if st.wMinute != self.last_minute {
            self.last_minute = st.wMinute;
            redraw = true;
        }
        if self.mode == Mode::Expanded && self.page == Page::Music {
            let m = self.media.lock().unwrap();
            if m.has && m.playing {
                redraw = true;
            }
        }
        if redraw && !self.animating {
            self.render();
        }
    }

    // ---- entrada

    fn grab_focus(&mut self) {
        unsafe {
            let fg = GetForegroundWindow();
            if fg != self.hwnd {
                self.prev_fg = fg.0 as isize;
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.hwnd);
            }
        }
    }

    fn restore_focus(&mut self) {
        if self.prev_fg != 0 && !self.pinned {
            unsafe {
                if GetForegroundWindow() == self.hwnd {
                    let _ = SetForegroundWindow(HWND(self.prev_fg as _));
                }
            }
            self.prev_fg = 0;
        }
    }

    fn set_hover(&mut self, h: bool) {
        if h == self.hover {
            return;
        }
        self.hover = h;
        if h {
            if !self.pinned && self.collapsed_at.elapsed() > Duration::from_secs(8) {
                let m = self.media.lock().unwrap().clone();
                self.page = self.home_page(&m);
            }
        } else {
            self.collapsed_at = Instant::now();
            self.hover_act = None;
            self.restore_focus();
        }
        self.ensure_page();
        self.refresh_mode();
        self.update_vis_timer();
    }

    fn on_move(&mut self, x: i32, y: i32) {
        if !self.tracking {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.tracking = true;
        }
        let (x0, y0, w, h) = self.pill_rect();
        let (fx, fy) = (x as f32, y as f32);
        let inside = fx >= x0 - 2.0 && fx <= x0 + w + 2.0 && fy >= y0 - 2.0 && fy <= y0 + h + 2.0;
        if inside != self.hover {
            self.set_hover(inside);
        } else if inside {
            let ha = self.hit(x, y).map(|wd| wd.act);
            if ha != self.hover_act {
                self.hover_act = ha;
                if !self.animating {
                    self.render();
                }
            }
        }
    }

    fn on_leave(&mut self) {
        self.tracking = false;
        self.set_hover(false);
    }

    fn hit(&self, x: i32, y: i32) -> Option<Widget> {
        if self.mode != Mode::Expanded {
            return None;
        }
        let m = self.media.lock().unwrap().clone();
        let (x, y) = (x as f32, y as f32);
        for wd in self.layout(self.mode, self.page, &m) {
            if x >= wd.x && x < wd.x + wd.w && y >= wd.y && y < wd.y + wd.h {
                return Some(wd);
            }
        }
        None
    }

    fn click(&mut self, x: i32, y: i32) {
        if let Some(wd) = self.hit(x, y) {
            let act = match wd.act {
                Act::Seek(_) => {
                    let s = self.s;
                    let (tx, tw) = (wd.x + 8.0 * s, wd.w - 16.0 * s);
                    Act::Seek(((x as f32 - tx) / tw).clamp(0.0, 1.0))
                }
                a => a,
            };
            self.do_act(act);
        }
    }

    fn do_act(&mut self, act: Act) {
        let now = Instant::now();
        match act {
            Act::Tab(p) => self.page = p,
            Act::Pin => {
                self.pinned = !self.pinned;
                if !self.pinned {
                    self.restore_focus();
                }
            }
            Act::Prev => media_cmd(Cmd::Prev),
            Act::Toggle => media_cmd(Cmd::Toggle),
            Act::Next => media_cmd(Cmd::Next),
            Act::Seek(f) => {
                let dur = self.media.lock().unwrap().dur;
                if dur > 0.0 {
                    media_cmd(Cmd::Seek(f as f64 * dur));
                }
            }
            Act::Preset(mins) => {
                self.timer_set = mins * 60;
                self.timer_paused = None;
                self.timer_end = Some(now + Duration::from_secs(mins * 60));
            }
            Act::TimerAdd => {
                if let Some(e) = self.timer_end {
                    self.timer_end = Some(e + Duration::from_secs(60));
                } else if let Some(p) = self.timer_paused {
                    self.timer_paused = Some(p + 60.0);
                } else {
                    self.timer_set = (self.timer_set + 60).min(99 * 60);
                }
            }
            Act::TimerToggle => {
                if let Some(e) = self.timer_end {
                    self.timer_paused = Some(e.saturating_duration_since(now).as_secs_f32());
                    self.timer_end = None;
                } else if let Some(p) = self.timer_paused {
                    self.timer_end = Some(now + Duration::from_secs_f32(p.max(0.0)));
                    self.timer_paused = None;
                } else {
                    let secs = if self.timer_set == 0 { 60 } else { self.timer_set };
                    self.timer_set = secs;
                    self.timer_end = Some(now + Duration::from_secs(secs));
                }
            }
            Act::TimerReset => {
                self.timer_end = None;
                self.timer_paused = None;
            }
            Act::SwToggle => {
                if let Some(st) = self.sw_start {
                    self.sw_acc += st.elapsed().as_secs_f64();
                    self.sw_start = None;
                } else {
                    self.sw_start = Some(now);
                }
            }
            Act::SwLap => {
                let t = self.sw_elapsed();
                if t > 0.0 {
                    self.laps.push(t);
                    if self.laps.len() > 99 {
                        self.laps.remove(0);
                    }
                }
            }
            Act::SwReset => {
                self.sw_start = None;
                self.sw_acc = 0.0;
                self.laps.clear();
            }
            Act::Key(i) => {
                self.grab_focus();
                self.calc_press(KEYS[i].0);
            }
            Act::Feature(i) => {
                self.set_feature(i, !self.features[i]);
                return;
            }
            Act::Autostart => {
                set_autostart(!self.autostart);
                self.autostart = autostart_enabled();
            }
        }
        self.changed();
    }

    fn set_feature(&mut self, i: usize, on: bool) {
        self.features[i] = on;
        match i {
            F_MUSIC => {
                MUSIC_ON.store(on, Ordering::Relaxed);
                if !on {
                    *self.media.lock().unwrap() = Media::default();
                }
            }
            F_COVER => COVER_ON.store(on, Ordering::Relaxed),
            F_TIMER => {
                if !on {
                    self.timer_end = None;
                    self.timer_paused = None;
                }
            }
            F_STOPWATCH => {
                if !on {
                    self.sw_start = None;
                    self.sw_acc = 0.0;
                    self.laps.clear();
                }
            }
            F_CALC => {
                if !on {
                    self.calc_expr.clear();
                    self.calc_prev.clear();
                    self.calc_done = false;
                    self.calc_err = false;
                }
            }
            F_FULLSCREEN => {
                if !on && self.hidden {
                    self.hidden = false;
                    unsafe {
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
            _ => {}
        }
        save_features(&self.features);
        self.changed();
    }

    fn menu_cmd(&mut self, id: i32) {
        match id {
            120 => {
                set_autostart(!self.autostart);
                self.autostart = autostart_enabled();
                if !self.animating {
                    self.render();
                }
            }
            140 => {
                self.page = Page::Settings;
                self.pinned = true;
                self.changed();
            }
            200..=299 => {
                let i = (id - 200) as usize;
                if i < F_COUNT {
                    self.set_feature(i, !self.features[i]);
                }
            }
            _ => {}
        }
    }

    // ---- calculadora

    fn calc_clear(&mut self) {
        self.calc_expr.clear();
        self.calc_prev.clear();
        self.calc_done = false;
        self.calc_err = false;
    }

    fn calc_press(&mut self, k: &str) {
        match k {
            "AC" => self.calc_clear(),
            "←" => {
                if self.calc_done || self.calc_err {
                    self.calc_clear();
                } else {
                    self.calc_expr.pop();
                }
            }
            "=" => {
                if !self.calc_expr.is_empty() && !self.calc_done {
                    match eval_loose(&self.calc_expr) {
                        Some(v) => {
                            self.calc_prev = format!("{} =", disp_expr(&self.calc_expr));
                            self.calc_expr = fmt_num(v);
                            self.calc_done = true;
                            self.calc_err = false;
                        }
                        None => {
                            self.calc_err = true;
                            self.calc_done = false;
                        }
                    }
                }
            }
            _ => {
                if let Some(c) = key_char(k) {
                    if self.calc_err {
                        self.calc_clear();
                    }
                    if self.calc_done {
                        if c.is_ascii_digit() || c == '.' || c == '(' {
                            self.calc_expr.clear();
                            self.calc_prev.clear();
                        }
                        self.calc_done = false;
                    }
                    if self.calc_expr.len() < 40 {
                        self.calc_expr.push(c);
                    }
                }
            }
        }
    }

    fn on_char(&mut self, c: char) {
        if self.mode != Mode::Expanded || self.page != Page::Calc {
            return;
        }
        let k = match c {
            '0' => "0",
            '1' => "1",
            '2' => "2",
            '3' => "3",
            '4' => "4",
            '5' => "5",
            '6' => "6",
            '7' => "7",
            '8' => "8",
            '9' => "9",
            '.' | ',' => ".",
            '+' => "+",
            '-' => "−",
            '*' | 'x' | 'X' => "×",
            '/' => "÷",
            '(' => "(",
            ')' => ")",
            '=' | '\r' => "=",
            '\u{8}' => "←",
            '\u{1b}' | 'c' | 'C' => "AC",
            _ => return,
        };
        self.calc_press(k);
        if !self.animating {
            self.render();
        }
    }

    // ---- geometria de los controles (la usan el dibujo y el mouse)

    fn layout(&self, mode: Mode, page: Page, m: &Media) -> Vec<Widget> {
        let mut v: Vec<Widget> = Vec::new();
        if mode != Mode::Expanded {
            return v;
        }
        let s = self.s;
        let cx = self.pw as f32 / 2.0;
        let y0 = TOP * s;
        let mk = |x: f32, y: f32, w: f32, h: f32, kind: WKind, act: Act, label: &'static str, on: bool| Widget {
            x,
            y,
            w,
            h,
            kind,
            act,
            label,
            on,
        };

        // barra de pestañas
        let all: [(Page, &'static str, f32); 5] = [
            (Page::Music, "Música", 62.0),
            (Page::Clock, "Reloj", 55.0),
            (Page::Timer, "Timer", 55.0),
            (Page::Stopwatch, "Crono", 55.0),
            (Page::Calc, "Calc", 48.0),
        ];
        let mut items: Vec<(Page, &'static str, f32)> = Vec::new();
        for t in all.iter() {
            if self.page_enabled(t.0) {
                items.push(*t);
            }
        }
        items.push((Page::Settings, "Ajustes", 69.0));
        let gap = 4.0;
        let pin_w = 28.0;
        let total: f32 = items.iter().map(|t| t.2).sum::<f32>() + gap * items.len() as f32 + pin_w;
        let mut x = cx - total * s / 2.0;
        for (p, l, wd) in items {
            v.push(mk(x, y0 + 8.0 * s, wd * s, 24.0 * s, WKind::Tab, Act::Tab(p), l, p == page));
            x += (wd + gap) * s;
        }
        v.push(mk(x, y0 + 8.0 * s, pin_w * s, 24.0 * s, WKind::Pin, Act::Pin, "", self.pinned));

        match page {
            Page::Music => {
                if m.has {
                    let yc = y0 + 134.0 * s;
                    v.push(mk(cx - 84.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Icon(Icon::Prev), Act::Prev, "", false));
                    let big = if m.playing { Icon::Pause } else { Icon::Play };
                    v.push(mk(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::Toggle, "", false));
                    v.push(mk(cx + 40.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Icon(Icon::Next), Act::Next, "", false));
                    v.push(mk(cx - 156.0 * s, y0 + 98.0 * s, 312.0 * s, 20.0 * s, WKind::Bar, Act::Seek(0.0), "", false));
                }
            }
            Page::Timer => {
                let presets: [(&'static str, u64); 4] = [("1 min", 1), ("5 min", 5), ("10 min", 10), ("25 min", 25)];
                for (i, (l, mins)) in presets.iter().enumerate() {
                    let px = cx - 128.0 * s + i as f32 * 66.0 * s;
                    v.push(mk(px, y0 + 90.0 * s, 58.0 * s, 22.0 * s, WKind::Pill, Act::Preset(*mins), *l, false));
                }
                let yc = y0 + 132.0 * s;
                v.push(mk(cx - 136.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::TimerReset, "Reiniciar", false));
                let big = if self.timer_end.is_some() { Icon::Pause } else { Icon::Play };
                v.push(mk(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::TimerToggle, "", false));
                v.push(mk(cx + 64.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::TimerAdd, "+1 min", false));
            }
            Page::Stopwatch => {
                let yc = y0 + 124.0 * s;
                v.push(mk(cx - 136.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::SwLap, "Vuelta", false));
                let big = if self.sw_start.is_some() { Icon::Pause } else { Icon::Play };
                v.push(mk(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::SwToggle, "", false));
                v.push(mk(cx + 64.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::SwReset, "Reiniciar", false));
            }
            Page::Calc => {
                for (i, (lab, cls)) in KEYS.iter().enumerate() {
                    let (r, c) = ((i / 4) as f32, (i % 4) as f32);
                    v.push(mk(
                        cx - 168.0 * s + c * 86.0 * s,
                        y0 + 96.0 * s + r * 40.0 * s,
                        78.0 * s,
                        32.0 * s,
                        WKind::Key(*cls),
                        Act::Key(i),
                        *lab,
                        false,
                    ));
                }
            }
            Page::Settings => {
                for i in 0..=F_COUNT {
                    let (label, act, on): (&'static str, Act, bool) = if i < F_COUNT {
                        (FEATURES[i].1, Act::Feature(i), self.features[i])
                    } else {
                        ("Iniciar con Windows", Act::Autostart, self.autostart)
                    };
                    let (col, row) = ((i % 2) as f32, (i / 2) as f32);
                    v.push(mk(
                        cx - 194.0 * s + col * 200.0 * s,
                        y0 + 44.0 * s + row * 28.0 * s,
                        188.0 * s,
                        26.0 * s,
                        WKind::Toggle,
                        act,
                        label,
                        on,
                    ));
                }
            }
            Page::Clock => {}
        }
        v
    }
}

// ------------------------------------------------------------ app: dibujo

impl App {
    #[allow(clippy::too_many_arguments)]
    unsafe fn txt(&self, font: HFONT, s: &str, l: f32, t: f32, r: f32, b: f32, col: (u8, u8, u8), f: f32, fmt: DRAW_TEXT_FORMAT) {
        if s.is_empty() || f < 0.02 || r <= l {
            return;
        }
        let c = |v: u8| (v as f32 * f) as u32;
        SetTextColor(self.mem, COLORREF((c(col.2) << 16) | (c(col.1) << 8) | c(col.0)));
        let mut rc = RECT { left: l.round() as i32, top: t.round() as i32, right: r.round() as i32, bottom: b.round() as i32 };
        let mut v: Vec<u16> = s.encode_utf16().collect();
        let old = SelectObject(self.mem, HGDIOBJ(font.0));
        DrawTextW(self.mem, &mut v, &mut rc, fmt);
        SelectObject(self.mem, old);
    }

    fn draw_widgets(&self, cv: &mut Canvas, wl: &[Widget], f: f32) {
        let s = self.s;
        for wd in wl {
            let hov = self.hover_act == Some(wd.act);
            let (cx, cy) = (wd.x + wd.w / 2.0, wd.y + wd.h / 2.0);
            match wd.kind {
                WKind::Tab => {
                    if wd.on {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, wd.h / 2.0, WHITE, 0.16 * f);
                    } else if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, wd.h / 2.0, WHITE, 0.07 * f);
                    }
                }
                WKind::Pin => {
                    if hov {
                        cv.circle(cx, cy, 11.0 * s, WHITE, 0.08 * f);
                    }
                    if wd.on {
                        cv.circle(cx, cy, 5.0 * s, ORANGE, f);
                    } else {
                        cv.circle(cx, cy, 5.0 * s, (150.0, 150.0, 158.0), f);
                        cv.circle(cx, cy, 3.2 * s, BLACK, f);
                    }
                }
                WKind::Pill => {
                    let c = if hov { (86.0, 86.0, 94.0) } else { (62.0, 62.0, 70.0) };
                    cv.rrect(wd.x, wd.y, wd.w, wd.h, wd.h / 2.0, c, f);
                }
                WKind::Key(cls) => {
                    let base = match cls {
                        0 => (58.0, 58.0, 64.0),
                        1 => (160.0, 160.0, 168.0),
                        _ => ORANGE,
                    };
                    let c = if hov {
                        ((base.0 + 28.0).min(255.0), (base.1 + 28.0).min(255.0), (base.2 + 28.0).min(255.0))
                    } else {
                        base
                    };
                    cv.rrect(wd.x, wd.y, wd.w, wd.h, 12.0 * s, c, f);
                }
                WKind::Toggle => {
                    if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, 8.0 * s, WHITE, 0.06 * f);
                    }
                    cv.switch(wd.x + wd.w - 40.0 * s, cy - 10.0 * s, s, wd.on, f);
                }
                WKind::Icon(ic) => {
                    if hov {
                        cv.circle(cx, cy, 19.0 * s, WHITE, 0.14 * f);
                    }
                    cv.icon(ic, cx, cy, s, WHITE, f);
                }
                WKind::Big(ic) => {
                    let bg = if hov { (225.0, 225.0, 228.0) } else { WHITE };
                    cv.circle(cx, cy, 17.0 * s, bg, f);
                    cv.icon(ic, cx, cy, s, BLACK, f);
                }
                WKind::Bar => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_software(&self, cv: &mut Canvas, m: &Media, mode: Mode, page: Page, wl: &[Widget], x0: f32, y0: f32, w: f32, h: f32, f: f32) {
        let s = self.s;
        let t = self.t0.elapsed().as_secs_f32();
        let cxm = self.pw as f32 / 2.0;
        let accent = m.cover.as_ref().map(|c| c.accent).unwrap_or(WHITE);
        let vis = self.feat(F_VIS);
        let cover_or_placeholder = |cv: &mut Canvas, x: f32, y: f32, size: f32, r: f32| match &m.cover {
            Some(c) => cv.cover(c, x, y, size, r, f),
            None => cv.rrect(x, y, size, size, r, (52.0, 52.0, 58.0), f),
        };

        match mode {
            Mode::Collapsed => match self.collapsed_view(m) {
                0 => cv.clock_icon(x0 + 21.0 * s, y0 + h / 2.0, 8.0 * s, s, ORANGE, f),
                1 => cv.clock_icon(x0 + 21.0 * s, y0 + h / 2.0, 8.0 * s, s, GREEN, f),
                2 => {
                    let size = 22.0 * s;
                    cover_or_placeholder(cv, x0 + 8.0 * s, y0 + (h - size) / 2.0, size, 6.0 * s);
                    if vis {
                        cv.bars(x0 + w - 14.0 * s - 19.5 * s, y0 + h / 2.0, s, t, m.playing, accent, f);
                    }
                }
                _ => {}
            },
            Mode::Expanded => {
                self.draw_widgets(cv, wl, f);
                match page {
                    Page::Music => {
                        if m.has {
                            cover_or_placeholder(cv, cxm - 194.0 * s, y0 + 42.0 * s, 56.0 * s, 11.0 * s);
                            if vis {
                                cv.bars(cxm + 174.5 * s, y0 + 56.0 * s, s, t, m.playing, accent, f);
                            }
                            let (bx0, bx1) = (cxm - 148.0 * s, cxm + 148.0 * s);
                            let yc = y0 + 108.0 * s;
                            cv.rrect(bx0, yc - 2.0 * s, bx1 - bx0, 4.0 * s, 2.0 * s, (70.0, 70.0, 78.0), f);
                            if m.dur > 0.0 {
                                let frac = (m.cur_pos() / m.dur).clamp(0.0, 1.0) as f32;
                                let len = (bx1 - bx0) * frac;
                                cv.rrect(bx0, yc - 2.0 * s, len.max(0.5), 4.0 * s, 2.0 * s, accent, f);
                                cv.circle(bx0 + len, yc, 4.5 * s, WHITE, f);
                            }
                        }
                    }
                    Page::Clock => {
                        if self.batt.has {
                            cv.battery(cxm - 34.0 * s, y0 + 106.0 * s, s, self.batt, f);
                        }
                    }
                    _ => {}
                }
            }
            Mode::Toast => {
                if let Some(tt) = &self.toast {
                    if tt.battery {
                        cv.battery(x0 + 18.0 * s, y0 + h / 2.0 - 6.0 * s, s, self.batt, f);
                    } else {
                        cv.clock_icon(x0 + 28.0 * s, y0 + h / 2.0, 10.0 * s, s, ORANGE, f);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_text(&self, m: &Media, mode: Mode, page: Page, wl: &[Widget], x0: f32, y0: f32, w: f32, h: f32, f: f32) {
        let dc = self.mem;
        let s = self.s;
        let cxm = self.pw as f32 / 2.0;
        SetBkMode(dc, TRANSPARENT);
        IntersectClipRect(dc, x0 as i32, y0 as i32, (x0 + w).ceil() as i32, (y0 + h).ceil() as i32);
        let base = DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
        let left = base | DT_LEFT | DT_VCENTER;
        let right = base | DT_RIGHT | DT_VCENTER;
        let center = base | DT_CENTER | DT_VCENTER;
        let white = (255u8, 255u8, 255u8);
        let gray = (165u8, 165u8, 172u8);
        let orange = (255u8, 159u8, 10u8);
        let st = GetLocalTime();
        let clock = format!("{:02}:{:02}", st.wHour, st.wMinute);

        match mode {
            Mode::Collapsed => match self.collapsed_view(m) {
                0 => {
                    let col = if self.timer_end.is_some() { orange } else { (200, 200, 205) };
                    self.txt(self.font_main, &fmt_timer(self.timer_secs()), x0 + 36.0 * s, y0, x0 + w - 12.0 * s, y0 + h, col, f, center);
                }
                1 => {
                    self.txt(self.font_main, &fmt_sw(self.sw_elapsed()), x0 + 36.0 * s, y0, x0 + w - 10.0 * s, y0 + h, (48, 209, 88), f, center);
                }
                2 => {
                    self.txt(self.font_small, &m.title, x0 + 40.0 * s, y0, x0 + w - 40.0 * s, y0 + h, white, f, center);
                }
                3 => {
                    self.txt(self.font_main, &clock, x0 + 10.0 * s, y0, x0 + w - 10.0 * s, y0 + h, white, f, center);
                }
                _ => {}
            },
            Mode::Expanded => {
                match page {
                    Page::Music => {
                        if m.has {
                            self.txt(self.font_main, &m.title, cxm - 126.0 * s, y0 + 44.0 * s, cxm + 160.0 * s, y0 + 66.0 * s, white, f, left);
                            self.txt(self.font_small, &m.artist, cxm - 126.0 * s, y0 + 67.0 * s, cxm + 160.0 * s, y0 + 85.0 * s, gray, f, left);
                            if m.dur > 0.0 {
                                self.txt(self.font_small, &fmt_time(m.cur_pos()), cxm - 208.0 * s, y0 + 98.0 * s, cxm - 154.0 * s, y0 + 118.0 * s, gray, f, right);
                                self.txt(self.font_small, &fmt_time(m.dur), cxm + 154.0 * s, y0 + 98.0 * s, cxm + 208.0 * s, y0 + 118.0 * s, gray, f, left);
                            }
                        } else {
                            self.txt(self.font_small, "Nada reproduciéndose", cxm - 200.0 * s, y0 + 40.0 * s, cxm + 200.0 * s, y0 + 92.0 * s, gray, f, center);
                        }
                    }
                    Page::Clock => {
                        const D: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
                        const M: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];
                        let date = format!("{} {} {}", D[(st.wDayOfWeek as usize) % 7], st.wDay, M[((st.wMonth as usize).max(1) - 1) % 12]);
                        self.txt(self.font_big, &clock, cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 82.0 * s, white, f, center);
                        self.txt(self.font_small, &date, cxm - 150.0 * s, y0 + 82.0 * s, cxm + 150.0 * s, y0 + 102.0 * s, gray, f, center);
                        if self.batt.has {
                            self.txt(self.font_small, &format!("{}%", self.batt.pct), cxm + 4.0 * s, y0 + 103.0 * s, cxm + 70.0 * s, y0 + 121.0 * s, gray, f, left);
                        }
                    }
                    Page::Timer => {
                        let col = if self.timer_end.is_some() {
                            orange
                        } else if self.timer_paused.is_some() {
                            (200, 200, 205)
                        } else {
                            white
                        };
                        self.txt(self.font_big, &fmt_timer(self.timer_secs()), cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 86.0 * s, col, f, center);
                    }
                    Page::Stopwatch => {
                        self.txt(self.font_big, &fmt_sw(self.sw_elapsed()), cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 84.0 * s, white, f, center);
                        let n = self.laps.len();
                        if n > 0 {
                            let prev = if n >= 2 { self.laps[n - 2] } else { 0.0 };
                            let line = format!("Vuelta {} · {}", n, fmt_sw(self.laps[n - 1] - prev));
                            self.txt(self.font_small, &line, cxm - 150.0 * s, y0 + 84.0 * s, cxm + 150.0 * s, y0 + 100.0 * s, gray, f, center);
                        }
                    }
                    Page::Calc => {
                        let line = if self.calc_done {
                            self.calc_prev.clone()
                        } else if self.calc_err {
                            String::new()
                        } else {
                            let has_op = self.calc_expr.chars().skip(1).any(|c| "+-*/".contains(c)) || self.calc_expr.contains('(');
                            if has_op {
                                eval_loose(&self.calc_expr).map(|v| format!("= {}", fmt_num(v))).unwrap_or_default()
                            } else {
                                String::new()
                            }
                        };
                        self.txt(self.font_small, &tail_chars(&line, 30), cxm - 168.0 * s, y0 + 38.0 * s, cxm + 168.0 * s, y0 + 54.0 * s, gray, f, right);
                        let big = if self.calc_err {
                            "Error".to_string()
                        } else if self.calc_expr.is_empty() {
                            "0".to_string()
                        } else {
                            tail_chars(&disp_expr(&self.calc_expr), 16)
                        };
                        self.txt(self.font_big, &big, cxm - 168.0 * s, y0 + 54.0 * s, cxm + 168.0 * s, y0 + 90.0 * s, white, f, right);
                    }
                    Page::Settings => {}
                }
                // etiquetas de los controles
                for wd in wl {
                    match wd.kind {
                        WKind::Tab => {
                            let col = if wd.on { white } else { (150, 150, 158) };
                            self.txt(self.font_small, wd.label, wd.x, wd.y, wd.x + wd.w, wd.y + wd.h, col, f, center);
                        }
                        WKind::Pill => {
                            self.txt(self.font_small, wd.label, wd.x, wd.y, wd.x + wd.w, wd.y + wd.h, white, f, center);
                        }
                        WKind::Key(cls) => {
                            let col = if cls == 1 { (0, 0, 0) } else { white };
                            self.txt(self.font_main, wd.label, wd.x, wd.y, wd.x + wd.w, wd.y + wd.h, col, f, center);
                        }
                        WKind::Toggle => {
                            self.txt(self.font_small, wd.label, wd.x + 4.0 * s, wd.y, wd.x + wd.w - 46.0 * s, wd.y + wd.h, white, f, left);
                        }
                        _ => {}
                    }
                }
            }
            Mode::Toast => {
                if let Some(t) = &self.toast {
                    self.txt(self.font_main, &t.text, x0 + 58.0 * s, y0, x0 + w - 16.0 * s, y0 + h, white, f, left);
                }
            }
        }
        SelectClipRgn(dc, HRGN::default());
    }

    fn render(&mut self) {
        if self.hidden {
            return;
        }
        let m = self.media.lock().unwrap().clone();
        let s = self.s;
        let (pw, ph) = (self.pw as usize, self.ph as usize);
        let buf: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(self.bits, pw * ph) };
        buf.fill(0);
        self.cov.fill(0);
        self.sh.fill(0);

        let (x0, y0, w, h) = self.pill_rect();
        let r = (h / 2.0).min(26.0 * s);
        let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
        let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);

        let xa = (x0.floor() as i32 - 1).max(0) as usize;
        let xb = ((x0 + w).ceil() as i32 + 1).clamp(0, pw as i32) as usize;
        let ya = (y0.floor() as i32 - 1).max(0) as usize;
        let yb = ((y0 + h).ceil() as i32 + 1).clamp(0, ph as i32) as usize;

        // sombra (se salta el interior de la pildora)
        let (bl, off) = (20.0 * s, 5.0 * s);
        let sxa = (x0 - bl - 2.0).floor().max(0.0) as usize;
        let sxb = ((x0 + w + bl + 2.0).ceil() as usize).min(pw);
        let sya = (y0 - bl).floor().max(0.0) as usize;
        let syb = ((y0 + h + bl + off + 2.0).ceil() as usize).min(ph);
        if self.feat(F_SHADOW) {
            let il = (x0 + 1.5 * s).ceil().max(0.0) as usize;
            let ir = (x0 + w - 1.5 * s).floor().max(0.0) as usize;
            for y in sya..syb {
                let yf = y as f32 + 0.5;
                let (a0, a1) = if yf >= y0 + r && yf <= y0 + h - r { (il, ir) } else { (0, 0) };
                let mut x = sxa;
                while x < sxb {
                    if x >= a0 && x < a1 {
                        x = a1;
                        continue;
                    }
                    let d = sdf_rr(x as f32 + 0.5, yf, cx, cy + off, hx, hy, r);
                    if d < bl {
                        let t = 1.0 - d.max(0.0) / bl;
                        self.sh[y * pw + x] = (0.5 * t * t * 255.0) as u8;
                    }
                    x += 1;
                }
            }
        }

        // pildora + borde interior sutil (el interior se rellena de golpe)
        let ix0 = ((x0 + 2.0 * s).ceil() as usize).min(xb);
        let ix1 = ((x0 + w - 2.0 * s).floor().max(0.0) as usize).min(xb);
        let iy0 = ((y0 + r).ceil() as usize).min(yb);
        let iy1 = ((y0 + h - r).floor().max(0.0) as usize).min(yb);
        for y in ya..yb {
            if y >= iy0 && y < iy1 && ix0 < ix1 {
                for x in xa..ix0 {
                    pill_pixel(x, y, pw, cx, cy, hx, hy, r, s, &mut self.cov, buf);
                }
                let row = y * pw;
                self.cov[row + ix0..row + ix1].fill(255);
                for x in ix1..xb {
                    pill_pixel(x, y, pw, cx, cy, hx, hy, r, s, &mut self.cov, buf);
                }
            } else {
                for x in xa..xb {
                    pill_pixel(x, y, pw, cx, cy, hx, hy, r, s, &mut self.cov, buf);
                }
            }
        }

        // contenido; el anterior se desvanece rapido mientras entra el nuevo
        let f = smooth(self.fade);
        let old = match self.prev_view {
            Some((pm, pp)) => {
                let fo = smooth(1.0 - self.fade * 2.5);
                if fo > 0.0 {
                    Some((pm, pp, fo))
                } else {
                    None
                }
            }
            None => None,
        };
        if old.is_none() {
            self.prev_view = None;
        }
        let wl_new = self.layout(self.mode, self.page, &m);
        let wl_old = match old {
            Some((pm, pp, _)) => self.layout(pm, pp, &m),
            None => Vec::new(),
        };
        {
            let mut cv = Canvas { b: &mut *buf, pw, ph };
            if let Some((pm, pp, fo)) = old {
                self.draw_software(&mut cv, &m, pm, pp, &wl_old, x0, y0, w, h, fo);
            }
            self.draw_software(&mut cv, &m, self.mode, self.page, &wl_new, x0, y0, w, h, f);
        }
        unsafe {
            if let Some((pm, pp, fo)) = old {
                self.draw_text(&m, pm, pp, &wl_old, x0, y0, w, h, fo);
            }
            self.draw_text(&m, self.mode, self.page, &wl_new, x0, y0, w, h, f);
            let _ = GdiFlush();
        }

        // alpha premultiplicado: pildora sobre sombra
        for y in sya..syb {
            for x in sxa..sxb {
                let i = y * pw + x;
                let c = self.cov[i] as u32;
                let sh = self.sh[i] as u32;
                let a = c + sh * (255 - c) / 255;
                if a == 0 {
                    buf[i] = 0;
                    continue;
                }
                let p = buf[i];
                let (r_, g_, b_) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
                buf[i] = (a << 24) | ((r_ * c / 255) << 16) | ((g_ * c / 255) << 8) | (b_ * c / 255);
            }
        }

        unsafe {
            let src = POINT { x: 0, y: 0 };
            let size = SIZE { cx: self.pw, cy: self.ph };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                HDC::default(),
                Some(&self.pos as *const POINT),
                Some(&size as *const SIZE),
                self.mem,
                Some(&src as *const POINT),
                COLORREF(0),
                Some(&blend as *const BLENDFUNCTION),
                ULW_ALPHA,
            );
        }
    }
}

// ------------------------------------------------------------ bandeja (tray)

fn make_icon() -> HICON {
    unsafe {
        let n = 32i32;
        let mut px = vec![0u32; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                let d = sdf_rr(x as f32 + 0.5, y as f32 + 0.5, 16.0, 16.0, 7.0, 0.0, 7.0);
                let a = (0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 {
                    let c = if d > -2.0 { 225.0 } else { 0.0 };
                    let al = (a * 255.0) as u32;
                    let v = (c * a) as u32;
                    px[(y * n + x) as usize] = (al << 24) | (v << 16) | (v << 8) | v;
                }
            }
        }
        let mask_bits = vec![0u8; (n * n / 8) as usize];
        let color = CreateBitmap(n, n, 1, 32, Some(px.as_ptr() as *const c_void));
        let mask = CreateBitmap(n, n, 1, 1, Some(mask_bits.as_ptr() as *const c_void));
        let ii = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    for (i, c) in "Dynamic Island".encode_utf16().enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

fn tray_add(hwnd: HWND) {
    unsafe {
        let mut nid = tray_data(hwnd);
        nid.hIcon = make_icon();
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn tray_remove(hwnd: HWND) {
    unsafe {
        let nid = tray_data(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn popup(hwnd: HWND) {
    let cmd = show_menu(hwnd);
    unsafe {
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    }
    if cmd == 130 {
        unsafe { PostQuitMessage(0) };
    } else if cmd != 0 {
        with_app(|a| a.menu_cmd(cmd));
    }
}

fn show_menu(hwnd: HWND) -> i32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let (feats, auto) = APP.with(|c| {
            c.try_borrow()
                .ok()
                .and_then(|b| b.as_ref().map(|a| (a.features, a.autostart)))
                .unwrap_or(([true; F_COUNT], false))
        });
        let _ = AppendMenuW(menu, MF_STRING, 140, w!("Abrir configuración"));
        if let Ok(sub) = CreatePopupMenu() {
            for (i, (_, label)) in FEATURES.iter().enumerate() {
                let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let fl = if feats[i] { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(sub, fl, 200 + i, PCWSTR(wide.as_ptr()));
            }
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Funciones"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let fl = if auto { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, fl, 120, w!("Iniciar con Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 130, w!("Salir"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, None);
        let _ = DestroyMenu(menu);
        cmd.0
    }
}

// ------------------------------------------------------------ ventana

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                let id = wp.0;
                with_app(|a| match id {
                    T_VIS => a.vis_tick(),
                    _ => a.clock_tick(),
                });
                LRESULT(0)
            }
            WM_APP_FRAME => {
                FRAME_PENDING.store(false, Ordering::Release);
                with_app(|a| {
                    if a.animating {
                        a.tick()
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                with_app(|a| a.on_move(x, y));
                LRESULT(0)
            }
            WM_MOUSELEAVE_ => {
                with_app(|a| a.on_leave());
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                with_app(|a| a.click(x, y));
                LRESULT(0)
            }
            WM_CHAR => {
                let ch = char::from_u32(wp.0 as u32).unwrap_or('\0');
                with_app(|a| a.on_char(ch));
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let mut ok = false;
                with_app(|a| ok = a.feat(F_VOLUME));
                if ok {
                    key_tap(0xAD, 1); // silenciar
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                let mut over = false;
                with_app(|a| over = a.hover && a.feat(F_VOLUME));
                if over && delta != 0 {
                    let n = (delta.abs() / 120).clamp(1, 5);
                    key_tap(if delta > 0 { 0xAF } else { 0xAE }, n);
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                popup(hwnd);
                LRESULT(0)
            }
            WM_TRAY => {
                let ev = (lp.0 & 0xFFFF) as u32;
                if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
                    popup(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_MEDIA => {
                with_app(|a| a.on_media());
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE_),
            WM_DESTROY => {
                tray_remove(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => {
                if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
                    tray_add(hwnd); // el Explorer se reinicio: volver a poner el icono
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

/// Marca el ritmo de los cuadros con el compositor (vsync) solo mientras hay animacion.
fn anim_thread(hwnd: isize) {
    loop {
        if !ANIM_ON.load(Ordering::Acquire) {
            std::thread::park();
            continue;
        }
        unsafe {
            if DwmFlush().is_err() {
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        if ANIM_ON.load(Ordering::Acquire) && !FRAME_PENDING.swap(true, Ordering::AcqRel) {
            unsafe {
                let _ = PostMessageW(HWND(hwnd as _), WM_APP_FRAME, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn main() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDPIAware();
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("DynamicIslandWnd"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let s = GetDpiForSystem() as f32 / 96.0;
        let (pw, ph) = ((CW * s) as i32, (CH * s) as i32);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let pos = POINT { x: (sw - pw) / 2, y: 0 };

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("DynamicIslandWnd"),
            w!("DynamicIsland"),
            WS_POPUP,
            pos.x,
            pos.y,
            pw,
            ph,
            None,
            None,
            instance,
            None,
        )?;

        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pw,
                biHeight: -ph,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0)?;
        SelectObject(mem, HGDIOBJ(bmp.0));
        ReleaseDC(None, screen);

        let features = load_features();
        MUSIC_ON.store(features[F_MUSIC], Ordering::Relaxed);
        COVER_ON.store(features[F_COVER], Ordering::Relaxed);

        let media = Arc::new(Mutex::new(Media::default()));
        {
            let m = media.clone();
            let hv = hwnd.0 as isize;
            std::thread::spawn(move || media_thread(hv, m));
        }
        {
            let hv = hwnd.0 as isize;
            let h = std::thread::spawn(move || anim_thread(hv));
            let _ = ANIM_THREAD.set(h.thread().clone());
        }

        let n = (pw * ph) as usize;
        let long_ago = Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(Instant::now);
        let app = App {
            hwnd,
            mem,
            bits: bits as *mut u32,
            pw,
            ph,
            pos,
            s,
            font_main: make_font((15.0 * s) as i32, 600),
            font_small: make_font((13.0 * s) as i32, 400),
            font_big: make_font((30.0 * s) as i32, 300),
            cur: [120.0, 34.0],
            vel: [0.0, 0.0],
            mode: Mode::Collapsed,
            page: Page::Clock,
            prev_view: None,
            layout_key: (Mode::Collapsed, Page::Clock, false, false),
            fade: 1.0,
            hover: false,
            pinned: false,
            hover_act: None,
            tracking: false,
            animating: false,
            vis_on: false,
            hidden: false,
            last: Instant::now(),
            t0: Instant::now(),
            collapsed_at: long_ago,
            media,
            cov: vec![0u8; n],
            sh: vec![0u8; n],
            last_minute: 99,
            toast: None,
            batt: read_batt(),
            prev_ac: None,
            low_warned: false,
            features,
            autostart: autostart_enabled(),
            timer_set: 5 * 60,
            timer_end: None,
            timer_paused: None,
            sw_start: None,
            sw_acc: 0.0,
            laps: Vec::new(),
            calc_expr: String::new(),
            calc_prev: String::new(),
            calc_done: false,
            calc_err: false,
            prev_fg: 0,
        };
        APP.with(|c| *c.borrow_mut() = Some(app));

        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        tray_add(hwnd);

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        with_app(|a| {
            a.ensure_page();
            a.render();
        });
        SetTimer(hwnd, T_CLOCK, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray_remove(hwnd);
    }
    Ok(())
}
