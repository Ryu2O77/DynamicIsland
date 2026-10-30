//! Lector de EPUB sin dependencias externas: ZIP + inflate (RFC 1951),
//! lectura del OPF/spine, extracción de texto del XHTML y paginación.
//! No usa nada de Windows: la medición de texto la aporta quien llama.

use std::collections::HashMap;

// ------------------------------------------------------------ inflate

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, buf: 0, cnt: 0 }
    }

    /// Lee `n` bits (n <= 16), el menos significativo primero.
    fn need(&mut self, n: u32) -> Result<u32, String> {
        while self.cnt < n {
            if self.pos >= self.d.len() {
                return Err("datos comprimidos truncados".into());
            }
            self.buf |= (self.d[self.pos] as u32) << self.cnt;
            self.pos += 1;
            self.cnt += 8;
        }
        let v = self.buf & ((1u32 << n) - 1);
        self.buf >>= n;
        self.cnt -= n;
        Ok(v)
    }
}

struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}

fn build(lengths: &[u8]) -> Result<Huff, String> {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize] += 1;
    }
    let mut left: i32 = 1;
    for c in count.iter().skip(1) {
        left <<= 1;
        left -= *c as i32;
        if left < 0 {
            return Err("tabla de Huffman inválida".into());
        }
    }
    let mut offs = [0u16; 16];
    for len in 1..15 {
        offs[len + 1] = offs[len] + count[len];
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (sym, &l) in lengths.iter().enumerate() {
        if l != 0 {
            symbol[offs[l as usize] as usize] = sym as u16;
            offs[l as usize] += 1;
        }
    }
    Ok(Huff { count, symbol })
}

fn decode(b: &mut Bits, h: &Huff) -> Result<u16, String> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for len in 1..16 {
        code |= b.need(1)? as i32;
        let count = h.count[len] as i32;
        if code - count < first {
            return Ok(h.symbol[(index + (code - first)) as usize]);
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err("código de Huffman inválido".into())
}

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff, limit: usize) -> Result<(), String> {
    loop {
        let sym = decode(b, lit)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err("símbolo de longitud inválido".into());
            }
            let len = LBASE[s] as usize + b.need(LEXT[s] as u32)? as usize;
            let ds = decode(b, dist)? as usize;
            if ds >= 30 {
                return Err("símbolo de distancia inválido".into());
            }
            let d = DBASE[ds] as usize + b.need(DEXT[ds] as u32)? as usize;
            if d > out.len() {
                return Err("distancia fuera de rango".into());
            }
            let start = out.len() - d;
            for i in 0..len {
                let c = out[start + i];
                out.push(c);
            }
        }
        if out.len() > limit {
            return Err("el archivo descomprimido es demasiado grande".into());
        }
    }
}

/// Descomprime un flujo deflate "crudo" (el que usa ZIP).
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut b = Bits::new(data);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.need(1)?;
        let ty = b.need(2)?;
        match ty {
            0 => {
                // los bits sobrantes son los del byte actual: se descartan
                b.buf = 0;
                b.cnt = 0;
                if b.pos + 4 > data.len() {
                    return Err("bloque almacenado truncado".into());
                }
                let len = data[b.pos] as usize | (data[b.pos + 1] as usize) << 8;
                let nlen = data[b.pos + 2] as usize | (data[b.pos + 3] as usize) << 8;
                if len != (!nlen & 0xFFFF) {
                    return Err("bloque almacenado corrupto".into());
                }
                b.pos += 4;
                if b.pos + len > data.len() {
                    return Err("bloque almacenado truncado".into());
                }
                out.extend_from_slice(&data[b.pos..b.pos + len]);
                b.pos += len;
                if out.len() > limit {
                    return Err("el archivo descomprimido es demasiado grande".into());
                }
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lit = build(&l)?;
                let dist = build(&[5u8; 30])?;
                codes(&mut b, &mut out, &lit, &dist, limit)?;
            }
            2 => {
                let nlen = b.need(5)? as usize + 257;
                let ndist = b.need(5)? as usize + 1;
                let ncode = b.need(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("bloque dinámico inválido".into());
                }
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut cl = [0u8; 19];
                for &o in ORDER.iter().take(ncode) {
                    cl[o] = b.need(3)? as u8;
                }
                let lencode = build(&cl)?;
                let total = nlen + ndist;
                let mut lengths = [0u8; 320];
                let mut idx = 0;
                while idx < total {
                    let sym = decode(&mut b, &lencode)?;
                    if sym < 16 {
                        lengths[idx] = sym as u8;
                        idx += 1;
                    } else {
                        let (prev, rep) = match sym {
                            16 => {
                                if idx == 0 {
                                    return Err("repetición sin longitud previa".into());
                                }
                                (lengths[idx - 1], 3 + b.need(2)? as usize)
                            }
                            17 => (0, 3 + b.need(3)? as usize),
                            _ => (0, 11 + b.need(7)? as usize),
                        };
                        if idx + rep > total {
                            return Err("longitudes de código inválidas".into());
                        }
                        for _ in 0..rep {
                            lengths[idx] = prev;
                            idx += 1;
                        }
                    }
                }
                if lengths[256] == 0 {
                    return Err("falta el código de fin de bloque".into());
                }
                let lit = build(&lengths[..nlen])?;
                let dist = build(&lengths[nlen..total])?;
                codes(&mut b, &mut out, &lit, &dist, limit)?;
            }
            _ => return Err("tipo de bloque inválido".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

// ------------------------------------------------------------ zip

fn u16le(d: &[u8], i: usize) -> usize {
    d[i] as usize | (d[i + 1] as usize) << 8
}

fn u32le(d: &[u8], i: usize) -> usize {
    u16le(d, i) | u16le(d, i + 2) << 16
}

struct Entry {
    name: String,
    method: usize,
    csize: usize,
    off: usize,
}

pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

const MAX_ENTRY: usize = 128 * 1024 * 1024;

impl Zip {
    pub fn open(data: Vec<u8>) -> Result<Zip, String> {
        let n = data.len();
        if n < 22 {
            return Err("no es un archivo EPUB (zip) válido".into());
        }
        let lo = n.saturating_sub(22 + 65535);
        let mut eocd = None;
        let mut i = n - 22;
        loop {
            if u32le(&data, i) == 0x0605_4b50 {
                eocd = Some(i);
                break;
            }
            if i == lo {
                break;
            }
            i -= 1;
        }
        let e = eocd.ok_or("no es un archivo EPUB (zip) válido")?;
        let total = u16le(&data, e + 10);
        let cd_off = u32le(&data, e + 16);
        if total == 0xFFFF || cd_off == 0xFFFF_FFFF {
            return Err("los zip64 no están soportados".into());
        }
        let mut entries = Vec::new();
        let mut p = cd_off;
        for _ in 0..total {
            if p + 46 > n || u32le(&data, p) != 0x0201_4b50 {
                return Err("directorio central del zip dañado".into());
            }
            let flags = u16le(&data, p + 8);
            let method = u16le(&data, p + 10);
            let csize = u32le(&data, p + 20);
            let nlen = u16le(&data, p + 28);
            let elen = u16le(&data, p + 30);
            let clen = u16le(&data, p + 32);
            let off = u32le(&data, p + 42);
            if p + 46 + nlen > n {
                return Err("directorio central del zip dañado".into());
            }
            let name = String::from_utf8_lossy(&data[p + 46..p + 46 + nlen]).to_string();
            if flags & 1 == 0 {
                entries.push(Entry { name, method, csize, off });
            }
            p += 46 + nlen + elen + clen;
        }
        Ok(Zip { data, entries })
    }

    fn find(&self, name: &str) -> Option<&Entry> {
        let want = name.trim_start_matches('/');
        self.entries
            .iter()
            .find(|e| e.name == want)
            .or_else(|| self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(want)))
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let e = self.find(name).ok_or_else(|| format!("falta {} en el EPUB", name))?;
        let d = &self.data;
        if e.off + 30 > d.len() || u32le(d, e.off) != 0x0403_4b50 {
            return Err("cabecera local del zip dañada".into());
        }
        let start = e.off + 30 + u16le(d, e.off + 26) + u16le(d, e.off + 28);
        if start + e.csize > d.len() {
            return Err("entrada del zip truncada".into());
        }
        let raw = &d[start..start + e.csize];
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => inflate(raw, MAX_ENTRY),
            m => Err(format!("método de compresión {} no soportado", m)),
        }
    }
}

// ------------------------------------------------------------ xml / html

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut lit = 0; // inicio del tramo literal pendiente
    while i < b.len() {
        if b[i] == b'&' {
            let end = (i + 1..b.len().min(i + 12)).find(|&j| b[j] == b';');
            if let Some(j) = end {
                let name = &s[i + 1..j];
                if let Some(c) = entity(name) {
                    out.push_str(&s[lit..i]);
                    out.push(c);
                    i = j + 1;
                    lit = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&s[lit..]);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let v = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(h, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return char::from_u32(v);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "shy" => '\u{AD}',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "middot" => '·',
        "copy" => '©',
        "reg" => '®',
        "iexcl" => '¡',
        "iquest" => '¿',
        "aacute" => 'á',
        "eacute" => 'é',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "Aacute" => 'Á',
        "Eacute" => 'É',
        "Iacute" => 'Í',
        "Oacute" => 'Ó',
        "Uacute" => 'Ú',
        "agrave" => 'à',
        "egrave" => 'è',
        "igrave" => 'ì',
        "ograve" => 'ò',
        "ugrave" => 'ù',
        "acirc" => 'â',
        "ecirc" => 'ê',
        "icirc" => 'î',
        "ocirc" => 'ô',
        "ucirc" => 'û',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "uuml" => 'ü',
        "Uuml" => 'Ü',
        "auml" => 'ä',
        "ouml" => 'ö',
        "szlig" => 'ß',
        "ccedil" => 'ç',
        "Ccedil" => 'Ç',
        _ => return None,
    })
}

struct Tag {
    name: String, // minúsculas y sin prefijo de espacio de nombres
    closing: bool,
    attrs: Vec<(String, String)>,
}

fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == 0x0c
}

/// Analiza el elemento que empieza en `i` (`s[i] == '<'`).
/// Devuelve el índice posterior y la etiqueta (None si es comentario, doctype, etc.).
fn parse_tag(s: &str, i: usize) -> (usize, Option<Tag>) {
    let b = s.as_bytes();
    let n = b.len();
    if s[i..].starts_with("<!--") {
        return (s[i + 4..].find("-->").map(|p| i + 4 + p + 3).unwrap_or(n), None);
    }
    if s[i..].starts_with("<![CDATA[") {
        return (s[i + 9..].find("]]>").map(|p| i + 9 + p + 3).unwrap_or(n), None);
    }
    let mut j = i + 1;
    if j >= n {
        return (n, None);
    }
    if b[j] == b'!' || b[j] == b'?' {
        return (s[j..].find('>').map(|p| j + p + 1).unwrap_or(n), None);
    }
    let closing = b[j] == b'/';
    if closing {
        j += 1;
    }
    let ns = j;
    while j < n && !is_ws(b[j]) && b[j] != b'>' && b[j] != b'/' {
        j += 1;
    }
    let full = s[ns..j].to_ascii_lowercase();
    let name = full.rsplit(':').next().unwrap_or("").to_string();
    let mut attrs = Vec::new();
    loop {
        while j < n && is_ws(b[j]) {
            j += 1;
        }
        if j >= n {
            break;
        }
        if b[j] == b'>' {
            j += 1;
            break;
        }
        if b[j] == b'/' {
            j += 1;
            continue;
        }
        let ks = j;
        while j < n && !is_ws(b[j]) && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        let key = s[ks..j].to_ascii_lowercase();
        while j < n && is_ws(b[j]) {
            j += 1;
        }
        let mut val = String::new();
        if j < n && b[j] == b'=' {
            j += 1;
            while j < n && is_ws(b[j]) {
                j += 1;
            }
            if j < n && (b[j] == b'"' || b[j] == b'\'') {
                let q = b[j];
                j += 1;
                let vs = j;
                while j < n && b[j] != q {
                    j += 1;
                }
                val = decode_entities(&s[vs..j]);
                if j < n {
                    j += 1;
                }
            } else {
                let vs = j;
                while j < n && !is_ws(b[j]) && b[j] != b'>' {
                    j += 1;
                }
                val = decode_entities(&s[vs..j]);
            }
        }
        if !key.is_empty() {
            attrs.push((key, val));
        }
    }
    (j, Some(Tag { name, closing, attrs }))
}

struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    text: String, // texto hasta la siguiente etiqueta
}

impl Elem {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
}

/// Elementos de apertura de un XML sencillo (container.xml, OPF).
fn elements(xml: &str) -> Vec<Elem> {
    let mut v = Vec::new();
    let mut i = 0;
    while let Some(p) = xml[i..].find('<') {
        let (end, tag) = parse_tag(xml, i + p);
        i = end;
        if let Some(t) = tag {
            if !t.closing {
                let tend = xml[i..].find('<').map(|q| i + q).unwrap_or(xml.len());
                let text = decode_entities(xml[i..tend].trim());
                v.push(Elem { name: t.name, attrs: t.attrs, text });
            }
        }
    }
    v
}

#[derive(Clone)]
pub struct Para {
    pub heading: bool,
    pub text: String,
}

const SKIP_TAGS: [&str; 5] = ["script", "style", "head", "svg", "title"];
const BLOCK_TAGS: [&str; 26] = [
    "p", "div", "br", "li", "ul", "ol", "tr", "table", "blockquote", "section", "article", "header", "footer", "hr",
    "dt", "dd", "dl", "figure", "figcaption", "aside", "nav", "pre", "body", "td", "th", "caption",
];

fn push_text(cur: &mut String, t: &str) {
    for ch in t.chars() {
        if ch == '\u{AD}' || ch == '\u{200B}' || ch == '\u{FEFF}' {
            continue;
        }
        if ch.is_whitespace() {
            if !cur.is_empty() && !cur.ends_with(' ') {
                cur.push(' ');
            }
        } else if !ch.is_control() {
            cur.push(ch);
        }
    }
}

/// Convierte un documento XHTML en párrafos de texto plano.
pub fn html_to_paras(html: &str) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    let mut cur = String::new();
    let mut heading = false;
    let mut skip: Option<String> = None;
    let mut i = 0;
    let n = html.len();

    let flush = |cur: &mut String, heading: bool, out: &mut Vec<Para>| {
        let t = cur.trim();
        if !t.is_empty() {
            out.push(Para { heading, text: t.to_string() });
        }
        cur.clear();
    };

    while i < n {
        let Some(p) = html[i..].find('<') else {
            if skip.is_none() {
                push_text(&mut cur, &decode_entities(&html[i..]));
            }
            break;
        };
        if p > 0 && skip.is_none() {
            push_text(&mut cur, &decode_entities(&html[i..i + p]));
        }
        let (end, tag) = parse_tag(html, i + p);
        i = end;
        let Some(t) = tag else { continue };
        if let Some(s) = &skip {
            if t.closing && &t.name == s {
                skip = None;
            }
            continue;
        }
        if !t.closing && SKIP_TAGS.contains(&t.name.as_str()) {
            // etiquetas autocerradas (<title/>) no abren una zona omitida
            let self_closed = html[..i].ends_with("/>");
            if !self_closed {
                flush(&mut cur, heading, &mut out);
                skip = Some(t.name.clone());
            }
            continue;
        }
        let is_h = t.name.len() == 2 && t.name.starts_with('h') && t.name.as_bytes()[1].is_ascii_digit();
        if is_h {
            flush(&mut cur, heading, &mut out);
            heading = !t.closing;
        } else if BLOCK_TAGS.contains(&t.name.as_str()) {
            flush(&mut cur, heading, &mut out);
        }
    }
    flush(&mut cur, heading, &mut out);
    out
}

// ------------------------------------------------------------ libro

pub struct Chapter {
    pub title: String,
    pub paras: Vec<Para>,
    pub chars: usize,
}

pub struct Book {
    pub path: String,
    pub title: String,
    #[allow(dead_code)] // reservado para la futura biblioteca
    pub author: String,
    pub chapters: Vec<Chapter>,
    /// caracteres acumulados antes de cada capítulo (para el porcentaje)
    pub before: Vec<usize>,
    pub total_chars: usize,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Une `base` (directorio del OPF) con `href` y resuelve `.` y `..`.
fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

impl Book {
    pub fn load(path: &str) -> Result<Book, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("no se pudo abrir: {}", e))?;
        if meta.len() > 400 * 1024 * 1024 {
            return Err("el archivo es demasiado grande".into());
        }
        let data = std::fs::read(path).map_err(|e| format!("no se pudo leer: {}", e))?;
        let zip = Zip::open(data)?;

        let container = String::from_utf8_lossy(&zip.read("META-INF/container.xml")?).to_string();
        let opf_path = elements(&container)
            .iter()
            .find(|e| e.name == "rootfile")
            .and_then(|e| e.attr("full-path").map(|s| s.to_string()))
            .ok_or("container.xml sin rootfile")?;
        let opf = String::from_utf8_lossy(&zip.read(&opf_path)?).to_string();
        let base = match opf_path.rfind('/') {
            Some(p) => opf_path[..p].to_string(),
            None => String::new(),
        };

        let els = elements(&opf);
        let mut title = String::new();
        let mut author = String::new();
        let mut manifest: HashMap<String, (String, String)> = HashMap::new();
        let mut spine: Vec<String> = Vec::new();
        for e in &els {
            match e.name.as_str() {
                "title" if title.is_empty() => title = e.text.clone(),
                "creator" if author.is_empty() => author = e.text.clone(),
                "item" => {
                    if let (Some(id), Some(href)) = (e.attr("id"), e.attr("href")) {
                        manifest.insert(id.to_string(), (href.to_string(), e.attr("media-type").unwrap_or("").to_string()));
                    }
                }
                "itemref" => {
                    if let Some(r) = e.attr("idref") {
                        spine.push(r.to_string());
                    }
                }
                _ => {}
            }
        }
        if spine.is_empty() {
            return Err("el EPUB no tiene contenido (spine vacío)".into());
        }

        let mut chapters: Vec<Chapter> = Vec::new();
        for idref in &spine {
            let Some((href, mt)) = manifest.get(idref) else { continue };
            if !mt.is_empty() && !mt.contains("html") && !mt.contains("xml") {
                continue;
            }
            let Ok(bytes) = zip.read(&resolve(&base, href)) else { continue };
            let paras = html_to_paras(&String::from_utf8_lossy(&bytes));
            let chars: usize = paras.iter().map(|p| p.text.chars().count()).sum();
            if chars == 0 {
                continue;
            }
            let n = chapters.len() + 1;
            let title = match paras.iter().find(|p| p.heading) {
                Some(p) if p.text.chars().count() <= 60 => p.text.clone(),
                _ => format!("Sección {}", n),
            };
            chapters.push(Chapter { title, paras, chars });
        }
        if chapters.is_empty() {
            return Err("no se encontró texto legible (¿EPUB con DRM o solo imágenes?)".into());
        }
        let mut before = Vec::with_capacity(chapters.len());
        let mut acc = 0;
        for c in &chapters {
            before.push(acc);
            acc += c.chars;
        }
        if title.is_empty() {
            title = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Libro".to_string());
        }
        Ok(Book { path: path.to_string(), title, author, chapters, before, total_chars: acc.max(1) })
    }
}

// ------------------------------------------------------------ paginación

pub struct Line {
    pub para: usize,
    pub off: usize, // byte donde empieza la línea dentro del párrafo
    pub text: String,
    pub heading: bool,
}

const INDENT: &str = "    ";

fn wrap_para(pi: usize, p: &Para, width: i32, measure: &mut dyn FnMut(&str, bool) -> i32) -> Vec<Line> {
    let h = p.heading;
    let mut lines: Vec<Line> = Vec::new();
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut off = 0;
    for w in p.text.split(' ') {
        if !w.is_empty() {
            words.push((off, w));
        }
        off += w.len() + 1;
    }
    let mut cur = String::from(if h { "" } else { INDENT });
    let mut start = 0usize;
    let mut has = false;
    let mut i = 0;
    while i < words.len() {
        let (ws, word) = words[i];
        if has {
            let cand = format!("{} {}", cur, word);
            if measure(&cand, h) <= width {
                cur = cand;
                i += 1;
            } else {
                lines.push(Line { para: pi, off: start, text: std::mem::take(&mut cur), heading: h });
                has = false;
            }
            continue;
        }
        start = ws;
        let cand = format!("{}{}", cur, word);
        if measure(&cand, h) <= width {
            cur = cand;
            has = true;
            i += 1;
            continue;
        }
        // palabra más ancha que la línea: se parte por caracteres
        let mut first = true;
        for (ci, ch) in word.char_indices() {
            let mut t = cur.clone();
            t.push(ch);
            if !first && measure(&t, h) > width {
                lines.push(Line { para: pi, off: start, text: std::mem::take(&mut cur), heading: h });
                start = ws + ci;
            }
            cur.push(ch);
            first = false;
        }
        has = true;
        i += 1;
    }
    if has {
        lines.push(Line { para: pi, off: start, text: cur, heading: h });
    }
    if h {
        lines.push(Line { para: pi, off: p.text.len(), text: String::new(), heading: false });
    }
    lines
}

/// Reparte los párrafos en páginas de `per_page` líneas.
/// `measure(texto, es_titulo)` devuelve el ancho en píxeles.
pub fn paginate(
    paras: &[Para],
    width: i32,
    per_page: usize,
    measure: &mut dyn FnMut(&str, bool) -> i32,
) -> Vec<Vec<Line>> {
    let mut pages: Vec<Vec<Line>> = Vec::new();
    let mut cur: Vec<Line> = Vec::new();
    for (pi, p) in paras.iter().enumerate() {
        for l in wrap_para(pi, p, width, measure) {
            if l.text.is_empty() && cur.is_empty() {
                continue; // sin líneas en blanco al inicio de página
            }
            cur.push(l);
            if cur.len() == per_page {
                pages.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        pages.push(cur);
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    pages
}

/// Última página cuyo inicio es <= `pos` (párrafo, byte).
pub fn page_for(pages: &[Vec<Line>], pos: (usize, usize)) -> usize {
    let mut idx = 0;
    for (i, p) in pages.iter().enumerate() {
        if let Some(l) = p.first() {
            if (l.para, l.off) <= pos {
                idx = i;
            } else {
                break;
            }
        }
    }
    idx
}
