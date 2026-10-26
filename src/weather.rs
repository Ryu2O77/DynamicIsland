//! Clima con Open-Meteo (sin clave de API) y geolocalización por IP opcional.

use std::ffi::c_void;

use windows::core::{Error, HSTRING, PCWSTR};
use windows::Data::Json::{JsonArray, JsonObject};
use windows::Win32::Networking::WinHttp::*;

#[derive(Clone, Default)]
pub struct Day {
    pub label: String,
    pub code: u8,
    pub max: f32,
    pub min: f32,
}

#[derive(Clone, Default)]
pub struct Weather {
    pub state: u8, // 0 sin datos, 1 cargando, 2 listo, 3 error
    pub error: String,
    pub place: String,
    pub city: String,
    pub lat: f32,
    pub lon: f32,
    pub temp: f32,
    pub feels: f32,
    pub humidity: u32,
    pub wind: f32,
    pub code: u8,
    pub is_day: bool,
    pub daily: Vec<Day>,
    pub unit: u8, // 0 °C, 1 °F
}

impl Weather {
    pub fn condition(&self) -> &'static str {
        condition_text(self.code)
    }
}

pub fn condition_text(code: u8) -> &'static str {
    match code {
        0 => "Despejado",
        1 => "Mayormente despejado",
        2 => "Parcialmente nublado",
        3 => "Nublado",
        45 | 48 => "Niebla",
        51 | 53 | 55 => "Llovizna",
        56 | 57 => "Llovizna helada",
        61 => "Lluvia ligera",
        63 => "Lluvia",
        65 => "Lluvia fuerte",
        66 | 67 => "Lluvia helada",
        71 => "Nieve ligera",
        73 => "Nieve",
        75 => "Nieve fuerte",
        77 => "Granos de nieve",
        80 => "Chubascos ligeros",
        81 => "Chubascos",
        82 => "Chubascos fuertes",
        85 | 86 => "Chubascos de nieve",
        95 => "Tormenta",
        96 | 99 => "Tormenta con granizo",
        _ => "—",
    }
}

/// 0 sol, 1 sol y nube, 2 nube, 3 niebla, 4 lluvia, 5 nieve, 6 tormenta.
pub fn icon_kind(code: u8) -> u8 {
    match code {
        0 => 0,
        1 | 2 => 1,
        3 => 2,
        45 | 48 => 3,
        51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => 4,
        71 | 73 | 75 | 77 | 85 | 86 => 5,
        95 | 96 | 99 => 6,
        _ => 2,
    }
}

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

/// GET con WinHTTP (nativo). Evita depender de WinRT/HttpClient y funciona con
/// cadenas de certificados nuevas que WinHTTP acepta sin problemas.
fn http_get(url: &str) -> Result<String, String> {
    unsafe {
        let rest = url.strip_prefix("https://").ok_or("la URL debe ser https")?;
        let (host, path) = match rest.split_once('/') {
            Some((h, p)) => (h, format!("/{p}")),
            None => (rest, "/".to_string()),
        };
        let agent: Vec<u16> = "DynamicIsland/1.0\0".encode_utf16().collect();
        let hostw: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();
        let pathw: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let verb: Vec<u16> = "GET\0".encode_utf16().collect();

        let session = WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return Err(format!("HTTP: {}", Error::from_win32()));
        }
        let conn = WinHttpConnect(session, PCWSTR(hostw.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if conn.is_null() {
            let _ = WinHttpCloseHandle(session);
            return Err(format!("conexión: {}", Error::from_win32()));
        }
        let req = WinHttpOpenRequest(
            conn,
            PCWSTR(verb.as_ptr()),
            PCWSTR(pathw.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if req.is_null() {
            let _ = WinHttpCloseHandle(conn);
            let _ = WinHttpCloseHandle(session);
            return Err(format!("petición: {}", Error::from_win32()));
        }

        // TLS 1.2 y 1.3; el resto de versiones están obsoletas.
        let protos = (WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3).to_le_bytes();
        let _ = WinHttpSetOption(Some(req as *const c_void), WINHTTP_OPTION_SECURE_PROTOCOLS, Some(&protos));

        let result = (|| -> Result<String, String> {
            WinHttpSendRequest(req, None, None, 0, 0, 0).map_err(|e| format!("HTTP: {e}"))?;
            WinHttpReceiveResponse(req, std::ptr::null_mut()).map_err(|e| format!("HTTP: {e}"))?;

            let mut status = 0u32;
            let mut len = std::mem::size_of::<u32>() as u32;
            let _ = WinHttpQueryHeaders(
                req,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut c_void),
                &mut len,
                std::ptr::null_mut(),
            );
            if status != 0 && status != 200 {
                return Err(format!("HTTP {status}"));
            }

            let mut body = Vec::new();
            loop {
                let mut avail = 0u32;
                WinHttpQueryDataAvailable(req, &mut avail).map_err(|e| format!("HTTP: {e}"))?;
                if avail == 0 {
                    break;
                }
                let mut buf = vec![0u8; avail as usize];
                let mut read = 0u32;
                WinHttpReadData(req, buf.as_mut_ptr() as *mut c_void, avail, &mut read)
                    .map_err(|e| format!("HTTP: {e}"))?;
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..read as usize]);
                if body.len() > 8 * 1024 * 1024 {
                    return Err("respuesta demasiado grande".into());
                }
            }
            Ok(String::from_utf8_lossy(&body).into_owned())
        })();

        let _ = WinHttpCloseHandle(req);
        let _ = WinHttpCloseHandle(conn);
        let _ = WinHttpCloseHandle(session);
        result
    }
}

fn s(o: &JsonObject, k: &str) -> String {
    o.GetNamedString(&HSTRING::from(k)).map(|v| v.to_string()).unwrap_or_default()
}

fn n(o: &JsonObject, k: &str) -> f64 {
    o.GetNamedNumber(&HSTRING::from(k)).unwrap_or(0.0)
}

fn arr(o: &JsonObject, k: &str) -> Option<JsonArray> {
    o.GetNamedArray(&HSTRING::from(k)).ok()
}

fn obj(o: &JsonObject, k: &str) -> Option<JsonObject> {
    o.GetNamedObject(&HSTRING::from(k)).ok()
}

fn weekday_short(date: &str) -> String {
    let mut it = date.split('-');
    let y = it.next().and_then(|v| v.parse::<i32>().ok()).unwrap_or(2024);
    let m = it.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
    let d = it.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let dow = (yy + yy / 4 - yy / 100 + yy / 400 + t[(m.clamp(1, 12) - 1) as usize] + d as i32).rem_euclid(7);
    const NAMES: [&str; 7] = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];
    NAMES[dow as usize].to_string()
}

/// Busca una ciudad y devuelve (nombre, latitud, longitud).
pub fn geocode(city: &str) -> Result<(String, f32, f32), String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=es&format=json",
        url_encode(city.trim())
    );
    let body = http_get(&url)?;
    let o = JsonObject::Parse(&HSTRING::from(body.as_str())).map_err(|e| format!("respuesta: {e}"))?;
    let results = arr(&o, "results").ok_or("ciudad no encontrada")?;
    if results.Size().unwrap_or(0) == 0 {
        return Err("ciudad no encontrada".into());
    }
    let r = results.GetObjectAt(0).map_err(|e| format!("resultado: {e}"))?;
    let name = s(&r, "name");
    let country = s(&r, "country");
    let place = if country.is_empty() { name } else { format!("{name}, {country}") };
    let lat = n(&r, "latitude") as f32;
    let lon = n(&r, "longitude") as f32;
    if lat == 0.0 && lon == 0.0 {
        return Err("coordenadas no válidas".into());
    }
    Ok((place, lat, lon))
}

/// Intenta localizar al usuario por IP (sin permisos de ubicación).
pub fn locate() -> Result<(String, f32, f32), String> {
    if let Ok(body) = http_get("https://ipapi.co/json/") {
        if let Ok(o) = JsonObject::Parse(&HSTRING::from(body.as_str())) {
            let lat = n(&o, "latitude") as f32;
            let lon = n(&o, "longitude") as f32;
            if lat != 0.0 || lon != 0.0 {
                let city = s(&o, "city");
                let country = s(&o, "country_name");
                let place = if country.is_empty() { city } else { format!("{city}, {country}") };
                return Ok((place, lat, lon));
            }
        }
    }
    let body = http_get("https://ipwho.is/").map_err(|e| format!("ubicación: {e}"))?;
    let o = JsonObject::Parse(&HSTRING::from(body.as_str())).map_err(|e| format!("respuesta: {e}"))?;
    let lat = n(&o, "latitude") as f32;
    let lon = n(&o, "longitude") as f32;
    if lat == 0.0 && lon == 0.0 {
        return Err("no se pudo detectar la ubicación".into());
    }
    let city = s(&o, "city");
    let country = s(&o, "country");
    let place = if country.is_empty() { city } else { format!("{city}, {country}") };
    Ok((place, lat, lon))
}

/// Descarga el clima actual y los próximos 4 días.
pub fn fetch(lat: f32, lon: f32, unit: u8) -> Result<Weather, String> {
    let (tu, wu) = if unit == 1 { ("fahrenheit", "mph") } else { ("celsius", "kmh") };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,is_day,weather_code,wind_speed_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=4\
         &temperature_unit={tu}&wind_speed_unit={wu}"
    );
    let body = http_get(&url)?;
    let o = JsonObject::Parse(&HSTRING::from(body.as_str())).map_err(|e| format!("respuesta: {e}"))?;
    let cur = obj(&o, "current").ok_or("respuesta sin datos actuales")?;
    let daily = obj(&o, "daily").ok_or("respuesta sin pronóstico")?;

    let mut w = Weather {
        state: 2,
        lat,
        lon,
        unit,
        temp: n(&cur, "temperature_2m") as f32,
        feels: n(&cur, "apparent_temperature") as f32,
        humidity: n(&cur, "relative_humidity_2m").clamp(0.0, 100.0) as u32,
        wind: n(&cur, "wind_speed_10m") as f32,
        code: n(&cur, "weather_code").clamp(0.0, 99.0) as u8,
        is_day: n(&cur, "is_day") > 0.5,
        ..Default::default()
    };

    if let (Some(times), Some(codes), Some(maxs), Some(mins)) = (
        arr(&daily, "time"),
        arr(&daily, "weather_code"),
        arr(&daily, "temperature_2m_max"),
        arr(&daily, "temperature_2m_min"),
    ) {
        let count = times.Size().unwrap_or(0).min(4);
        for i in 0..count {
            let date = times.GetStringAt(i).map(|v| v.to_string()).unwrap_or_default();
            let label = match i {
                0 => "Hoy".to_string(),
                1 => "Mañana".to_string(),
                _ => weekday_short(&date),
            };
            w.daily.push(Day {
                label,
                code: codes.GetNumberAt(i).unwrap_or(0.0).clamp(0.0, 99.0) as u8,
                max: maxs.GetNumberAt(i).unwrap_or(0.0) as f32,
                min: mins.GetNumberAt(i).unwrap_or(0.0) as f32,
            });
        }
    }

    Ok(w)
}
