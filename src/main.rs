#![windows_subsystem = "windows"]

//! Dynamic Island para Windows (v0.5)
//! Paginas: Musica, Letras, Reloj, Calendario, Timer, Crono, Calculadora, Tareas, Ajustes.
//! Temas (Clasico / NieR), estilo isla o notch, mover libremente, opacidad por
//! proximidad, fuente elegible. Configuracion en %APPDATA%\DynamicIsland\config.ini.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Data::Json::{JsonArray, JsonObject};
use windows::Foundation::{TimeSpan, Uri};
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
use windows::Web::Http::HttpClient;
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
    keybd_event, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NOTIFYICONDATAW, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::*;

// ------------------------------------------------------------ constantes

const CW: f32 = 520.0; // lienzo (logico)
const CH: f32 = 390.0;
const TOP: f32 = 8.0; // separacion con el borde superior (estilo isla)
const EW: f32 = 440.0; // ancho de la isla expandida
const CS: usize = 96; // lado de la caratula decodificada

const WM_APP_MEDIA: u32 = 0x8000 + 1;
const WM_TRAY: u32 = 0x8000 + 2;
const WM_APP_FRAME: u32 = 0x8000 + 3;
const WM_MOUSELEAVE_: u32 = 0x02A3;
const MA_NOACTIVATE_: isize = 3;
const MA_ACTIVATE_: isize = 1;

const T_CLOCK: usize = 2;
const T_VIS: usize = 3;
const T_PROX: usize = 4;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ANIM_ON: AtomicBool = AtomicBool::new(false);
static FRAME_PENDING: AtomicBool = AtomicBool::new(false);
static ANIM_THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
static MUSIC_ON: AtomicBool = AtomicBool::new(true);
static COVER_ON: AtomicBool = AtomicBool::new(true);

type Rgb = (f32, f32, f32);
const fn c(r: u8, g: u8, b: u8) -> Rgb {
    (r as f32, g as f32, b as f32)
}
const WHITE: Rgb = c(255, 255, 255);
const ORANGE: Rgb = c(255, 159, 10);
const GREEN: Rgb = c(48, 209, 88);

// ------------------------------------------------------------ temas

#[derive(Clone, Copy)]
struct Theme {
    bg: Rgb,
    fg: Rgb,
    fg2: Rgb,
    accent: Rgb,
    on_accent: Rgb,
    good: Rgb,
    surface: Rgb,
    surface_hi: Rgb,
    track: Rgb,
    key_fn: Rgb,
    key_fn_fg: Rgb,
    knob: Rgb,
    big_bg: Rgb,
    big_fg: Rgb,
    sharp: bool,
    ring: f32,
    frame: bool,
    font: &'static str,
}

const THEME_NAMES: [&str; 3] = ["Clásico", "NieR claro", "NieR oscuro"];
const THEMES: [Theme; 3] = [
    Theme {
        bg: c(0, 0, 0),
        fg: WHITE,
        fg2: c(165, 165, 172),
        accent: ORANGE,
        on_accent: WHITE,
        good: GREEN,
        surface: c(62, 62, 70),
        surface_hi: c(86, 86, 94),
        track: c(70, 70, 78),
        key_fn: c(160, 160, 168),
        key_fn_fg: c(0, 0, 0),
        knob: WHITE,
        big_bg: WHITE,
        big_fg: c(0, 0, 0),
        sharp: false,
        ring: 0.14,
        frame: false,
        font: "Segoe UI",
    },
    Theme {
        bg: c(218, 212, 187),
        fg: c(74, 71, 62),
        fg2: c(125, 121, 104),
        accent: c(176, 72, 52),
        on_accent: c(244, 241, 226),
        good: c(104, 134, 86),
        surface: c(201, 195, 168),
        surface_hi: c(184, 178, 150),
        track: c(188, 182, 155),
        key_fn: c(188, 182, 155),
        key_fn_fg: c(74, 71, 62),
        knob: c(244, 241, 226),
        big_bg: c(74, 71, 62),
        big_fg: c(218, 212, 187),
        sharp: true,
        ring: 0.0,
        frame: true,
        font: "Bahnschrift",
    },
    Theme {
        bg: c(66, 62, 54),
        fg: c(218, 212, 187),
        fg2: c(160, 155, 135),
        accent: c(222, 96, 66),
        on_accent: c(30, 28, 24),
        good: c(140, 170, 110),
        surface: c(86, 81, 70),
        surface_hi: c(106, 100, 86),
        track: c(98, 93, 80),
        key_fn: c(120, 114, 98),
        key_fn_fg: c(34, 32, 26),
        knob: c(228, 223, 200),
        big_bg: c(218, 212, 187),
        big_fg: c(66, 62, 54),
        sharp: true,
        ring: 0.0,
        frame: true,
        font: "Bahnschrift",
    },
];

fn theme_of(i: u8) -> Theme {
    THEMES[(i as usize).min(THEMES.len() - 1)]
}

const FONTS: [&str; 12] = [
    "Automática",
    "Segoe UI",
    "Bahnschrift",
    "Segoe UI Variable",
    "Cascadia Code",
    "Consolas",
    "Georgia",
    "Cambria",
    "Trebuchet MS",
    "Calibri",
    "Verdana",
    "Courier New",
];

// ------------------------------------------------------------ funciones activables

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
const F_LYRICS: usize = 11;
const F_CALENDAR: usize = 12;
const F_TODO: usize = 13;
const F_NOTIF: usize = 14;
const F_PROX: usize = 15;
const F_DRAG: usize = 16;
const F_LYRMODE: usize = 17;
const F_COUNT: usize = 18;

// (clave en config.ini, etiqueta)
const FEATURES: [(&str, &str); F_COUNT] = [
    ("music", "Música"),
    ("cover", "Carátula"),
    ("visualizer", "Visualizador"),
    ("clock", "Reloj"),
    ("timer", "Temporizador"),
    ("stopwatch", "Cronómetro"),
    ("calculator", "Calculadora"),
    ("battery", "Aviso de batería"),
    ("fullscreen", "Ocultar en pantalla completa"),
    ("volume", "Volumen con la rueda"),
    ("shadow", "Sombra"),
    ("lyrics", "Letras"),
    ("calendar", "Calendario"),
    ("todo", "Tareas"),
    ("notif", "Botón de notificaciones"),
    ("proximity", "Opacidad por proximidad"),
    ("drag", "Mover libremente"),
    ("lyricmode", "Letra en la isla"),
];

// que funciones salen en la pestaña "Funciones" de Ajustes
const FUNC_LIST: [usize; 11] = [
    F_MUSIC, F_LYRICS, F_LYRMODE, F_CLOCK, F_CALENDAR, F_TIMER, F_STOPWATCH, F_CALC, F_TODO, F_NOTIF, F_BATT,
];

// teclas de la calculadora: (etiqueta, estilo 0=numero 1=funcion 2=operador)
const KEYS: [(&str, u8); 20] = [
    ("AC", 1), ("(", 1), (")", 1), ("÷", 2),
    ("7", 0), ("8", 0), ("9", 0), ("×", 2),
    ("4", 0), ("5", 0), ("6", 0), ("−", 2),
    ("1", 0), ("2", 0), ("3", 0), ("+", 2),
    ("←", 1), ("0", 0), (".", 0), ("=", 2),
];

#[derive(Clone, Copy)]
struct Cfg {
    theme: u8,
    style: u8, // 0 isla, 1 notch
    font: u8,
    op_min: f32,
    prox_dist: f32,
    dx: i32,
    dy: i32,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { theme: 0, style: 0, font: 0, op_min: 0.35, prox_dist: 300.0, dx: 0, dy: 0 }
    }
}

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
    Lyrics,
    Clock,
    Calendar,
    Timer,
    Stopwatch,
    Calc,
    Todo,
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
    None,
    Tab(Page),
    Pin,
    Notif,
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
    Cycle(u8, i8),
    Slider(u8),
    SubTab(u8),
    CalPrev,
    CalNext,
    CalToday,
    TodoToggle(usize),
    TodoDel(usize),
    TodoInput,
    TodoAdd,
    ResetPos,
}

#[derive(Clone, Copy)]
enum WKind {
    Tab(Page),
    Pin,
    Bell,
    Pill,
    Key(u8),
    Toggle,
    Icon(Icon),
    Big(Icon),
    Bar,
    Chip,
    Selector,
    Slider,
    Arrow(i8),
    Check,
    Del,
    Input,
    Label,
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
    label2: &'static str,
    on: bool,
    val: f32,
}

impl Widget {
    fn new(x: f32, y: f32, w: f32, h: f32, kind: WKind, act: Act, label: &'static str) -> Widget {
        Widget { x, y, w, h, kind, act, label, label2: "", on: false, val: 0.0 }
    }
    fn on(mut self, b: bool) -> Widget {
        self.on = b;
        self
    }
    fn val(mut self, v: f32) -> Widget {
        self.val = v;
        self
    }
    fn l2(mut self, s: &'static str) -> Widget {
        self.label2 = s;
        self
    }
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

#[derive(Default)]
struct Lyrics {
    key: (String, String),
    lines: Vec<(f64, String)>,
    plain: Vec<String>,
    state: u8, // 0 nada, 1 buscando, 2 listas, 3 no encontradas
}

struct DragState {
    c0: POINT,
    p0: POINT,
    moved: bool,
}

struct App {
    hwnd: HWND,
    mem: HDC,
    bits: *mut u32,
    pw: i32,
    ph: i32,
    pos: POINT,
    home: POINT,
    s: f32,
    font_main: HFONT,
    font_small: HFONT,
    font_big: HFONT,
    font_strike: HFONT,
    cfg: Cfg,
    th: Theme,
    cur: [f32; 2],
    vel: [f32; 2],
    mode: Mode,
    page: Page,
    prev_view: Option<(Mode, Page)>,
    layout_key: (Mode, Page, bool, u8),
    lyr_idx: isize,
    fade: f32,
    opacity: f32,
    opacity_target: f32,
    hover: bool,
    pinned: bool,
    editing: bool,
    hover_act: Option<Act>,
    tracking: bool,
    animating: bool,
    vis_on: bool,
    prox_on: bool,
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
    settings_tab: u8,
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
    cal_year: i32,
    cal_month: u32,
    todos: Vec<(bool, String)>,
    todo_input: String,
    todo_scroll: usize,
    lyrics: Arc<Mutex<Lyrics>>,
    lyr_key: (String, String),
    slider_drag: Option<u8>,
    drag: Option<DragState>,
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

fn data_dir() -> Option<std::path::PathBuf> {
    let d = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(d).join("DynamicIsland"))
}

fn load_config() -> ([bool; F_COUNT], Cfg) {
    let mut f = [true; F_COUNT];
    f[F_PROX] = false; // la opacidad por proximidad es opt-in
    f[F_LYRMODE] = false; // la letra en la isla es opt-in
    let mut cfg = Cfg::default();
    if let Some(d) = data_dir() {
        if let Ok(txt) = std::fs::read_to_string(d.join("config.ini")) {
            for line in txt.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    for (i, (key, _)) in FEATURES.iter().enumerate() {
                        if *key == k {
                            f[i] = v != "0";
                        }
                    }
                    match k {
                        "theme" => cfg.theme = v.parse::<u8>().unwrap_or(0).min(THEMES.len() as u8 - 1),
                        "style" => cfg.style = v.parse::<u8>().unwrap_or(0).min(1),
                        "font" => cfg.font = v.parse::<u8>().unwrap_or(0).min(FONTS.len() as u8 - 1),
                        "opacity_min" => cfg.op_min = v.parse::<f32>().unwrap_or(0.35).clamp(0.1, 0.9),
                        "prox_dist" => cfg.prox_dist = v.parse::<f32>().unwrap_or(300.0).clamp(100.0, 600.0),
                        "pos_dx" => cfg.dx = v.parse::<i32>().unwrap_or(0),
                        "pos_dy" => cfg.dy = v.parse::<i32>().unwrap_or(0),
                        _ => {}
                    }
                }
            }
        }
    }
    (f, cfg)
}

fn save_config(f: &[bool; F_COUNT], cfg: &Cfg) {
    if let Some(d) = data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let mut s = String::new();
        for (i, (k, _)) in FEATURES.iter().enumerate() {
            s.push_str(&format!("{}={}\n", k, if f[i] { 1 } else { 0 }));
        }
        s.push_str(&format!("theme={}\nstyle={}\nfont={}\n", cfg.theme, cfg.style, cfg.font));
        s.push_str(&format!("opacity_min={:.2}\nprox_dist={:.0}\n", cfg.op_min, cfg.prox_dist));
        s.push_str(&format!("pos_dx={}\npos_dy={}\n", cfg.dx, cfg.dy));
        let _ = std::fs::write(d.join("config.ini"), s);
    }
}

fn load_todos() -> Vec<(bool, String)> {
    let mut v = Vec::new();
    if let Some(d) = data_dir() {
        if let Ok(txt) = std::fs::read_to_string(d.join("todo.txt")) {
            for line in txt.lines() {
                if let Some((flag, text)) = line.split_once('|') {
                    if !text.is_empty() {
                        v.push((flag == "1", text.to_string()));
                    }
                }
            }
        }
    }
    v
}

fn save_todos(t: &[(bool, String)]) {
    if let Some(d) = data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let mut s = String::new();
        for (done, text) in t {
            s.push_str(&format!("{}|{}\n", if *done { 1 } else { 0 }, text));
        }
        let _ = std::fs::write(d.join("todo.txt"), s);
    }
}

// ------------------------------------------------------------ calendario

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// 0 = lunes ... 6 = domingo
fn weekday_mon0(y: i32, m: u32, d: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let dow = (yy + yy / 4 - yy / 100 + yy / 400 + t[(m - 1) as usize] + d as i32).rem_euclid(7);
    ((dow + 6) % 7) as u32
}

const MONTHS_ES: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre",
    "octubre", "noviembre", "diciembre",
];

// ------------------------------------------------------------ letras (lrclib.net)

fn url_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

fn parse_lrc(s: &str) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim();
        let mut times: Vec<f64> = Vec::new();
        while rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                let tag = &rest[1..end];
                if let Some((m, sec)) = tag.split_once(':') {
                    if let (Ok(m), Ok(sec)) = (m.trim().parse::<f64>(), sec.trim().parse::<f64>()) {
                        times.push(m * 60.0 + sec);
                    }
                }
                rest = rest[end + 1..].trim_start();
            } else {
                break;
            }
        }
        for t in times {
            out.push((t, rest.trim().to_string()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn lyric_line(lines: &[(f64, String)], i: isize) -> &str {
    if i < 0 || i as usize >= lines.len() {
        ""
    } else {
        let t = lines[i as usize].1.as_str();
        if t.is_empty() {
            "…"
        } else {
            t
        }
    }
}

fn http_get(url: &str) -> windows::core::Result<String> {
    let client = HttpClient::new()?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let body = client.GetStringAsync(&uri)?.get()?;
    Ok(body.to_string())
}

fn lyrics_from_obj(o: &JsonObject) -> (Vec<(f64, String)>, Vec<String>) {
    let get = |k: &str| -> String {
        o.GetNamedString(&HSTRING::from(k)).map(|h| h.to_string()).unwrap_or_default()
    };
    let synced = parse_lrc(&get("syncedLyrics"));
    let plain: Vec<String> = get("plainLyrics").lines().map(|l| l.trim().to_string()).collect();
    (synced, plain)
}

fn fetch_lyrics(artist: &str, title: &str, dur: f64) -> Option<(Vec<(f64, String)>, Vec<String>)> {
    let q = format!("artist_name={}&track_name={}", url_encode(artist), url_encode(title));
    let url = format!("https://lrclib.net/api/get?{}&duration={}", q, dur.round() as u32);
    if let Ok(body) = http_get(&url) {
        if let Ok(o) = JsonObject::Parse(&HSTRING::from(body.as_str())) {
            let (synced, plain) = lyrics_from_obj(&o);
            if !synced.is_empty() || !plain.iter().all(|l| l.is_empty()) {
                return Some((synced, plain));
            }
        }
    }
    let url = format!("https://lrclib.net/api/search?{}", q);
    let body = http_get(&url).ok()?;
    let arr = JsonArray::Parse(&HSTRING::from(body.as_str())).ok()?;
    let n = arr.Size().ok()?;
    let mut fallback: Option<(Vec<(f64, String)>, Vec<String>)> = None;
    for i in 0..n.min(8) {
        if let Ok(o) = arr.GetObjectAt(i) {
            let (synced, plain) = lyrics_from_obj(&o);
            if !synced.is_empty() {
                return Some((synced, plain));
            }
            if fallback.is_none() && !plain.iter().all(|l| l.is_empty()) {
                fallback = Some((synced, plain));
            }
        }
    }
    fallback
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
    th: Theme,
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
        let body = self.th.fg2;
        self.rrect(x, y, 24.0 * s, 12.0 * s, 3.5 * s, body, f);
        self.rrect(x + 1.6 * s, y + 1.6 * s, 20.8 * s, 8.8 * s, 2.2 * s, self.th.bg, f);
        let col = if b.ac {
            self.th.good
        } else if b.pct <= 20 {
            (255.0, 69.0, 58.0)
        } else {
            self.th.fg
        };
        let len = (18.0 * b.pct as f32 / 100.0).max(1.0);
        self.rrect(x + 3.0 * s, y + 3.0 * s, len * s, 6.0 * s, 1.2 * s, col, f);
        self.rrect(x + 24.8 * s, y + 3.8 * s, 2.2 * s, 4.4 * s, 1.1 * s, body, f);
    }

    fn clock_icon(&mut self, cx: f32, cy: f32, r: f32, s: f32, c: Rgb, f: f32) {
        self.circle(cx, cy, r, c, f);
        self.circle(cx, cy, r - 1.7 * s, self.th.bg, f);
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
        let col = if on { self.th.good } else { self.th.track };
        self.rrect(x, y, w, h, h / 2.0, col, f);
        let kx = if on { x + w - h / 2.0 } else { x + h / 2.0 };
        self.circle(kx, y + h / 2.0, 8.0 * s, self.th.knob, f);
    }
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



// ------------------------------------------------------------ dibujo: extras

fn pack(c: Rgb) -> u32 {
    ((c.0 as u32) << 16) | ((c.1 as u32) << 8) | (c.2 as u32)
}

/// Forma de la isla: rectangulo redondeado; en modo notch, extendido por encima
/// de la pantalla y con "orejas" concavas en las esquinas superiores.
#[derive(Clone, Copy)]
struct Shape {
    cx: f32,
    cy: f32,
    hx: f32,
    hy: f32,
    r: f32,
    notch: bool,
    ex0: f32,
    ex1: f32,
    e: f32,
}

impl Shape {
    fn sdf(&self, px: f32, py: f32) -> f32 {
        let mut d = sdf_rr(px, py, self.cx, self.cy, self.hx, self.hy, self.r);
        if self.notch && py < self.e + 1.0 {
            for side in 0..2 {
                let (bx, ccx) = if side == 0 {
                    (self.ex0 - self.e / 2.0, self.ex0 - self.e)
                } else {
                    (self.ex1 + self.e / 2.0, self.ex1 + self.e)
                };
                let qx = (px - bx).abs() - self.e / 2.0;
                let qy = (py - self.e / 2.0).abs() - self.e / 2.0;
                let dbox = qx.max(qy);
                let dc = ((px - ccx) * (px - ccx) + (py - self.e) * (py - self.e)).sqrt() - self.e;
                d = d.min(dbox.max(-dc));
            }
        }
        d
    }
}

/// Un pixel del borde de la pildora: cobertura + relleno del tema + brillo del borde.
#[inline]
#[allow(clippy::too_many_arguments)]
fn pill_pixel(x: usize, y: usize, pw: usize, sh: &Shape, s: f32, bgp: u32, ring: f32, cov: &mut [u8], buf: &mut [u32]) {
    let d = sh.sdf(x as f32 + 0.5, y as f32 + 0.5);
    let a = (0.5 - d).clamp(0.0, 1.0);
    if a > 0.0 {
        let i = y * pw + x;
        cov[i] = (a * 255.0) as u8;
        let mut p = bgp;
        if ring > 0.0 {
            let depth = (-d).max(0.0);
            let rg = (1.0 - depth / (1.2 * s)).max(0.0) * ring;
            if rg > 0.0 {
                blend(&mut p, WHITE, rg);
            }
        }
        buf[i] = p;
    }
}

fn make_font(face: &str, height: i32, weight: i32, strike: bool) -> HFONT {
    let mut name = [0u16; 32];
    for (i, ch) in face.encode_utf16().take(31).enumerate() {
        name[i] = ch;
    }
    let lf = LOGFONTW {
        lfHeight: -height,
        lfWeight: weight,
        lfStrikeOut: if strike { 1 } else { 0 },
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: ANTIALIASED_QUALITY,
        lfFaceName: name,
        ..Default::default()
    };
    unsafe { CreateFontIndirectW(&lf) }
}

impl<'a> Canvas<'a> {
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, t: f32, col: Rgb, f: f32) {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let l = dx.hypot(dy).max(1e-4);
        let (nx, ny) = (-dy / l * t / 2.0, dx / l * t / 2.0);
        self.poly(&[(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)], col, f);
    }

    fn arrow(&mut self, dir: i8, cx: f32, cy: f32, s: f32, col: Rgb, f: f32) {
        if dir < 0 {
            self.poly(&[(cx + 3.0 * s, cy - 5.0 * s), (cx + 3.0 * s, cy + 5.0 * s), (cx - 3.5 * s, cy)], col, f);
        } else {
            self.poly(&[(cx - 3.0 * s, cy - 5.0 * s), (cx - 3.0 * s, cy + 5.0 * s), (cx + 3.5 * s, cy)], col, f);
        }
    }

    fn bell(&mut self, cx: f32, cy: f32, s: f32, col: Rgb, f: f32) {
        self.circle(cx, cy - 2.0 * s, 5.0 * s, col, f);
        self.poly(
            &[(cx - 5.0 * s, cy - 2.0 * s), (cx + 5.0 * s, cy - 2.0 * s), (cx + 7.0 * s, cy + 4.0 * s), (cx - 7.0 * s, cy + 4.0 * s)],
            col,
            f,
        );
        self.circle(cx, cy + 6.0 * s, 1.8 * s, col, f);
    }

    /// Marco fino estilo NieR con marcas en las esquinas.
    fn frame(&mut self, x0: f32, y0: f32, w: f32, h: f32, s: f32) {
        let th = self.th;
        let (fx, fy, fw, fh) = (x0 + 6.0 * s, y0 + 6.0 * s, w - 12.0 * s, h - 12.0 * s);
        if fw < 80.0 * s || fh < 40.0 * s {
            return;
        }
        self.rrect(fx, fy, fw, fh, 1.0 * s, th.fg, 0.4);
        self.rrect(fx + 1.0 * s, fy + 1.0 * s, fw - 2.0 * s, fh - 2.0 * s, 0.5 * s, th.bg, 1.0);
        let q = 3.0 * s;
        for (px, py) in [(fx, fy), (fx + fw - q, fy), (fx, fy + fh - q), (fx + fw - q, fy + fh - q)] {
            self.rrect(px, py, q, q, 0.5 * s, th.fg, 0.75);
        }
    }

    fn tab_icon(&mut self, page: Page, cx: f32, cy: f32, s: f32, col: Rgb, f: f32) {
        let bg = self.th.bg;
        match page {
            Page::Music => {
                self.circle(cx - 3.0 * s, cy + 4.5 * s, 3.2 * s, col, f);
                self.rrect(cx - 0.2 * s, cy - 7.0 * s, 1.7 * s, 12.0 * s, 0.8 * s, col, f);
                self.poly(&[(cx + 1.4 * s, cy - 7.0 * s), (cx + 6.0 * s, cy - 4.0 * s), (cx + 1.4 * s, cy - 2.0 * s)], col, f);
            }
            Page::Lyrics => {
                self.rrect(cx - 7.0 * s, cy - 5.5 * s, 14.0 * s, 2.0 * s, 1.0 * s, col, f);
                self.rrect(cx - 7.0 * s, cy - 1.0 * s, 10.0 * s, 2.0 * s, 1.0 * s, col, f);
                self.rrect(cx - 7.0 * s, cy + 3.5 * s, 12.0 * s, 2.0 * s, 1.0 * s, col, f);
            }
            Page::Clock => self.clock_icon(cx, cy, 7.0 * s, s, col, f),
            Page::Calendar => {
                self.rrect(cx - 7.0 * s, cy - 6.0 * s, 14.0 * s, 13.0 * s, 2.5 * s, col, f);
                self.rrect(cx - 5.5 * s, cy - 2.5 * s, 11.0 * s, 8.5 * s, 1.2 * s, bg, f);
                self.rrect(cx - 3.8 * s, cy - 8.0 * s, 1.7 * s, 4.0 * s, 0.8 * s, col, f);
                self.rrect(cx + 2.1 * s, cy - 8.0 * s, 1.7 * s, 4.0 * s, 0.8 * s, col, f);
                self.circle(cx - 2.2 * s, cy + 1.8 * s, 1.1 * s, col, f);
                self.circle(cx + 2.2 * s, cy + 1.8 * s, 1.1 * s, col, f);
            }
            Page::Timer => {
                self.poly(&[(cx - 5.0 * s, cy - 6.0 * s), (cx + 5.0 * s, cy - 6.0 * s), (cx, cy)], col, f);
                self.poly(&[(cx - 5.0 * s, cy + 6.0 * s), (cx + 5.0 * s, cy + 6.0 * s), (cx, cy)], col, f);
                self.rrect(cx - 6.5 * s, cy - 8.0 * s, 13.0 * s, 1.8 * s, 0.9 * s, col, f);
                self.rrect(cx - 6.5 * s, cy + 6.2 * s, 13.0 * s, 1.8 * s, 0.9 * s, col, f);
            }
            Page::Stopwatch => {
                self.circle(cx, cy + 1.0 * s, 6.5 * s, col, f);
                self.circle(cx, cy + 1.0 * s, 4.8 * s, bg, f);
                self.rrect(cx - 1.5 * s, cy - 8.5 * s, 3.0 * s, 2.2 * s, 0.8 * s, col, f);
                self.rrect(cx - 0.7 * s, cy - 3.0 * s, 1.4 * s, 4.5 * s, 0.7 * s, col, f);
            }
            Page::Calc => {
                self.rrect(cx - 6.0 * s, cy - 7.0 * s, 12.0 * s, 14.0 * s, 2.5 * s, col, f);
                self.rrect(cx - 4.0 * s, cy - 5.0 * s, 8.0 * s, 3.0 * s, 1.0 * s, bg, f);
                for (dx, dy) in [(-2.5f32, 0.5f32), (2.5, 0.5), (-2.5, 4.0), (2.5, 4.0)] {
                    self.circle(cx + dx * s, cy + dy * s, 1.1 * s, bg, f);
                }
            }
            Page::Todo => {
                self.rrect(cx - 6.5 * s, cy - 6.5 * s, 13.0 * s, 13.0 * s, 3.0 * s, col, f);
                self.rrect(cx - 5.0 * s, cy - 5.0 * s, 10.0 * s, 10.0 * s, 2.0 * s, bg, f);
                self.line(cx - 3.2 * s, cy + 0.2 * s, cx - 1.0 * s, cy + 2.6 * s, 1.7 * s, col, f);
                self.line(cx - 1.0 * s, cy + 2.6 * s, cx + 3.6 * s, cy - 2.6 * s, 1.7 * s, col, f);
            }
            Page::Settings => {
                for (i, kx) in [-3.0f32, 3.5, -1.0].iter().enumerate() {
                    let y = cy + (i as f32 - 1.0) * 4.6 * s;
                    self.rrect(cx - 7.0 * s, y - 0.8 * s, 14.0 * s, 1.6 * s, 0.8 * s, col, f);
                    self.circle(cx + kx * s, y, 2.6 * s, col, f);
                    self.circle(cx + kx * s, y, 1.1 * s, bg, f);
                }
            }
        }
    }
}


// ------------------------------------------------------------ app: logica

fn open_notif() {
    // Win+N abre el centro de notificaciones (Windows 11)
    unsafe {
        keybd_event(0x5B, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(0x4E, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(0x4E, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(0x5B, 0, KEYEVENTF_KEYUP, 0);
    }
}

impl App {
    fn feat(&self, i: usize) -> bool {
        self.features[i]
    }

    fn notch(&self) -> bool {
        self.cfg.style == 1
    }

    fn top(&self) -> f32 {
        if self.notch() {
            0.0
        } else {
            TOP * self.s
        }
    }

    fn rad(&self, h: f32) -> f32 {
        if self.th.sharp {
            2.0 * self.s
        } else {
            h / 2.0
        }
    }

    fn page_enabled(&self, p: Page) -> bool {
        match p {
            Page::Music => self.feat(F_MUSIC),
            Page::Lyrics => self.feat(F_LYRICS) && self.feat(F_MUSIC),
            Page::Clock => self.feat(F_CLOCK),
            Page::Calendar => self.feat(F_CALENDAR),
            Page::Timer => self.feat(F_TIMER),
            Page::Stopwatch => self.feat(F_STOPWATCH),
            Page::Calc => self.feat(F_CALC),
            Page::Todo => self.feat(F_TODO),
            Page::Settings => true,
        }
    }

    fn home_page(&self, m: &Media) -> Page {
        if self.feat(F_MUSIC) && m.has {
            return Page::Music;
        }
        for p in [Page::Clock, Page::Music, Page::Calendar, Page::Timer, Page::Stopwatch, Page::Calc, Page::Todo] {
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

    /// Letra sincronizada lista para mostrarse en la isla colapsada.
    fn lyric_ready(&self, m: &Media) -> bool {
        if !self.feat(F_LYRMODE) || !m.has {
            return false;
        }
        let l = self.lyrics.lock().unwrap();
        l.state == 2 && !l.lines.is_empty() && l.key.0 == m.artist && l.key.1 == m.title
    }

    /// Indice de la linea de letra actual (-1 si aun no empieza).
    fn lyric_idx(&self, m: &Media) -> isize {
        let pos = m.cur_pos() + 0.25;
        let l = self.lyrics.lock().unwrap();
        l.lines.iter().rposition(|(t0, _)| *t0 <= pos).map(|i| i as isize).unwrap_or(-1)
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
        } else if self.lyric_ready(m) {
            5
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
        let mc = self.media.lock().unwrap().clone();
        let (has, playing) = (mc.has, mc.playing);
        let vis = has && playing && self.feat(F_VIS);
        let sw = self.sw_start.is_some();
        let lyr_mode = playing && self.collapsed_view(&mc) == 5;
        match self.mode {
            Mode::Collapsed => {
                if self.timer_active() {
                    false
                } else if sw || lyr_mode {
                    true
                } else {
                    vis
                }
            }
            Mode::Expanded => match self.page {
                Page::Music => vis,
                Page::Stopwatch => sw,
                Page::Lyrics => has && playing && self.lyrics.lock().unwrap().state == 2,
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
                        (EW, 158.0)
                    } else {
                        (EW, 100.0)
                    }
                }
                Page::Lyrics => (EW, 150.0),
                Page::Clock => (EW, 128.0),
                Page::Calendar => (EW, 244.0),
                Page::Timer => (EW, 156.0),
                Page::Stopwatch => (EW, 150.0),
                Page::Calc => (EW, 304.0),
                Page::Todo => (EW, 230.0),
                Page::Settings => match self.settings_tab {
                    0 => (EW, 272.0),
                    1 => (EW, 272.0),
                    _ => (EW, 332.0),
                },
            },
            Mode::Collapsed => match self.collapsed_view(m) {
                0 | 1 => (132.0, 34.0),
                2 => (226.0, 34.0),
                5 => (320.0, 34.0),
                3 => (120.0, 34.0),
                _ => (96.0, 30.0),
            },
        }
    }

    fn pill_rect(&self) -> (f32, f32, f32, f32) {
        let s = self.s;
        let top = self.top();
        let w = (self.cur[0] * s).clamp(1.0, self.pw as f32);
        let h = (self.cur[1] * s).clamp(1.0, self.ph as f32 - top);
        ((self.pw as f32 - w) / 2.0, top, w, h)
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
        } else if self.hover || self.pinned || self.editing {
            Mode::Expanded
        } else {
            Mode::Collapsed
        };
        let kp = if new == Mode::Expanded { self.page } else { Page::Clock };
        let mc = self.media.lock().unwrap().clone();
        let cv = self.collapsed_view(&mc);
        let key = (new, kp, mc.has, cv);
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
        if (new == Mode::Expanded && self.page == Page::Lyrics) || self.feat(F_LYRMODE) {
            self.check_lyrics();
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

    fn check_lyrics(&mut self) {
        if !self.feat(F_LYRICS) && !self.feat(F_LYRMODE) {
            return;
        }
        let m = self.media.lock().unwrap().clone();
        if !m.has || m.title.is_empty() {
            return;
        }
        let key = (m.artist.clone(), m.title.clone());
        if key == self.lyr_key {
            return;
        }
        self.lyr_key = key.clone();
        *self.lyrics.lock().unwrap() = Lyrics { key: key.clone(), state: 1, ..Default::default() };
        let ly = self.lyrics.clone();
        let hv = self.hwnd.0 as isize;
        let dur = m.dur;
        std::thread::spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            let res = fetch_lyrics(&key.0, &key.1, dur);
            {
                let mut l = ly.lock().unwrap();
                if l.key == key {
                    match res {
                        Some((lines, plain)) => {
                            l.lines = lines;
                            l.plain = plain;
                            l.state = 2;
                        }
                        None => l.state = 3,
                    }
                }
            }
            unsafe {
                let _ = PostMessageW(HWND(hv as _), WM_APP_MEDIA, WPARAM(0), LPARAM(0));
            }
        });
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
        // opacidad (suavizada)
        let kk = 1.0 - (-dt * 10.0).exp();
        self.opacity += (self.opacity_target - self.opacity) * kk;
        if (self.opacity_target - self.opacity).abs() > 0.004 {
            settled = false;
        } else {
            self.opacity = self.opacity_target;
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
        if self.live_visible() {
            // con la letra en la isla solo se redibuja al cambiar de linea
            if self.mode == Mode::Collapsed && self.sw_start.is_none() && !self.timer_active() {
                let m = self.media.lock().unwrap().clone();
                if self.collapsed_view(&m) == 5 {
                    let idx = self.lyric_idx(&m);
                    if idx != self.lyr_idx {
                        self.lyr_idx = idx;
                        if !self.animating {
                            self.render();
                        }
                    }
                    return;
                }
            }
            if !self.animating {
                self.render();
            }
        } else {
            self.update_vis_timer();
        }
    }

    /// Opacidad segun la distancia del cursor a la isla.
    fn prox_tick(&mut self) {
        if self.hidden {
            return;
        }
        let target = if !self.feat(F_PROX) {
            1.0
        } else {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
            }
            let (x0, y0, w, h) = self.pill_rect();
            let (l, t) = (self.pos.x as f32 + x0, self.pos.y as f32 + y0);
            let (px, py) = (p.x as f32, p.y as f32);
            let dx = (l - px).max(px - (l + w)).max(0.0);
            let dy = (t - py).max(py - (t + h)).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt();
            let near = self.hover || self.pinned || self.editing || self.toast.is_some() || self.drag.is_some();
            if near {
                1.0
            } else {
                let k = smooth(dist / self.cfg.prox_dist);
                1.0 + (self.cfg.op_min - 1.0) * k
            }
        };
        if (target - self.opacity_target).abs() > 0.01 {
            self.opacity_target = target;
            self.start_anim();
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

    // ---- edicion de texto (tareas / calculadora)

    fn begin_edit(&mut self) {
        if self.editing {
            return;
        }
        self.editing = true;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as isize));
            let fg = GetForegroundWindow();
            if fg != self.hwnd {
                self.prev_fg = fg.0 as isize;
            }
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
        }
        self.refresh_mode();
    }

    fn end_edit(&mut self, restore: bool) {
        if !self.editing {
            return;
        }
        self.editing = false;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            if restore && self.prev_fg != 0 && GetForegroundWindow() == self.hwnd {
                let _ = SetForegroundWindow(HWND(self.prev_fg as _));
            }
        }
        self.prev_fg = 0;
        self.refresh_mode();
        self.update_vis_timer();
        if !self.animating {
            self.render();
        }
    }

    // ---- posicion

    fn clamp_pos(&self, p: POINT) -> POINT {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let half = self.pw / 2;
            let m = (60.0 * self.s) as i32;
            POINT {
                x: p.x.clamp(vx - half + m, vx + vw - half - m),
                y: p.y.clamp(vy - (TOP * self.s) as i32, vy + vh - (80.0 * self.s) as i32),
            }
        }
    }

    fn apply_pos(&mut self) {
        let p = if self.notch() {
            self.home
        } else {
            self.clamp_pos(POINT { x: self.home.x + self.cfg.dx, y: self.home.y + self.cfg.dy })
        };
        self.pos = p;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND::default(), p.x, p.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn drag_move(&mut self) {
        let mut cur = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cur);
        }
        let (c0, p0, moved) = match &self.drag {
            Some(d) => (d.c0, d.p0, d.moved),
            None => return,
        };
        let (dx, dy) = (cur.x - c0.x, cur.y - c0.y);
        if !moved && dx.abs() + dy.abs() < 4 {
            return;
        }
        if let Some(d) = self.drag.as_mut() {
            d.moved = true;
        }
        let p = self.clamp_pos(POINT { x: p0.x + dx, y: p0.y + dy });
        self.pos = p;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND::default(), p.x, p.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn finish_drag(&mut self) {
        let s = self.s;
        let mut p = self.pos;
        if (p.x - self.home.x).abs() < (24.0 * s) as i32 {
            p.x = self.home.x;
        }
        if (p.y - self.home.y).abs() < (16.0 * s) as i32 {
            p.y = self.home.y;
        }
        self.pos = p;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND::default(), p.x, p.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.cfg.dx = p.x - self.home.x;
        self.cfg.dy = p.y - self.home.y;
        save_config(&self.features, &self.cfg);
    }

    // ---- entrada

    fn set_hover(&mut self, h: bool) {
        if h == self.hover {
            return;
        }
        self.hover = h;
        if h {
            if !self.pinned && !self.editing && self.collapsed_at.elapsed() > Duration::from_secs(8) {
                let m = self.media.lock().unwrap().clone();
                self.page = self.home_page(&m);
            }
        } else {
            self.collapsed_at = Instant::now();
            self.hover_act = None;
        }
        self.ensure_page();
        self.refresh_mode();
        self.update_vis_timer();
        self.prox_tick();
    }

    fn on_move(&mut self, x: i32, y: i32) {
        if let Some(id) = self.slider_drag {
            self.slider_set(id, x as f32);
            return;
        }
        if self.drag.is_some() {
            self.drag_move();
            return;
        }
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
        if self.slider_drag.is_none() && self.drag.is_none() {
            self.set_hover(false);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Widget> {
        if self.mode != Mode::Expanded {
            return None;
        }
        let m = self.media.lock().unwrap().clone();
        let (x, y) = (x as f32, y as f32);
        for wd in self.layout(self.mode, self.page, &m) {
            if wd.act == Act::None {
                continue;
            }
            if x >= wd.x && x < wd.x + wd.w && y >= wd.y && y < wd.y + wd.h {
                return Some(wd);
            }
        }
        None
    }

    fn on_down(&mut self, x: i32, y: i32) {
        if let Some(wd) = self.hit(x, y) {
            if let Act::Slider(id) = wd.act {
                self.slider_drag = Some(id);
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.slider_set(id, x as f32);
            }
            return;
        }
        if self.feat(F_DRAG) && !self.notch() {
            let (x0, y0, w, h) = self.pill_rect();
            let (fx, fy) = (x as f32, y as f32);
            if fx >= x0 && fx <= x0 + w && fy >= y0 && fy <= y0 + h {
                let mut p = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut p);
                    SetCapture(self.hwnd);
                }
                self.drag = Some(DragState { c0: p, p0: self.pos, moved: false });
            }
        }
    }

    fn on_up(&mut self, x: i32, y: i32) {
        if self.slider_drag.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
            save_config(&self.features, &self.cfg);
            return;
        }
        if let Some(d) = self.drag.take() {
            unsafe {
                let _ = ReleaseCapture();
            }
            if d.moved {
                self.finish_drag();
            }
            return;
        }
        self.click(x, y);
    }

    fn slider_set(&mut self, id: u8, x: f32) {
        let m = self.media.lock().unwrap().clone();
        let s = self.s;
        let found = self.layout(self.mode, self.page, &m).into_iter().find(|w| w.act == Act::Slider(id));
        if let Some(wd) = found {
            let (tx, tw) = (wd.x + 8.0 * s, wd.w - 16.0 * s);
            let frac = ((x - tx) / tw).clamp(0.0, 1.0);
            if id == 0 {
                self.cfg.op_min = 0.1 + 0.8 * frac;
            } else {
                self.cfg.prox_dist = 100.0 + 500.0 * frac;
            }
            self.prox_tick();
            if !self.animating {
                self.render();
            }
        }
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

    fn on_wheel(&mut self, delta: i32) {
        if self.mode == Mode::Expanded && self.page == Page::Todo {
            let max = self.todos.len().saturating_sub(5);
            if delta > 0 {
                self.todo_scroll = self.todo_scroll.saturating_sub(1);
            } else {
                self.todo_scroll = (self.todo_scroll + 1).min(max);
            }
            if !self.animating {
                self.render();
            }
            return;
        }
        if self.hover && self.feat(F_VOLUME) && delta != 0 {
            let n = (delta.abs() / 120).clamp(1, 5);
            key_tap(if delta > 0 { 0xAF } else { 0xAE }, n);
        }
    }

    fn rebuild_fonts(&mut self) {
        unsafe {
            for f in [self.font_main, self.font_small, self.font_big, self.font_strike] {
                let _ = DeleteObject(HGDIOBJ(f.0));
            }
        }
        let face: &str = if self.cfg.font == 0 { self.th.font } else { FONTS[self.cfg.font as usize] };
        let s = self.s;
        self.font_main = make_font(face, (15.0 * s) as i32, 600, false);
        self.font_small = make_font(face, (13.0 * s) as i32, 400, false);
        self.font_big = make_font(face, (30.0 * s) as i32, 300, false);
        self.font_strike = make_font(face, (13.0 * s) as i32, 400, true);
    }

    fn add_todo(&mut self) {
        let t = self.todo_input.trim().to_string();
        if !t.is_empty() {
            self.todos.insert(0, (false, t));
            self.todo_input.clear();
            self.todo_scroll = 0;
            save_todos(&self.todos);
        }
    }

    fn do_act(&mut self, act: Act) {
        let now = Instant::now();
        match act {
            Act::None | Act::Slider(_) => return,
            Act::Tab(p) => {
                if self.editing {
                    self.end_edit(true);
                }
                self.page = p;
            }
            Act::Pin => {
                self.pinned = !self.pinned;
                self.prox_tick();
            }
            Act::Notif => open_notif(),
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
                self.begin_edit();
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
            Act::Cycle(id, dir) => {
                let d = dir as i32;
                match id {
                    0 => {
                        let n = THEMES.len() as i32;
                        self.cfg.theme = ((self.cfg.theme as i32 + d).rem_euclid(n)) as u8;
                        self.th = theme_of(self.cfg.theme);
                        self.rebuild_fonts();
                    }
                    1 => {
                        self.cfg.style = ((self.cfg.style as i32 + d).rem_euclid(2)) as u8;
                        self.apply_pos();
                    }
                    _ => {
                        let n = FONTS.len() as i32;
                        self.cfg.font = ((self.cfg.font as i32 + d).rem_euclid(n)) as u8;
                        self.rebuild_fonts();
                    }
                }
                save_config(&self.features, &self.cfg);
            }
            Act::SubTab(i) => self.settings_tab = i,
            Act::CalPrev => {
                if self.cal_month == 1 {
                    self.cal_month = 12;
                    self.cal_year -= 1;
                } else {
                    self.cal_month -= 1;
                }
            }
            Act::CalNext => {
                if self.cal_month == 12 {
                    self.cal_month = 1;
                    self.cal_year += 1;
                } else {
                    self.cal_month += 1;
                }
            }
            Act::CalToday => {
                let st = unsafe { GetLocalTime() };
                self.cal_year = st.wYear as i32;
                self.cal_month = st.wMonth as u32;
            }
            Act::TodoToggle(i) => {
                if i < self.todos.len() {
                    self.todos[i].0 = !self.todos[i].0;
                    save_todos(&self.todos);
                }
            }
            Act::TodoDel(i) => {
                if i < self.todos.len() {
                    self.todos.remove(i);
                    let max = self.todos.len().saturating_sub(5);
                    self.todo_scroll = self.todo_scroll.min(max);
                    save_todos(&self.todos);
                }
            }
            Act::TodoInput => self.begin_edit(),
            Act::TodoAdd => {
                self.add_todo();
                self.begin_edit();
            }
            Act::ResetPos => {
                self.cfg.dx = 0;
                self.cfg.dy = 0;
                self.apply_pos();
                save_config(&self.features, &self.cfg);
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
                    self.calc_clear();
                }
            }
            F_TODO => {
                if !on {
                    self.end_edit(true);
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
            F_PROX => unsafe {
                if on && !self.prox_on {
                    SetTimer(self.hwnd, T_PROX, 150, None);
                    self.prox_on = true;
                    self.prox_tick();
                } else if !on {
                    if self.prox_on {
                        let _ = KillTimer(self.hwnd, T_PROX);
                        self.prox_on = false;
                    }
                    self.opacity_target = 1.0;
                    self.start_anim();
                }
            },
            _ => {}
        }
        save_config(&self.features, &self.cfg);
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
            150 => {
                self.cfg.dx = 0;
                self.cfg.dy = 0;
                self.apply_pos();
                save_config(&self.features, &self.cfg);
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
                if let Some(ch) = key_char(k) {
                    if self.calc_err {
                        self.calc_clear();
                    }
                    if self.calc_done {
                        if ch.is_ascii_digit() || ch == '.' || ch == '(' {
                            self.calc_expr.clear();
                            self.calc_prev.clear();
                        }
                        self.calc_done = false;
                    }
                    if self.calc_expr.len() < 40 {
                        self.calc_expr.push(ch);
                    }
                }
            }
        }
    }

    fn on_char(&mut self, ch: char) {
        if self.mode != Mode::Expanded {
            return;
        }
        match self.page {
            Page::Todo => {
                if !self.editing {
                    return;
                }
                match ch {
                    '\r' => self.add_todo(),
                    '\u{8}' => {
                        self.todo_input.pop();
                    }
                    '\u{1b}' => {
                        self.end_edit(true);
                        return;
                    }
                    c2 if !c2.is_control() => {
                        if self.todo_input.chars().count() < 80 {
                            self.todo_input.push(c2);
                        }
                    }
                    _ => {}
                }
            }
            Page::Calc => {
                let k = match ch {
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
            }
            _ => return,
        }
        if !self.animating {
            self.render();
        }
    }
}


// ------------------------------------------------------------ app: geometria y dibujo

impl App {
    /// Geometria de los controles; la usan el dibujo y el mouse.
    fn layout(&self, mode: Mode, page: Page, m: &Media) -> Vec<Widget> {
        let mut v: Vec<Widget> = Vec::new();
        if mode != Mode::Expanded {
            return v;
        }
        let s = self.s;
        let cx = self.pw as f32 / 2.0;
        let y0 = self.top();
        let half = EW / 2.0;

        // barra de pestañas (iconos; la activa muestra su nombre)
        const TABS: [(Page, &str, f32); 9] = [
            (Page::Music, "Música", 46.0),
            (Page::Lyrics, "Letras", 42.0),
            (Page::Clock, "Reloj", 36.0),
            (Page::Calendar, "Cal.", 30.0),
            (Page::Timer, "Timer", 38.0),
            (Page::Stopwatch, "Crono", 40.0),
            (Page::Calc, "Calc", 32.0),
            (Page::Todo, "Tareas", 44.0),
            (Page::Settings, "Ajustes", 48.0),
        ];
        let mut x = cx - (half - 16.0) * s;
        for (p, l, lw) in TABS.iter() {
            if !self.page_enabled(*p) {
                continue;
            }
            let sel = *p == page;
            let wd = if sel { 28.0 + lw + 8.0 } else { 28.0 };
            v.push(Widget::new(x, y0 + 8.0 * s, wd * s, 24.0 * s, WKind::Tab(*p), Act::Tab(*p), *l).on(sel));
            x += (wd + 3.0) * s;
        }
        v.push(Widget::new(cx + (half - 40.0) * s, y0 + 8.0 * s, 24.0 * s, 24.0 * s, WKind::Pin, Act::Pin, "").on(self.pinned));
        if self.feat(F_NOTIF) {
            v.push(Widget::new(cx + (half - 68.0) * s, y0 + 8.0 * s, 24.0 * s, 24.0 * s, WKind::Bell, Act::Notif, ""));
        }

        match page {
            Page::Music => {
                if m.has {
                    let yc = y0 + 134.0 * s;
                    v.push(Widget::new(cx - 84.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Icon(Icon::Prev), Act::Prev, ""));
                    let big = if m.playing { Icon::Pause } else { Icon::Play };
                    v.push(Widget::new(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::Toggle, ""));
                    v.push(Widget::new(cx + 40.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Icon(Icon::Next), Act::Next, ""));
                    v.push(Widget::new(cx - 156.0 * s, y0 + 98.0 * s, 312.0 * s, 20.0 * s, WKind::Bar, Act::Seek(0.0), ""));
                }
            }
            Page::Calendar => {
                v.push(Widget::new(cx - 150.0 * s, y0 + 40.0 * s, 28.0 * s, 24.0 * s, WKind::Arrow(-1), Act::CalPrev, ""));
                v.push(Widget::new(cx + 118.0 * s, y0 + 40.0 * s, 28.0 * s, 24.0 * s, WKind::Arrow(1), Act::CalNext, ""));
                v.push(Widget::new(cx + 158.0 * s, y0 + 41.0 * s, 48.0 * s, 22.0 * s, WKind::Pill, Act::CalToday, "Hoy"));
            }
            Page::Timer => {
                let presets: [(&'static str, u64); 4] = [("1 min", 1), ("5 min", 5), ("10 min", 10), ("25 min", 25)];
                for (i, (l, mins)) in presets.iter().enumerate() {
                    let px = cx - 128.0 * s + i as f32 * 66.0 * s;
                    v.push(Widget::new(px, y0 + 90.0 * s, 58.0 * s, 22.0 * s, WKind::Pill, Act::Preset(*mins), *l));
                }
                let yc = y0 + 132.0 * s;
                v.push(Widget::new(cx - 136.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::TimerReset, "Reiniciar"));
                let big = if self.timer_end.is_some() { Icon::Pause } else { Icon::Play };
                v.push(Widget::new(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::TimerToggle, ""));
                v.push(Widget::new(cx + 64.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::TimerAdd, "+1 min"));
            }
            Page::Stopwatch => {
                let yc = y0 + 124.0 * s;
                v.push(Widget::new(cx - 136.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::SwLap, "Vuelta"));
                let big = if self.sw_start.is_some() { Icon::Pause } else { Icon::Play };
                v.push(Widget::new(cx - 22.0 * s, yc - 22.0 * s, 44.0 * s, 44.0 * s, WKind::Big(big), Act::SwToggle, ""));
                v.push(Widget::new(cx + 64.0 * s, yc - 13.0 * s, 72.0 * s, 26.0 * s, WKind::Pill, Act::SwReset, "Reiniciar"));
            }
            Page::Calc => {
                for (i, (lab, cls)) in KEYS.iter().enumerate() {
                    let (r, cc) = ((i / 4) as f32, (i % 4) as f32);
                    v.push(Widget::new(
                        cx - 168.0 * s + cc * 86.0 * s,
                        y0 + 96.0 * s + r * 40.0 * s,
                        78.0 * s,
                        32.0 * s,
                        WKind::Key(*cls),
                        Act::Key(i),
                        *lab,
                    ));
                }
            }
            Page::Todo => {
                let ix = cx - 196.0 * s;
                v.push(Widget::new(ix, y0 + 40.0 * s, 354.0 * s, 26.0 * s, WKind::Input, Act::TodoInput, "").on(self.editing));
                v.push(Widget::new(cx + 164.0 * s, y0 + 40.0 * s, 32.0 * s, 26.0 * s, WKind::Pill, Act::TodoAdd, "+"));
                for k in 0..5usize {
                    let i = self.todo_scroll + k;
                    if i >= self.todos.len() {
                        break;
                    }
                    let y = y0 + 74.0 * s + k as f32 * 28.0 * s;
                    v.push(Widget::new(ix, y, 360.0 * s, 26.0 * s, WKind::Check, Act::TodoToggle(i), "").on(self.todos[i].0));
                    v.push(Widget::new(cx + 168.0 * s, y, 28.0 * s, 26.0 * s, WKind::Del, Act::TodoDel(i), ""));
                }
            }
            Page::Settings => {
                let chips: [(&'static str, f32); 3] = [("Funciones", 90.0), ("Apariencia", 90.0), ("Comportamiento", 118.0)];
                let total: f32 = chips.iter().map(|c| c.1).sum::<f32>() + 12.0;
                let mut x = cx - total * s / 2.0;
                for (i, (l, wd)) in chips.iter().enumerate() {
                    v.push(Widget::new(x, y0 + 40.0 * s, wd * s, 22.0 * s, WKind::Chip, Act::SubTab(i as u8), *l).on(self.settings_tab == i as u8));
                    x += (wd + 6.0) * s;
                }
                let ry = y0 + 78.0 * s;
                let (pitch, rh) = (30.0 * s, 26.0 * s);
                let toggle = |v: &mut Vec<Widget>, row: f32, fi: usize| {
                    v.push(Widget::new(cx - 200.0 * s, ry + row * pitch, 400.0 * s, rh, WKind::Toggle, Act::Feature(fi), FEATURES[fi].1).on(self.features[fi]));
                };
                let selector = |v: &mut Vec<Widget>, row: f32, id: u8, name: &'static str, val: &'static str| {
                    let y = ry + row * pitch;
                    v.push(Widget::new(cx - 200.0 * s, y, 180.0 * s, rh, WKind::Label, Act::None, name));
                    v.push(Widget::new(cx - 10.0 * s, y + 2.0 * s, 22.0 * s, rh - 4.0 * s, WKind::Arrow(-1), Act::Cycle(id, -1), ""));
                    v.push(Widget::new(cx + 14.0 * s, y, 158.0 * s, rh, WKind::Selector, Act::Cycle(id, 1), "").l2(val));
                    v.push(Widget::new(cx + 174.0 * s, y + 2.0 * s, 22.0 * s, rh - 4.0 * s, WKind::Arrow(1), Act::Cycle(id, 1), ""));
                };
                match self.settings_tab {
                    0 => {
                        for (k, &fi) in FUNC_LIST.iter().enumerate() {
                            let (col, row) = ((k % 2) as f32, (k / 2) as f32);
                            v.push(
                                Widget::new(cx - 208.0 * s + col * 212.0 * s, ry + row * pitch, 200.0 * s, rh, WKind::Toggle, Act::Feature(fi), FEATURES[fi].1)
                                    .on(self.features[fi]),
                            );
                        }
                    }
                    1 => {
                        const STYLES: [&str; 2] = ["Isla", "Notch"];
                        selector(&mut v, 0.0, 0, "Tema", THEME_NAMES[self.cfg.theme as usize]);
                        selector(&mut v, 1.0, 1, "Estilo", STYLES[self.cfg.style as usize]);
                        selector(&mut v, 2.0, 2, "Fuente", FONTS[self.cfg.font as usize]);
                        toggle(&mut v, 3.0, F_SHADOW);
                        toggle(&mut v, 4.0, F_VIS);
                        toggle(&mut v, 5.0, F_COVER);
                    }
                    _ => {
                        toggle(&mut v, 0.0, F_DRAG);
                        let y1 = ry + pitch;
                        v.push(Widget::new(cx - 200.0 * s, y1, 180.0 * s, rh, WKind::Label, Act::None, "Posición de la isla"));
                        v.push(Widget::new(cx + 86.0 * s, y1 + 1.0 * s, 110.0 * s, rh - 2.0 * s, WKind::Pill, Act::ResetPos, "Restablecer"));
                        toggle(&mut v, 2.0, F_PROX);
                        let y3 = ry + 3.0 * pitch;
                        v.push(Widget::new(cx - 200.0 * s, y3, 160.0 * s, rh, WKind::Label, Act::None, "Opacidad mínima"));
                        v.push(Widget::new(cx - 30.0 * s, y3, 172.0 * s, rh, WKind::Slider, Act::Slider(0), "").val((self.cfg.op_min - 0.1) / 0.8));
                        let y4 = ry + 4.0 * pitch;
                        v.push(Widget::new(cx - 200.0 * s, y4, 160.0 * s, rh, WKind::Label, Act::None, "Distancia"));
                        v.push(Widget::new(cx - 30.0 * s, y4, 172.0 * s, rh, WKind::Slider, Act::Slider(1), "").val((self.cfg.prox_dist - 100.0) / 500.0));
                        toggle(&mut v, 5.0, F_FULLSCREEN);
                        toggle(&mut v, 6.0, F_VOLUME);
                        let y7 = ry + 7.0 * pitch;
                        v.push(
                            Widget::new(cx - 200.0 * s, y7, 400.0 * s, rh, WKind::Toggle, Act::Autostart, "Iniciar con Windows").on(self.autostart),
                        );
                    }
                }
            }
            Page::Lyrics | Page::Clock => {}
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn txt(&self, font: HFONT, s: &str, l: f32, t: f32, r: f32, b: f32, col: Rgb, f: f32, fmt: DRAW_TEXT_FORMAT) {
        if s.is_empty() || f < 0.02 || r <= l {
            return;
        }
        let bg = self.th.bg;
        let mix = |c: f32, b0: f32| (b0 + (c - b0) * f).clamp(0.0, 255.0) as u32;
        SetTextColor(self.mem, COLORREF((mix(col.2, bg.2) << 16) | (mix(col.1, bg.1) << 8) | mix(col.0, bg.0)));
        let mut rc = RECT { left: l.round() as i32, top: t.round() as i32, right: r.round() as i32, bottom: b.round() as i32 };
        let mut v: Vec<u16> = s.encode_utf16().collect();
        let old = SelectObject(self.mem, HGDIOBJ(font.0));
        DrawTextW(self.mem, &mut v, &mut rc, fmt);
        SelectObject(self.mem, old);
    }

    fn draw_widgets(&self, cv: &mut Canvas, wl: &[Widget], f: f32) {
        let s = self.s;
        let th = self.th;
        for wd in wl {
            let hov = wd.act != Act::None && self.hover_act == Some(wd.act);
            let (cx, cy) = (wd.x + wd.w / 2.0, wd.y + wd.h / 2.0);
            match wd.kind {
                WKind::Tab(p) => {
                    if wd.on {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.fg, 0.16 * f);
                    } else if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.fg, 0.07 * f);
                    }
                    let col = if wd.on { th.fg } else { th.fg2 };
                    cv.tab_icon(p, wd.x + 14.0 * s, cy, s, col, f);
                }
                WKind::Pin => {
                    if hov {
                        cv.circle(cx, cy, 11.0 * s, th.fg, 0.08 * f);
                    }
                    if wd.on {
                        cv.circle(cx, cy, 5.0 * s, th.accent, f);
                    } else {
                        cv.circle(cx, cy, 5.0 * s, th.fg2, f);
                        cv.circle(cx, cy, 3.2 * s, th.bg, f);
                    }
                }
                WKind::Bell => {
                    if hov {
                        cv.circle(cx, cy, 11.0 * s, th.fg, 0.08 * f);
                    }
                    cv.bell(cx, cy, s, if hov { th.fg } else { th.fg2 }, f);
                }
                WKind::Pill => {
                    let c2 = if hov { th.surface_hi } else { th.surface };
                    cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), c2, f);
                }
                WKind::Key(cls) => {
                    let base = match cls {
                        0 => th.surface,
                        1 => th.key_fn,
                        _ => th.accent,
                    };
                    let c2 = if hov {
                        ((base.0 + 28.0).min(255.0), (base.1 + 28.0).min(255.0), (base.2 + 28.0).min(255.0))
                    } else {
                        base
                    };
                    let rr = if th.sharp { 2.0 * s } else { 12.0 * s };
                    cv.rrect(wd.x, wd.y, wd.w, wd.h, rr, c2, f);
                }
                WKind::Toggle => {
                    if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, 8.0 * s, th.fg, 0.06 * f);
                    }
                    cv.switch(wd.x + wd.w - 40.0 * s, cy - 10.0 * s, s, wd.on, f);
                }
                WKind::Icon(ic) => {
                    if hov {
                        cv.circle(cx, cy, 19.0 * s, th.fg, 0.14 * f);
                    }
                    cv.icon(ic, cx, cy, s, th.fg, f);
                }
                WKind::Big(ic) => {
                    let bg = if hov {
                        ((th.big_bg.0 * 0.9), (th.big_bg.1 * 0.9), (th.big_bg.2 * 0.9))
                    } else {
                        th.big_bg
                    };
                    cv.circle(cx, cy, 17.0 * s, bg, f);
                    cv.icon(ic, cx, cy, s, th.big_fg, f);
                }
                WKind::Chip => {
                    if wd.on {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.surface_hi, f);
                    } else if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.surface, f);
                    }
                }
                WKind::Selector => {
                    if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.surface, 0.7 * f);
                    }
                }
                WKind::Slider => {
                    let (tx, tw) = (wd.x + 8.0 * s, wd.w - 16.0 * s);
                    cv.rrect(tx, cy - 2.0 * s, tw, 4.0 * s, 2.0 * s, th.track, f);
                    let len = tw * wd.val.clamp(0.0, 1.0);
                    cv.rrect(tx, cy - 2.0 * s, len.max(0.5), 4.0 * s, 2.0 * s, th.accent, f);
                    cv.circle(tx + len, cy, 6.5 * s, th.knob, f);
                }
                WKind::Arrow(d) => {
                    cv.arrow(d, cx, cy, s, if hov { th.fg } else { th.fg2 }, f);
                }
                WKind::Check => {
                    if hov {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, self.rad(wd.h), th.surface, 0.5 * f);
                    }
                    let (bx, by, bs) = (wd.x + 4.0 * s, cy - 8.0 * s, 16.0 * s);
                    let rr = if th.sharp { 1.5 * s } else { 4.0 * s };
                    if wd.on {
                        cv.rrect(bx, by, bs, bs, rr, th.accent, f);
                        cv.line(bx + 3.5 * s, by + 8.5 * s, bx + 6.8 * s, by + 12.0 * s, 2.0 * s, th.on_accent, f);
                        cv.line(bx + 6.8 * s, by + 12.0 * s, bx + 12.8 * s, by + 4.5 * s, 2.0 * s, th.on_accent, f);
                    } else {
                        cv.rrect(bx, by, bs, bs, rr, th.fg2, f);
                        cv.rrect(bx + 1.5 * s, by + 1.5 * s, bs - 3.0 * s, bs - 3.0 * s, rr * 0.6, th.bg, f);
                    }
                }
                WKind::Del => {
                    let col = if hov { th.accent } else { th.fg2 };
                    cv.line(cx - 4.0 * s, cy - 4.0 * s, cx + 4.0 * s, cy + 4.0 * s, 1.6 * s, col, f);
                    cv.line(cx - 4.0 * s, cy + 4.0 * s, cx + 4.0 * s, cy - 4.0 * s, 1.6 * s, col, f);
                }
                WKind::Input => {
                    let rr = self.rad(wd.h);
                    if wd.on {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, rr, th.accent, f);
                        cv.rrect(wd.x + 1.5 * s, wd.y + 1.5 * s, wd.w - 3.0 * s, wd.h - 3.0 * s, rr * 0.8, th.surface, f);
                    } else {
                        cv.rrect(wd.x, wd.y, wd.w, wd.h, rr, th.surface, f);
                    }
                }
                WKind::Bar | WKind::Label => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_software(&self, cv: &mut Canvas, m: &Media, mode: Mode, page: Page, wl: &[Widget], x0: f32, y0: f32, w: f32, h: f32, f: f32) {
        let s = self.s;
        let th = self.th;
        let t = self.t0.elapsed().as_secs_f32();
        let cxm = self.pw as f32 / 2.0;
        let acc = m.cover.as_ref().map(|c| c.accent).unwrap_or(th.fg);
        let vis = self.feat(F_VIS);
        let cover_or_placeholder = |cv: &mut Canvas, x: f32, y: f32, size: f32, r: f32| match &m.cover {
            Some(c) => cv.cover(c, x, y, size, r, f),
            None => cv.rrect(x, y, size, size, r, th.surface, f),
        };

        match mode {
            Mode::Collapsed => match self.collapsed_view(m) {
                0 => cv.clock_icon(x0 + 21.0 * s, y0 + h / 2.0, 8.0 * s, s, th.accent, f),
                1 => cv.clock_icon(x0 + 21.0 * s, y0 + h / 2.0, 8.0 * s, s, th.good, f),
                2 => {
                    let size = 22.0 * s;
                    cover_or_placeholder(cv, x0 + 8.0 * s, y0 + (h - size) / 2.0, size, 6.0 * s);
                    if vis {
                        cv.bars(x0 + w - 14.0 * s - 19.5 * s, y0 + h / 2.0, s, t, m.playing, acc, f);
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
                                cv.bars(cxm + 174.5 * s, y0 + 56.0 * s, s, t, m.playing, acc, f);
                            }
                            let (bx0, bx1) = (cxm - 148.0 * s, cxm + 148.0 * s);
                            let yc = y0 + 108.0 * s;
                            cv.rrect(bx0, yc - 2.0 * s, bx1 - bx0, 4.0 * s, 2.0 * s, th.track, f);
                            if m.dur > 0.0 {
                                let frac = (m.cur_pos() / m.dur).clamp(0.0, 1.0) as f32;
                                let len = (bx1 - bx0) * frac;
                                cv.rrect(bx0, yc - 2.0 * s, len.max(0.5), 4.0 * s, 2.0 * s, acc, f);
                                cv.circle(bx0 + len, yc, 4.5 * s, th.knob, f);
                            }
                        }
                    }
                    Page::Clock => {
                        if self.batt.has {
                            cv.battery(cxm - 34.0 * s, y0 + 106.0 * s, s, self.batt, f);
                        }
                    }
                    Page::Calendar => {
                        let st = unsafe { GetLocalTime() };
                        if st.wYear as i32 == self.cal_year && st.wMonth as u32 == self.cal_month {
                            let first = weekday_mon0(self.cal_year, self.cal_month, 1);
                            let idx = first + st.wDay as u32 - 1;
                            let (row, col) = ((idx / 7) as f32, (idx % 7) as f32);
                            let (tx, ty) = (cxm - 140.0 * s + col * 40.0 * s + 20.0 * s, y0 + 88.0 * s + row * 24.0 * s + 12.0 * s);
                            if th.sharp {
                                cv.rrect(tx - 12.0 * s, ty - 10.0 * s, 24.0 * s, 20.0 * s, 2.0 * s, th.accent, f);
                            } else {
                                cv.circle(tx, ty, 11.0 * s, th.accent, f);
                            }
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
                        cv.clock_icon(x0 + 28.0 * s, y0 + h / 2.0, 10.0 * s, s, th.accent, f);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_text(&self, m: &Media, mode: Mode, page: Page, wl: &[Widget], x0: f32, y0: f32, w: f32, h: f32, f: f32) {
        let dc = self.mem;
        let s = self.s;
        let th = self.th;
        let cxm = self.pw as f32 / 2.0;
        SetBkMode(dc, TRANSPARENT);
        IntersectClipRect(dc, x0 as i32, y0 as i32, (x0 + w).ceil() as i32, (y0 + h).ceil() as i32);
        let base = DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
        let left = base | DT_LEFT | DT_VCENTER;
        let right = base | DT_RIGHT | DT_VCENTER;
        let center = base | DT_CENTER | DT_VCENTER;
        let (fg, fg2, accent) = (th.fg, th.fg2, th.accent);
        let st = GetLocalTime();
        let clock = format!("{:02}:{:02}", st.wHour, st.wMinute);

        match mode {
            Mode::Collapsed => match self.collapsed_view(m) {
                0 => {
                    let col = if self.timer_end.is_some() { accent } else { fg2 };
                    self.txt(self.font_main, &fmt_timer(self.timer_secs()), x0 + 36.0 * s, y0, x0 + w - 12.0 * s, y0 + h, col, f, center);
                }
                1 => {
                    self.txt(self.font_main, &fmt_sw(self.sw_elapsed()), x0 + 36.0 * s, y0, x0 + w - 10.0 * s, y0 + h, th.good, f, center);
                }
                2 => {
                    self.txt(self.font_small, &m.title, x0 + 40.0 * s, y0, x0 + w - 40.0 * s, y0 + h, fg, f, center);
                }
                5 => {
                    let ly = self.lyrics.lock().unwrap();
                    let pos = m.cur_pos() + 0.25;
                    let ci: isize = ly.lines.iter().rposition(|(t0, _)| *t0 <= pos).map(|i| i as isize).unwrap_or(-1);
                    let text: &str = if ci < 0 { m.title.as_str() } else { lyric_line(&ly.lines, ci) };
                    self.txt(self.font_main, text, x0 + 14.0 * s, y0, x0 + w - 14.0 * s, y0 + h, fg, f, center);
                }
                3 => {
                    self.txt(self.font_main, &clock, x0 + 10.0 * s, y0, x0 + w - 10.0 * s, y0 + h, fg, f, center);
                }
                _ => {}
            },
            Mode::Expanded => {
                match page {
                    Page::Music => {
                        if m.has {
                            self.txt(self.font_main, &m.title, cxm - 126.0 * s, y0 + 44.0 * s, cxm + 160.0 * s, y0 + 66.0 * s, fg, f, left);
                            self.txt(self.font_small, &m.artist, cxm - 126.0 * s, y0 + 67.0 * s, cxm + 160.0 * s, y0 + 85.0 * s, fg2, f, left);
                            if m.dur > 0.0 {
                                self.txt(self.font_small, &fmt_time(m.cur_pos()), cxm - 208.0 * s, y0 + 98.0 * s, cxm - 154.0 * s, y0 + 118.0 * s, fg2, f, right);
                                self.txt(self.font_small, &fmt_time(m.dur), cxm + 154.0 * s, y0 + 98.0 * s, cxm + 208.0 * s, y0 + 118.0 * s, fg2, f, left);
                            }
                        } else {
                            self.txt(self.font_small, "Nada reproduciéndose", cxm - 200.0 * s, y0 + 40.0 * s, cxm + 200.0 * s, y0 + 92.0 * s, fg2, f, center);
                        }
                    }
                    Page::Lyrics => {
                        let (lx0, lx1) = (cxm - 200.0 * s, cxm + 200.0 * s);
                        if !m.has {
                            self.txt(self.font_small, "Nada reproduciéndose", lx0, y0 + 40.0 * s, lx1, y0 + 140.0 * s, fg2, f, center);
                        } else {
                            let head = format!("{} — {}", m.title, m.artist);
                            self.txt(self.font_small, &head, lx0, y0 + 38.0 * s, lx1, y0 + 56.0 * s, fg2, f, center);
                            let ly = self.lyrics.lock().unwrap();
                            match ly.state {
                                1 => self.txt(self.font_main, "Buscando letras…", lx0, y0 + 60.0 * s, lx1, y0 + 130.0 * s, fg, f, center),
                                3 => self.txt(self.font_main, "No se encontraron letras", lx0, y0 + 60.0 * s, lx1, y0 + 130.0 * s, fg, f, center),
                                2 => {
                                    if !ly.lines.is_empty() {
                                        let pos = m.cur_pos() + 0.25;
                                        let ci: isize = ly.lines.iter().rposition(|(t0, _)| *t0 <= pos).map(|i| i as isize).unwrap_or(-1);
                                        let lines = ly.lines.as_slice();
                                        self.txt(self.font_small, lyric_line(lines, ci - 1), lx0, y0 + 62.0 * s, lx1, y0 + 82.0 * s, fg2, f, center);
                                        self.txt(self.font_main, lyric_line(lines, ci), lx0, y0 + 84.0 * s, lx1, y0 + 110.0 * s, fg, f, center);
                                        self.txt(self.font_small, lyric_line(lines, ci + 1), lx0, y0 + 112.0 * s, lx1, y0 + 132.0 * s, fg2, f, center);
                                    } else {
                                        let mut yy = y0 + 60.0 * s;
                                        for line in ly.plain.iter().filter(|l| !l.is_empty()).take(4) {
                                            self.txt(self.font_small, line, lx0, yy, lx1, yy + 20.0 * s, fg, f, center);
                                            yy += 20.0 * s;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Page::Clock => {
                        const D: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
                        let date = format!("{} {} de {}", D[(st.wDayOfWeek as usize) % 7], st.wDay, MONTHS_ES[((st.wMonth as usize).max(1) - 1) % 12]);
                        self.txt(self.font_big, &clock, cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 82.0 * s, fg, f, center);
                        self.txt(self.font_small, &date, cxm - 150.0 * s, y0 + 82.0 * s, cxm + 150.0 * s, y0 + 102.0 * s, fg2, f, center);
                        if self.batt.has {
                            self.txt(self.font_small, &format!("{}%", self.batt.pct), cxm + 4.0 * s, y0 + 103.0 * s, cxm + 70.0 * s, y0 + 121.0 * s, fg2, f, left);
                        }
                    }
                    Page::Calendar => {
                        let title = format!("{} {}", MONTHS_ES[(self.cal_month as usize - 1) % 12], self.cal_year);
                        self.txt(self.font_main, &title, cxm - 112.0 * s, y0 + 40.0 * s, cxm + 112.0 * s, y0 + 64.0 * s, fg, f, center);
                        for (i, l) in ["L", "M", "X", "J", "V", "S", "D"].iter().enumerate() {
                            let x = cxm - 140.0 * s + i as f32 * 40.0 * s;
                            self.txt(self.font_small, l, x, y0 + 66.0 * s, x + 40.0 * s, y0 + 84.0 * s, fg2, f, center);
                        }
                        let first = weekday_mon0(self.cal_year, self.cal_month, 1);
                        let n = days_in_month(self.cal_year, self.cal_month);
                        let is_cur = st.wYear as i32 == self.cal_year && st.wMonth as u32 == self.cal_month;
                        for d in 1..=n {
                            let idx = first + d - 1;
                            let (row, col) = ((idx / 7) as f32, idx % 7);
                            let x = cxm - 140.0 * s + col as f32 * 40.0 * s;
                            let y = y0 + 88.0 * s + row * 24.0 * s;
                            let color = if is_cur && d == st.wDay as u32 {
                                th.on_accent
                            } else if col >= 5 {
                                fg2
                            } else {
                                fg
                            };
                            self.txt(self.font_small, &d.to_string(), x, y, x + 40.0 * s, y + 24.0 * s, color, f, center);
                        }
                    }
                    Page::Timer => {
                        let col = if self.timer_end.is_some() {
                            accent
                        } else if self.timer_paused.is_some() {
                            fg2
                        } else {
                            fg
                        };
                        self.txt(self.font_big, &fmt_timer(self.timer_secs()), cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 86.0 * s, col, f, center);
                    }
                    Page::Stopwatch => {
                        self.txt(self.font_big, &fmt_sw(self.sw_elapsed()), cxm - 150.0 * s, y0 + 36.0 * s, cxm + 150.0 * s, y0 + 84.0 * s, fg, f, center);
                        let n = self.laps.len();
                        if n > 0 {
                            let prev = if n >= 2 { self.laps[n - 2] } else { 0.0 };
                            let line = format!("Vuelta {} · {}", n, fmt_sw(self.laps[n - 1] - prev));
                            self.txt(self.font_small, &line, cxm - 150.0 * s, y0 + 84.0 * s, cxm + 150.0 * s, y0 + 100.0 * s, fg2, f, center);
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
                        self.txt(self.font_small, &tail_chars(&line, 30), cxm - 168.0 * s, y0 + 38.0 * s, cxm + 168.0 * s, y0 + 54.0 * s, fg2, f, right);
                        let big = if self.calc_err {
                            "Error".to_string()
                        } else if self.calc_expr.is_empty() {
                            "0".to_string()
                        } else {
                            tail_chars(&disp_expr(&self.calc_expr), 16)
                        };
                        self.txt(self.font_big, &big, cxm - 168.0 * s, y0 + 54.0 * s, cxm + 168.0 * s, y0 + 90.0 * s, fg, f, right);
                    }
                    Page::Todo => {
                        if self.todos.is_empty() {
                            self.txt(self.font_small, "Sin tareas. Escribe arriba y pulsa Enter.", cxm - 196.0 * s, y0 + 80.0 * s, cxm + 196.0 * s, y0 + 110.0 * s, fg2, f, center);
                        }
                    }
                    Page::Settings => {}
                }

                // etiquetas de los controles
                for wd in wl {
                    let (l, t, r, b) = (wd.x, wd.y, wd.x + wd.w, wd.y + wd.h);
                    match wd.kind {
                        WKind::Tab(_) => {
                            if wd.on {
                                self.txt(self.font_small, wd.label, l + 28.0 * s, t, r, b, fg, f, left);
                            }
                        }
                        WKind::Pill => self.txt(self.font_small, wd.label, l, t, r, b, fg, f, center),
                        WKind::Chip => {
                            let col = if wd.on { fg } else { fg2 };
                            self.txt(self.font_small, wd.label, l, t, r, b, col, f, center);
                        }
                        WKind::Key(cls) => {
                            let col = match cls {
                                0 => fg,
                                1 => th.key_fn_fg,
                                _ => th.on_accent,
                            };
                            self.txt(self.font_main, wd.label, l, t, r, b, col, f, center);
                        }
                        WKind::Toggle => self.txt(self.font_small, wd.label, l + 4.0 * s, t, r - 46.0 * s, b, fg, f, left),
                        WKind::Label => self.txt(self.font_small, wd.label, l, t, r, b, fg, f, left),
                        WKind::Selector => self.txt(self.font_small, wd.label2, l, t, r, b, fg, f, center),
                        WKind::Slider => {
                            let val = match wd.act {
                                Act::Slider(0) => format!("{}%", (self.cfg.op_min * 100.0).round() as i32),
                                _ => format!("{} px", self.cfg.prox_dist.round() as i32),
                            };
                            self.txt(self.font_small, &val, r + 6.0 * s, t, r + 62.0 * s, b, fg2, f, left);
                        }
                        WKind::Check => {
                            if let Act::TodoToggle(i) = wd.act {
                                if i < self.todos.len() {
                                    let (done, text) = (&self.todos[i].0, self.todos[i].1.as_str());
                                    let font = if *done { self.font_strike } else { self.font_small };
                                    let col = if *done { fg2 } else { fg };
                                    self.txt(font, text, l + 28.0 * s, t, r - 6.0 * s, b, col, f, left);
                                }
                            }
                        }
                        WKind::Input => {
                            if self.todo_input.is_empty() && !self.editing {
                                self.txt(self.font_small, "Añadir tarea…", l + 10.0 * s, t, r - 8.0 * s, b, fg2, f, left);
                            } else {
                                let shown = format!("{}{}", self.todo_input, if self.editing { "|" } else { "" });
                                self.txt(self.font_small, &tail_chars(&shown, 46), l + 10.0 * s, t, r - 8.0 * s, b, fg, f, left);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Mode::Toast => {
                if let Some(t) = &self.toast {
                    self.txt(self.font_main, &t.text, x0 + 58.0 * s, y0, x0 + w - 16.0 * s, y0 + h, fg, f, left);
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
        let th = self.th;
        let (pw, ph) = (self.pw as usize, self.ph as usize);
        let buf: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(self.bits, pw * ph) };
        buf.fill(0);
        self.cov.fill(0);
        self.sh.fill(0);

        let (x0, y0, w, h) = self.pill_rect();
        let notch = self.notch();
        let r = if th.sharp { (3.0 * s).min(h / 2.0) } else { (h / 2.0).min(26.0 * s) };
        let ext = if notch { 40.0 * s } else { 0.0 };
        let (sy, shh) = (y0 - ext, h + ext);
        let shape = Shape {
            cx: x0 + w / 2.0,
            cy: sy + shh / 2.0,
            hx: w / 2.0 - r,
            hy: shh / 2.0 - r,
            r,
            notch,
            ex0: x0,
            ex1: x0 + w,
            e: 14.0 * s,
        };
        let e = if notch { shape.e } else { 0.0 };
        let (cx, cy, hx, hy) = (shape.cx, shape.cy, shape.hx, shape.hy);

        let xa = ((x0 - e).floor() as i32 - 1).max(0) as usize;
        let xb = ((x0 + w + e).ceil() as i32 + 1).clamp(0, pw as i32) as usize;
        let ya = if notch { 0 } else { (y0.floor() as i32 - 1).max(0) as usize };
        let yb = ((y0 + h).ceil() as i32 + 1).clamp(0, ph as i32) as usize;

        // sombra (solo estilo isla; se salta el interior de la pildora)
        let shadow_on = self.feat(F_SHADOW) && !notch;
        let (bl, off) = (20.0 * s, 5.0 * s);
        let (sxa, sxb, sya, syb) = if shadow_on {
            (
                (x0 - bl - 2.0).floor().max(0.0) as usize,
                ((x0 + w + bl + 2.0).ceil() as usize).min(pw),
                (y0 - bl).floor().max(0.0) as usize,
                ((y0 + h + bl + off + 2.0).ceil() as usize).min(ph),
            )
        } else {
            (xa, xb, ya, yb)
        };
        if shadow_on {
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

        // pildora (relleno del tema + borde); el interior se rellena de golpe
        let bgp = pack(th.bg);
        let ix0 = ((x0 + 2.0 * s).ceil() as usize).min(xb);
        let ix1 = ((x0 + w - 2.0 * s).floor().max(0.0) as usize).min(xb);
        let iy0 = if notch { 0 } else { ((y0 + r).ceil() as usize).min(yb) };
        let iy1 = ((y0 + h - r).floor().max(0.0) as usize).min(yb);
        for y in ya..yb {
            if y >= iy0 && y < iy1 && ix0 < ix1 {
                for x in xa..ix0 {
                    pill_pixel(x, y, pw, &shape, s, bgp, th.ring, &mut self.cov, buf);
                }
                let row = y * pw;
                self.cov[row + ix0..row + ix1].fill(255);
                buf[row + ix0..row + ix1].fill(bgp);
                for x in ix1..xb {
                    pill_pixel(x, y, pw, &shape, s, bgp, th.ring, &mut self.cov, buf);
                }
            } else {
                for x in xa..xb {
                    pill_pixel(x, y, pw, &shape, s, bgp, th.ring, &mut self.cov, buf);
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
            let mut cv = Canvas { b: &mut *buf, pw, ph, th };
            if th.frame && self.mode == Mode::Expanded {
                cv.frame(x0, y0, w, h, s);
            }
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
                let shv = self.sh[i] as u32;
                let a = c + shv * (255 - c) / 255;
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
                SourceConstantAlpha: (self.opacity.clamp(0.05, 1.0) * 255.0) as u8,
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
                    let cc = if d > -2.0 { 225.0 } else { 0.0 };
                    let al = (a * 255.0) as u32;
                    let v = (cc * a) as u32;
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
    for (i, ch) in "Dynamic Island".encode_utf16().enumerate() {
        nid.szTip[i] = ch;
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
        let _ = AppendMenuW(menu, MF_STRING, 150, w!("Restablecer posición"));
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
                    T_PROX => a.prox_tick(),
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
            WM_LBUTTONDOWN => {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                with_app(|a| a.on_down(x, y));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                with_app(|a| a.on_up(x, y));
                LRESULT(0)
            }
            WM_CHAR => {
                let ch = char::from_u32(wp.0 as u32).unwrap_or('\0');
                with_app(|a| a.on_char(ch));
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                with_app(|a| a.end_edit(false));
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
                with_app(|a| a.on_wheel(delta));
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
            WM_MOUSEACTIVATE => {
                let mut ed = false;
                with_app(|a| ed = a.editing);
                LRESULT(if ed { MA_ACTIVATE_ } else { MA_NOACTIVATE_ })
            }
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
        let home = POINT { x: (sw - pw) / 2, y: 0 };

        let (features, cfg) = load_config();
        MUSIC_ON.store(features[F_MUSIC], Ordering::Relaxed);
        COVER_ON.store(features[F_COVER], Ordering::Relaxed);
        let th = theme_of(cfg.theme);
        let start = if cfg.style == 1 { home } else { POINT { x: home.x + cfg.dx, y: home.y + cfg.dy } };

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("DynamicIslandWnd"),
            w!("DynamicIsland"),
            WS_POPUP,
            start.x,
            start.y,
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

        let face: &str = if cfg.font == 0 { th.font } else { FONTS[cfg.font as usize] };
        let n = (pw * ph) as usize;
        let long_ago = Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(Instant::now);
        let now_t = GetLocalTime();
        let app = App {
            hwnd,
            mem,
            bits: bits as *mut u32,
            pw,
            ph,
            pos: start,
            home,
            s,
            font_main: make_font(face, (15.0 * s) as i32, 600, false),
            font_small: make_font(face, (13.0 * s) as i32, 400, false),
            font_big: make_font(face, (30.0 * s) as i32, 300, false),
            font_strike: make_font(face, (13.0 * s) as i32, 400, true),
            cfg,
            th,
            cur: [120.0, 34.0],
            vel: [0.0, 0.0],
            mode: Mode::Collapsed,
            page: Page::Clock,
            prev_view: None,
            layout_key: (Mode::Collapsed, Page::Clock, false, 3),
            lyr_idx: -2,
            fade: 1.0,
            opacity: 1.0,
            opacity_target: 1.0,
            hover: false,
            pinned: false,
            editing: false,
            hover_act: None,
            tracking: false,
            animating: false,
            vis_on: false,
            prox_on: false,
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
            settings_tab: 0,
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
            cal_year: now_t.wYear as i32,
            cal_month: now_t.wMonth as u32,
            todos: load_todos(),
            todo_input: String::new(),
            todo_scroll: 0,
            lyrics: Arc::new(Mutex::new(Lyrics::default())),
            lyr_key: (String::new(), String::new()),
            slider_drag: None,
            drag: None,
            prev_fg: 0,
        };
        APP.with(|c| *c.borrow_mut() = Some(app));

        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        tray_add(hwnd);

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        with_app(|a| {
            a.apply_pos();
            a.ensure_page();
            a.render();
            if a.feat(F_PROX) {
                SetTimer(a.hwnd, T_PROX, 150, None);
                a.prox_on = true;
            }
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
