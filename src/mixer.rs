//! Mezclador de volumen con Core Audio (WASAPI): volumen general y por aplicación.
//!
//! Un hilo propio con COM. Solo consulta los dispositivos mientras la pestaña está
//! visible; el resto del tiempo duerme (consumo nulo).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::Thread;
use std::time::Duration;

use windows::core::{Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

#[derive(Clone, Default, PartialEq)]
pub struct AppVol {
    pub name: String,
    pub vol: f32,
    pub muted: bool,
    pub active: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct MixState {
    pub ok: bool,
    pub master: f32,
    pub muted: bool,
    pub apps: Vec<AppVol>,
}

/// Dispositivo de salida de audio.
#[derive(Clone, Default)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// Dispositivos de salida y el elegido para una aplicación (`current` vacío = predeterminado).
#[derive(Clone, Default)]
pub struct DeviceList {
    pub devices: Vec<Device>,
    pub current: String,
}

enum Cmd {
    Master(f32),
    MasterMute(bool),
    App(String, f32),
    AppMute(String, bool),
    Devices(String, Sender<DeviceList>),
    SetDevice(String, String),
}

#[derive(Clone)]
pub struct Mixer {
    tx: Sender<Cmd>,
    state: Arc<Mutex<MixState>>,
    active: Arc<AtomicBool>,
    thread: Thread,
}

impl Mixer {
    /// `hwnd` y `msg`: ventana y mensaje a los que se avisa cuando cambia el estado.
    pub fn start(hwnd: isize, msg: u32) -> Mixer {
        let (tx, rx) = channel::<Cmd>();
        let state = Arc::new(Mutex::new(MixState::default()));
        let active = Arc::new(AtomicBool::new(false));
        let (s2, a2) = (state.clone(), active.clone());
        let h = std::thread::spawn(move || worker(rx, s2, a2, hwnd, msg));
        Mixer { tx, state, active, thread: h.thread().clone() }
    }

    pub fn state(&self) -> MixState {
        self.state.lock().unwrap().clone()
    }

    pub fn set_active(&self, on: bool) {
        let was = self.active.swap(on, Ordering::AcqRel);
        if on && !was {
            self.thread.unpark();
        }
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        self.thread.unpark();
    }

    pub fn set_master(&self, v: f32) {
        self.send(Cmd::Master(v));
    }

    pub fn set_master_mute(&self, m: bool) {
        self.send(Cmd::MasterMute(m));
    }

    pub fn set_app(&self, name: &str, v: f32) {
        self.send(Cmd::App(name.to_string(), v));
    }

    pub fn set_app_mute(&self, name: &str, m: bool) {
        self.send(Cmd::AppMute(name.to_string(), m));
    }

    /// Lista las salidas disponibles y la elegida para `app` (espera la respuesta del hilo).
    pub fn devices(&self, app: &str) -> Option<DeviceList> {
        let (tx, rx) = channel::<DeviceList>();
        self.send(Cmd::Devices(app.to_string(), tx));
        rx.recv_timeout(Duration::from_millis(2000)).ok()
    }

    /// Cambia la salida de una aplicación; `id` vacío = predeterminado del sistema.
    pub fn set_device(&self, app: &str, id: &str) {
        self.send(Cmd::SetDevice(app.to_string(), id.to_string()));
    }
}

fn worker(rx: Receiver<Cmd>, state: Arc<Mutex<MixState>>, active: Arc<AtomicBool>, hwnd: isize, msg: u32) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    loop {
        let mut touched = false;
        loop {
            match rx.try_recv() {
                Ok(c) => match c {
                    Cmd::Devices(name, reply) => {
                        let _ = reply.send(list_devices(&name));
                    }
                    other => {
                        apply(&other);
                        touched = true;
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let on = active.load(Ordering::Acquire);
        if on || touched {
            let new = read().unwrap_or_default();
            let changed = {
                let mut s = state.lock().unwrap();
                if *s != new {
                    *s = new;
                    true
                } else {
                    false
                }
            };
            if changed {
                unsafe {
                    let _ = PostMessageW(HWND(hwnd as _), msg, WPARAM(0), LPARAM(0));
                }
            }
        }
        if on {
            std::thread::park_timeout(Duration::from_millis(400));
        } else {
            std::thread::park();
        }
    }
}

unsafe fn default_device() -> windows::core::Result<IMMDevice> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    en.GetDefaultAudioEndpoint(eRender, eConsole)
}

fn read() -> windows::core::Result<MixState> {
    unsafe {
        let dev = default_device()?;
        let ep: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
        let master = ep.GetMasterVolumeLevelScalar()?;
        let muted = ep.GetMute()?.as_bool();

        let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
        let en = mgr.GetSessionEnumerator()?;
        let n = en.GetCount()?;
        let mut apps: Vec<AppVol> = Vec::new();
        for i in 0..n {
            let Ok(ctl) = en.GetSession(i) else { continue };
            let Ok(state) = ctl.GetState() else { continue };
            if state == AudioSessionStateExpired {
                continue;
            }
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else { continue };
            let name = session_name(&ctl, &ctl2);
            let v = vol.GetMasterVolume().unwrap_or(1.0);
            let m = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            let active = state == AudioSessionStateActive;
            if let Some(a) = apps.iter_mut().find(|a| a.name == name) {
                a.active |= active;
            } else {
                apps.push(AppVol { name, vol: v, muted: m, active });
            }
        }
        // primero las que suenan, luego orden alfabético
        apps.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.cmp(&b.name)));
        apps.truncate(6);
        Ok(MixState { ok: true, master, muted, apps })
    }
}

fn apply(c: &Cmd) {
    unsafe {
        let Ok(dev) = default_device() else { return };
        match c {
            Cmd::Master(v) => {
                if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let _ = ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
                }
            }
            Cmd::MasterMute(m) => {
                if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let _ = ep.SetMute(BOOL::from(*m), std::ptr::null());
                }
            }
            Cmd::App(name, v) => {
                let v = v.clamp(0.0, 1.0);
                for_sessions(&dev, name, |vol| {
                    let _ = vol.SetMasterVolume(v, std::ptr::null());
                });
            }
            Cmd::Devices(..) => {}
            Cmd::SetDevice(name, id) => {
                for pid in pids_for(name) {
                    set_persisted(pid, id);
                }
            }
            Cmd::AppMute(name, m) => {
                let m = *m;
                for_sessions(&dev, name, |vol| {
                    let _ = vol.SetMute(BOOL::from(m), std::ptr::null());
                });
            }
        }
    }
}

unsafe fn for_sessions(dev: &IMMDevice, name: &str, f: impl Fn(&ISimpleAudioVolume)) {
    let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return };
    let Ok(en) = mgr.GetSessionEnumerator() else { return };
    let n = en.GetCount().unwrap_or(0);
    for i in 0..n {
        let Ok(ctl) = en.GetSession(i) else { continue };
        let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
        if session_name(&ctl, &ctl2) != name {
            continue;
        }
        if let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() {
            f(&vol);
        }
    }
}

unsafe fn session_name(ctl: &IAudioSessionControl, ctl2: &IAudioSessionControl2) -> String {
    // S_OK (0) = es la sesión de sonidos del sistema
    if ctl2.IsSystemSoundsSession().0 == 0 {
        return "Sonidos del sistema".to_string();
    }
    let pid = ctl2.GetProcessId().unwrap_or(0);
    if pid != 0 {
        if let Some(n) = exe_name(pid) {
            return n;
        }
    }
    if let Ok(p) = ctl.GetDisplayName() {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const c_void));
        if !s.is_empty() {
            return s;
        }
    }
    "Aplicación".to_string()
}

fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let stem = std::path::Path::new(&path).file_stem()?.to_string_lossy().to_string();
        Some(friendly(&stem))
    }
}

fn friendly(stem: &str) -> String {
    let low = stem.to_lowercase();
    const KNOWN: [(&str, &str); 14] = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vivaldi", "Vivaldi"),
        ("vlc", "VLC"),
        ("foobar", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("discord", "Discord"),
        ("teams", "Teams"),
        ("zoom", "Zoom"),
        ("steam", "Steam"),
    ];
    for (k, n) in KNOWN.iter() {
        if low.contains(*k) {
            return n.to_string();
        }
    }
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}


// ---------------------------------------------------------------- salida por aplicación
//
// Windows guarda la salida de cada aplicación con una interfaz NO documentada
// (`Windows.Media.Internal.AudioPolicyConfig`, la misma que usan EarTrumpet o SoundSwitch).
// Puede cambiar entre versiones de Windows; si falla, simplemente no tiene efecto.

#[link(name = "runtimeobject")]
extern "system" {
    fn RoGetActivationFactory(class_id: *mut c_void, iid: *const GUID, factory: *mut *mut c_void) -> i32;
}

const IID_21H2: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);
const IID_DOWNLEVEL: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);
// IUnknown (3) + IInspectable (3) + 19 métodos sin documentar
const SLOT_SET: usize = 25;
const SLOT_GET: usize = 26;

unsafe fn policy_factory() -> *mut c_void {
    let cls = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
    let raw: *mut c_void = std::mem::transmute_copy(&cls);
    for iid in [&IID_21H2, &IID_DOWNLEVEL] {
        let mut out: *mut c_void = std::ptr::null_mut();
        if RoGetActivationFactory(raw, iid, &mut out) >= 0 && !out.is_null() {
            return out;
        }
    }
    std::ptr::null_mut()
}

unsafe fn vtbl_fn(obj: *mut c_void, slot: usize) -> usize {
    let vtbl = *(obj as *const *const usize);
    *vtbl.add(slot)
}

unsafe fn release(obj: *mut c_void) {
    let rel: extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(vtbl_fn(obj, 2));
    rel(obj);
}

fn device_path(id: &str) -> String {
    format!("\\\\?\\SWD#MMDEVAPI#{}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}", id)
}

/// Fija la salida de un proceso; `device_id` vacío = predeterminado del sistema.
fn set_persisted(pid: u32, device_id: &str) {
    unsafe {
        let f = policy_factory();
        if f.is_null() {
            return;
        }
        let set: extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> i32 = std::mem::transmute(vtbl_fn(f, SLOT_SET));
        let hs = if device_id.is_empty() { None } else { Some(HSTRING::from(device_path(device_id))) };
        let raw: *mut c_void = match &hs {
            Some(h) => std::mem::transmute_copy(h),
            None => std::ptr::null_mut(),
        };
        // 0 = eRender; roles 0 (consola) y 1 (multimedia)
        for role in [0i32, 1] {
            let _ = set(f, pid, 0, role, raw);
        }
        release(f);
    }
}

/// Id del dispositivo elegido para un proceso (None = predeterminado).
fn get_persisted(pid: u32) -> Option<String> {
    unsafe {
        let f = policy_factory();
        if f.is_null() {
            return None;
        }
        let get: extern "system" fn(*mut c_void, u32, i32, i32, *mut *mut c_void) -> i32 = std::mem::transmute(vtbl_fn(f, SLOT_GET));
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = get(f, pid, 0, 1, &mut raw);
        release(f);
        if hr < 0 || raw.is_null() {
            return None;
        }
        let hs: HSTRING = std::mem::transmute(raw); // toma la propiedad y la libera al salir
        let path = hs.to_string();
        let a = path.find("MMDEVAPI#")? + "MMDEVAPI#".len();
        let rest = &path[a..];
        let b = rest.find('#').unwrap_or(rest.len());
        let id = rest[..b].to_string();
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    }
}

unsafe fn pids_for(app: &str) -> Vec<u32> {
    let mut v: Vec<u32> = Vec::new();
    let Ok(dev) = default_device() else { return v };
    let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return v };
    let Ok(en) = mgr.GetSessionEnumerator() else { return v };
    for i in 0..en.GetCount().unwrap_or(0) {
        let Ok(ctl) = en.GetSession(i) else { continue };
        let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
        if session_name(&ctl, &ctl2) != app {
            continue;
        }
        let pid = ctl2.GetProcessId().unwrap_or(0);
        if pid != 0 && !v.contains(&pid) {
            v.push(pid);
        }
    }
    v
}

unsafe fn device_friendly_name(dev: &IMMDevice) -> String {
    let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { return "Dispositivo".to_string() };
    let Ok(mut pv) = store.GetValue(&PKEY_Device_FriendlyName) else { return "Dispositivo".to_string() };
    // El nombre viene como cadena (VT_LPWSTR = 31) dentro de la union de PROPVARIANT:
    // `vt` en el desplazamiento 0 y el puntero a la cadena en el 8 (Windows de 64 bits).
    let base = &pv as *const _ as *const u8;
    let vt = *(base as *const u16);
    let p = *(base.add(8) as *const *const u16);
    let name = if vt == 31 && !p.is_null() {
        PWSTR(p as *mut u16).to_string().unwrap_or_default()
    } else {
        String::new()
    };
    let _ = PropVariantClear(&mut pv);
    if name.is_empty() {
        "Dispositivo".to_string()
    } else {
        name
    }
}

fn list_devices(app: &str) -> DeviceList {
    unsafe {
        let mut out = DeviceList::default();
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return out;
        };
        if let Ok(coll) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            let n = coll.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(d) = coll.Item(i) else { continue };
                let Ok(idp) = d.GetId() else { continue };
                let id = idp.to_string().unwrap_or_default();
                CoTaskMemFree(Some(idp.0 as *const c_void));
                out.devices.push(Device { name: device_friendly_name(&d), id });
            }
        }
        if let Some(pid) = pids_for(app).first() {
            out.current = get_persisted(*pid).unwrap_or_default();
        }
        out
    }
}
