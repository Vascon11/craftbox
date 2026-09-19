//! Chamadas HTTPS de saída (Modrinth, Paper, Fabric, NeoForge, CurseForge,
//! GitHub, GeyserMC, Cloudflare), no lugar do `httpsJson`/`download`/`httpsReq`
//! do server.js:
//! - redirects são seguidos; status != 200 vira erro `"HTTP <code>"`;
//! - `download` grava em streaming direto no arquivo (modpacks têm centenas de MB);
//! - o Node não tinha timeout no `httpsJson` e tinha 60 s de ociosidade no
//!   `download`. O ureq não tem timeout de ociosidade, então: 60 s pra conectar e
//!   pra receber os cabeçalhos, e sem limite total no corpo (download grande em
//!   link lento não pode abortar no meio).

use crate::diag::UA;
use crate::json::{self, Value};
use std::io::{Read, Write};
use std::time::Duration;

const CONNECT: Duration = Duration::from_secs(60);
/// JSON de API é pequeno; um teto evita requisição pendurada pra sempre
const JSON_TOTAL: Duration = Duration::from_secs(120);
const MAX_JSON: u64 = 64 * 1024 * 1024;

fn agent(total: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT))
        .timeout_recv_response(Some(CONNECT))
        .timeout_global(total)
        .http_status_as_error(false)
        .user_agent(UA)
        .build()
        .into()
}

/// Erro de requisição: `status` quando o servidor respondeu != 200.
#[derive(Debug)]
pub struct HttpError {
    pub status: Option<u16>,
    pub msg: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl From<HttpError> for String {
    fn from(e: HttpError) -> String {
        e.msg
    }
}

fn transport(e: ureq::Error) -> HttpError {
    HttpError { status: None, msg: e.to_string() }
}

fn status_err(code: u16) -> HttpError {
    HttpError { status: Some(code), msg: format!("HTTP {}", code) }
}

fn read_json(mut resp: ureq::http::Response<ureq::Body>) -> Result<Value, HttpError> {
    let txt = resp
        .body_mut()
        .with_config()
        .limit(MAX_JSON)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(transport)?;
    json::parse(&txt).map_err(|e| HttpError { status: None, msg: e.0 })
}

/// `httpsJson(url)`: GET com `Accept: application/json`.
pub fn get_json(url: &str) -> Result<Value, HttpError> {
    let resp = agent(Some(JSON_TOTAL)).get(url).header("Accept", "application/json").call().map_err(transport)?;
    if resp.status().as_u16() != 200 {
        return Err(status_err(resp.status().as_u16()));
    }
    read_json(resp)
}

/// `httpsReq(method, url, {headers, body})` do CurseForge/Modrinth: JSON na ida e na volta.
pub fn req_json(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Result<Value, HttpError> {
    let a = agent(Some(Duration::from_secs(30)));
    let resp = match (method, body) {
        ("POST", Some(b)) => {
            let mut r = a.post(url).header("Accept", "application/json").header("Content-Type", "application/json");
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.send(json::stringify(b)).map_err(transport)?
        }
        _ => {
            let mut r = a.get(url).header("Accept", "application/json");
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.call().map_err(transport)?
        }
    };
    if resp.status().as_u16() != 200 {
        return Err(status_err(resp.status().as_u16()));
    }
    read_json(resp)
}

/// `download(url, dest)`: o arquivo é criado antes da requisição (como o
/// `createWriteStream` do Node) e removido se a transferência quebrar no meio.
pub fn download(url: &str, dest: &str) -> Result<(), String> {
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let resp = match agent(None).get(url).call() {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            return Err(e.to_string());
        }
    };
    if resp.status().as_u16() != 200 {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let mut body = resp.into_body();
    let mut rd = body.as_reader();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match rd.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = f.write_all(&buf[..n]) {
                    let _ = std::fs::remove_file(dest);
                    return Err(e.to_string());
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(dest);
                return Err(e.to_string());
            }
        }
    }
    f.flush().map_err(|e| e.to_string())
}

/// Mede a velocidade de download (`speedTest`): lê e descarta, 30 s no máximo.
pub fn count_download(url: &str, timeout: Duration) -> Result<u64, String> {
    let resp = agent(Some(timeout)).get(url).call().map_err(|e| {
        if matches!(e, ureq::Error::Timeout(_)) {
            "tempo esgotado".to_string()
        } else {
            e.to_string()
        }
    })?;
    if resp.status().as_u16() != 200 {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let mut body = resp.into_body();
    let mut rd = body.as_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = 0u64;
    loop {
        match rd.read(&mut buf) {
            Ok(0) => return Ok(n),
            Ok(k) => n += k as u64,
            Err(e) => {
                return Err(if e.kind() == std::io::ErrorKind::TimedOut { "tempo esgotado".into() } else { e.to_string() })
            }
        }
    }
}

/// `encodeURIComponent` para montar URLs.
pub fn enc(s: &str) -> String {
    crate::jsutil::encode_uri_component(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(resp: &'static [u8]) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 2048];
            let _ = s.read(&mut b);
            let _ = s.write_all(resp);
        });
        format!("http://127.0.0.1:{}/x", port)
    }

    #[test]
    fn json_and_status() {
        let u = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"a\":[1,2]}");
        assert_eq!(json::stringify(&get_json(&u).unwrap()), "{\"a\":[1,2]}");
        let u = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        let e = get_json(&u).unwrap_err();
        assert_eq!((e.status, e.msg.as_str()), (Some(404), "HTTP 404"));
    }

    #[test]
    fn download_writes_file() {
        let u = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        let p = std::env::temp_dir().join(format!("cbrs-dl-{}", std::process::id()));
        download(&u, p.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        let _ = std::fs::remove_file(&p);
    }
}
