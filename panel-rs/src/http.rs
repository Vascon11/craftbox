//! Servidor HTTP/1.1 mínimo sobre std::net, uma thread por conexão.
//!
//! Por que não hyper/axum/tokio: o painel atende 1–3 abas fazendo polling a
//! cada 3 s, e algumas rotas (instalar modpack, backup) ficam minutos
//! bloqueadas em disco/rede/processo filho — thread por conexão modela isso
//! direto, sem runtime async. Zero dependências = compilação rápida no Haswell e
//! controle total dos headers pra ficar igual ao `http` do Node:
//! keep-alive com `Keep-Alive: timeout=5`, `Date`, 100-continue, HEAD sem corpo,
//! 400 + close em requisição malformada, limite de 16 KiB de cabeçalho (431).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADER: usize = 16 * 1024;
/// o readBody do Node destrói a conexão depois de ~1e6 caracteres
pub const MAX_BODY: usize = 1_000_000;
/// Upload de mod bloqueado (corpo cru): os .jar passam fácil de 1 MB.
pub const MAX_UPLOAD: usize = 256 * 1024 * 1024;
/// rotas que recebem arquivo cru (mods) e por isso podem passar de MAX_BODY
pub const UPLOAD_PATHS: &[&str] = &["/api/modpacks/manual-upload", "/api/content/upload"];

/// Quem pode mandar corpo acima de MAX_BODY: o main registra a checagem de
/// sessão (recebe o cabeçalho Cookie) pra ninguém sem login encher a RAM.
type UploadAuth = Box<dyn Fn(&str) -> bool + Send + Sync>;
static UPLOAD_AUTH: std::sync::OnceLock<UploadAuth> = std::sync::OnceLock::new();

pub fn set_upload_auth(f: impl Fn(&str) -> bool + Send + Sync + 'static) {
    let _ = UPLOAD_AUTH.set(Box::new(f));
}
const MAX_CONNS: usize = 256;
const KEEPALIVE: Duration = Duration::from_secs(5);
const HEADERS_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Request {
    pub method: String,
    #[allow(dead_code)] // alvo cru, útil pra log/diagnóstico
    pub target: String,
    /// pathname já normalizado como o `new URL(req.url, 'http://localhost')`
    pub path: String,
    /// query crua (sem '?')
    pub query: String,
    /// nomes em minúsculas, na ordem recebida
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// IP remoto sem o prefixo `::ffff:`
    pub ip: String,
}

impl Request {
    /// Header como o `req.headers[name]` do Node: `cookie` repetido junta com "; ",
    /// os demais com ", ".
    pub fn header(&self, name: &str) -> Option<String> {
        let vals: Vec<&str> = self.headers.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str()).collect();
        if vals.is_empty() {
            return None;
        }
        Some(vals.join(if name == "cookie" { "; " } else { ", " }))
    }
    /// `url.searchParams.get(name)`
    pub fn query_get(&self, name: &str) -> Option<String> {
        for pair in self.query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if form_decode(k) == name {
                return Some(form_decode(v));
            }
        }
        None
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// corpo lido de um arquivo em streaming (o `createReadStream(f).pipe(res)` do Node)
    pub stream: Option<std::fs::File>,
}

impl Response {
    #[allow(dead_code)]
    pub fn new(status: u16) -> Self {
        Response { status, headers: Vec::new(), body: Vec::new(), stream: None }
    }
    /// `res.setHeader` antes do `writeHead`: o Node emite esses primeiro.
    pub fn header_first(mut self, k: &str, v: &str) -> Self {
        self.headers.insert(0, (k.into(), v.into()));
        self
    }
    /// `res.writeHead(code); res.end(text)` (sem Content-Type, como no Node)
    pub fn plain(status: u16, text: &str) -> Self {
        Response { status, headers: Vec::new(), body: text.as_bytes().to_vec(), stream: None }
    }
}

/// application/x-www-form-urlencoded (URLSearchParams): '+' = espaço,
/// %XX inválido fica literal, UTF-8 inválido vira U+FFFD.
pub fn form_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_single_dot(s: &str) -> bool {
    s == "." || s.eq_ignore_ascii_case("%2e")
}
fn is_double_dot(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), ".." | ".%2e" | "%2e." | "%2e%2e")
}

/// Pathname + query como o WHATWG URL parser (base `http://localhost`) produz:
/// forma absoluta e `//authority` perdem o host, `\` vira `/`, segmentos `.`/`..`
/// (inclusive `%2e`) são resolvidos e bytes fora do conjunto permitido são
/// percent-encoded. Nada é decodificado (o Node também não decodifica o pathname).
pub fn parse_target(target: &str) -> (String, String) {
    let t: String = target.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    let t = t.trim_matches(|c: char| c <= ' ');
    let (before_frag, _) = t.split_once('#').unwrap_or((t, ""));
    let (raw_path, query) = before_frag.split_once('?').unwrap_or((before_frag, ""));
    let mut p = raw_path.replace('\\', "/");
    let lower = p.to_ascii_lowercase();
    let after_scheme = ["http:", "https:"].iter().find_map(|s| lower.starts_with(s).then_some(s.len()));
    if let Some(n) = after_scheme {
        p = p[n..].to_string();
        if !p.starts_with("//") {
            p = format!("/{}", p.trim_start_matches('/'));
        }
    }
    if let Some(rest) = p.strip_prefix("//") {
        // authority até a próxima barra
        p = match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".into(),
        };
    }
    if !p.starts_with('/') {
        p = format!("/{}", p);
    }
    let segs: Vec<&str> = p[1..].split('/').collect();
    let mut out: Vec<String> = Vec::new();
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        let last = i == n - 1;
        if is_double_dot(seg) {
            out.pop();
            if last {
                out.push(String::new());
            }
        } else if is_single_dot(seg) {
            if last {
                out.push(String::new());
            }
        } else {
            out.push(pct_encode_path(seg));
        }
    }
    (format!("/{}", out.join("/")), query.to_string())
}

fn pct_encode_path(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b <= 0x20 || b >= 0x7f || matches!(b, b'"' | b'#' | b'<' | b'>' | b'?' | b'`' | b'{' | b'}') {
            o.push_str(&format!("%{:02X}", b));
        } else {
            o.push(b as char);
        }
    }
    o
}

pub fn reason(code: u16) -> &'static str {
    match code {
        100 => "Continue",
        200 => "OK",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Data no formato IMF-fixdate (header `Date`).
pub fn http_date() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::gmtime_r(&t, &mut tm) };
    const D: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        D[tm.tm_wday as usize % 7],
        tm.tm_mday,
        M[tm.tm_mon as usize % 12],
        tm.tm_year + 1900,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

enum ReadErr {
    Closed,
    Bad(u16),
    TooLarge,
}

fn read_head(r: &mut BufReader<TcpStream>) -> Result<Vec<String>, ReadErr> {
    let mut lines = Vec::new();
    let mut total = 0usize;
    loop {
        let mut line = Vec::new();
        let n = r.by_ref().take((MAX_HEADER + 2) as u64).read_until(b'\n', &mut line).map_err(|_| ReadErr::Closed)?;
        if n == 0 {
            return Err(if lines.is_empty() { ReadErr::Closed } else { ReadErr::Bad(400) });
        }
        total += n;
        if total > MAX_HEADER {
            return Err(ReadErr::Bad(431));
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.is_empty() {
            if lines.is_empty() {
                continue; // CRLF solto antes da request line é tolerado
            }
            return Ok(lines);
        }
        lines.push(String::from_utf8_lossy(&line).into_owned());
    }
}

fn read_chunked(r: &mut BufReader<TcpStream>, max: usize) -> Result<Vec<u8>, ReadErr> {
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        r.read_line(&mut line).map_err(|_| ReadErr::Closed)?;
        let size_s = line.trim().split(';').next().unwrap_or("");
        let size = usize::from_str_radix(size_s, 16).map_err(|_| ReadErr::Bad(400))?;
        if size == 0 {
            // trailers até a linha vazia
            loop {
                let mut l = String::new();
                if r.read_line(&mut l).map_err(|_| ReadErr::Closed)? == 0 || l.trim().is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len() + size > max {
            return Err(ReadErr::TooLarge);
        }
        let start = body.len();
        body.resize(start + size, 0);
        r.read_exact(&mut body[start..]).map_err(|_| ReadErr::Closed)?;
        let mut crlf = [0u8; 2];
        r.read_exact(&mut crlf).map_err(|_| ReadErr::Closed)?;
    }
}

/// Mesmo enquadramento do Node: com Content-Length definido pelo handler (as
/// respostas JSON) o corpo vai direto; sem ele (`writeHead` + `end(data)`, caso
/// dos estáticos e do 404 em texto) o Node usa `Transfer-Encoding: chunked`
/// em HTTP/1.1 e fecha a conexão em HTTP/1.0. HEAD não leva corpo nem framing.
fn write_response(
    s: &mut TcpStream,
    resp: &mut Response,
    head_only: bool,
    keep_alive: bool,
    http10: bool,
) -> std::io::Result<()> {
    let mut h = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
    let has = |n: &str| resp.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    for (k, v) in &resp.headers {
        h.push_str(&format!("{}: {}\r\n", k, v));
    }
    h.push_str(&format!("Date: {}\r\n", http_date()));
    if keep_alive {
        h.push_str("Connection: keep-alive\r\nKeep-Alive: timeout=5\r\n");
    } else {
        h.push_str("Connection: close\r\n");
    }
    let chunked = !has("content-length") && !head_only && !http10;
    if chunked {
        h.push_str("Transfer-Encoding: chunked\r\n");
    }
    h.push_str("\r\n");
    let mut out = h.into_bytes();
    if let (Some(f), false) = (resp.stream.as_mut(), head_only) {
        s.write_all(&out)?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if chunked {
                s.write_all(format!("{:x}\r\n", n).as_bytes())?;
                s.write_all(&buf[..n])?;
                s.write_all(b"\r\n")?;
            } else {
                s.write_all(&buf[..n])?;
            }
        }
        if chunked {
            s.write_all(b"0\r\n\r\n")?;
        }
        return s.flush();
    }
    if !head_only {
        if chunked {
            if !resp.body.is_empty() {
                out.extend_from_slice(format!("{:x}\r\n", resp.body.len()).as_bytes());
                out.extend_from_slice(&resp.body);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"0\r\n\r\n");
        } else {
            out.extend_from_slice(&resp.body);
        }
    }
    s.write_all(&out)?;
    s.flush()
}

fn simple_error(s: &mut TcpStream, code: u16) {
    let _ = s.write_all(format!("HTTP/1.1 {} {}\r\nConnection: close\r\n\r\n", code, reason(code)).as_bytes());
}

pub type Handler = dyn Fn(&Request) -> Response + Send + Sync;

fn handle_conn(stream: TcpStream, handler: Arc<Handler>) {
    let peer = stream
        .peer_addr()
        .map(|a| a.ip().to_string().trim_start_matches("::ffff:").to_string())
        .unwrap_or_else(|_| "-".into());
    let mut w = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut r = BufReader::new(stream);
    let mut first = true;
    loop {
        let _ = r.get_ref().set_read_timeout(Some(if first { HEADERS_TIMEOUT } else { KEEPALIVE }));
        first = false;
        let lines = match read_head(&mut r) {
            Ok(l) => l,
            Err(ReadErr::Closed) => return,
            Err(ReadErr::Bad(c)) => return simple_error(&mut w, c),
            Err(ReadErr::TooLarge) => return,
        };
        let _ = r.get_ref().set_read_timeout(Some(HEADERS_TIMEOUT));
        let mut rl = lines[0].split(' ');
        let (method, target, version) = match (rl.next(), rl.next(), rl.next(), rl.next()) {
            (Some(m), Some(t), Some(v), None)
                if !m.is_empty()
                    && m.bytes().all(|b| b.is_ascii_uppercase() || b == b'-')
                    && v.starts_with("HTTP/1.") =>
            {
                (m.to_string(), t.to_string(), v.to_string())
            }
            _ => return simple_error(&mut w, 400),
        };
        let mut headers = Vec::new();
        for l in &lines[1..] {
            let Some((k, v)) = l.split_once(':') else { return simple_error(&mut w, 400) };
            if k.is_empty() || k.contains(' ') {
                return simple_error(&mut w, 400);
            }
            headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
        }
        let hget = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.to_ascii_lowercase());
        let conn = hget("connection").unwrap_or_default();
        let http10 = version == "HTTP/1.0";
        // sem Content-Length o Node em HTTP/1.0 só consegue delimitar o corpo fechando a conexão
        let keep_alive = if http10 { false } else { !conn.contains("close") };
        if hget("expect").is_some_and(|e| e == "100-continue") {
            let _ = w.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        let big_ok = target.split('?').next().is_some_and(|p| UPLOAD_PATHS.contains(&p)) && {
            let ck = headers.iter().find(|(k, _)| k == "cookie").map(|(_, v)| v.as_str()).unwrap_or("");
            UPLOAD_AUTH.get().is_some_and(|f| f(ck))
        };
        let max = if big_ok { MAX_UPLOAD } else { MAX_BODY };
        let body = if hget("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
            match read_chunked(&mut r, max) {
                Ok(b) => b,
                Err(ReadErr::Bad(c)) => return simple_error(&mut w, c),
                Err(_) => return,
            }
        } else if let Some(cl) = hget("content-length") {
            let Ok(n) = cl.trim().parse::<usize>() else { return simple_error(&mut w, 400) };
            if n > max {
                return; // Node: req.destroy() → conexão cai sem resposta
            }
            let mut b = vec![0u8; n];
            if r.read_exact(&mut b).is_err() {
                return;
            }
            b
        } else {
            Vec::new()
        };
        let (path, query) = parse_target(&target);
        let req = Request { method, target, path, query, headers, body, ip: peer.clone() };
        let mut resp = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&req))) {
            Ok(r) => r,
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "erro interno".into());
                crate::routes::json(500, &crate::obj! { "error" => msg })
            }
        };
        if write_response(&mut w, &mut resp, req.method == "HEAD", keep_alive, http10).is_err() || !keep_alive {
            return;
        }
    }
}

pub fn serve(listener: TcpListener, handler: Arc<Handler>) {
    let active = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        if active.load(Ordering::SeqCst) >= MAX_CONNS {
            drop(stream);
            continue;
        }
        let _ = stream.set_nodelay(true);
        let h = handler.clone();
        let a = active.clone();
        a.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new().name("conn".into()).spawn(move || {
            handle_conn(stream, h);
            a.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whatwg_like_paths() {
        assert_eq!(parse_target("/api/status?server=a").0, "/api/status");
        assert_eq!(parse_target("/api/status?server=a").1, "server=a");
        assert_eq!(parse_target("/public/../server.js").0, "/server.js");
        assert_eq!(parse_target("/public/%2e%2E/x").0, "/x");
        assert_eq!(parse_target("/public/./a/.").0, "/public/a/");
        assert_eq!(parse_target("/a/..").0, "/");
        assert_eq!(parse_target("/logos\\..\\app.js").0, "/app.js");
        assert_eq!(parse_target("http://evil:1/app.js?x").0, "/app.js");
        assert_eq!(parse_target("//evil/app.js").0, "/app.js");
        assert_eq!(parse_target("/public//app.js").0, "/public//app.js");
        assert_eq!(parse_target("/a%20b/ç").0, "/a%20b/%C3%A7");
        assert_eq!(parse_target("*").0, "/*");
    }

    #[test]
    fn search_params() {
        let r = Request {
            method: "GET".into(),
            target: String::new(),
            path: "/".into(),
            query: "a=1&server=meu+srv%21&server=2&x=%zz".into(),
            headers: vec![],
            body: vec![],
            ip: String::new(),
        };
        assert_eq!(r.query_get("server").as_deref(), Some("meu srv!"));
        assert_eq!(r.query_get("x").as_deref(), Some("%zz"));
        assert_eq!(r.query_get("nada"), None);
    }
}
