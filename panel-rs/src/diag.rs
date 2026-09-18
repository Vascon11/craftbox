//! `GET /api/diag/mojang`: o host alcança os servidores de login da Mojang?
//! Mesma semântica do server.js: qualquer resposta HTTP (inclusive 3xx/4xx/5xx)
//! prova alcance; redirect não é seguido; 6 s de limite total por alvo; os dois
//! alvos em paralelo; `onlineMode` vem do server.properties da instância.

use crate::ctx::read_props;
use crate::json::Value;
use crate::jsutil::now_ms;
use crate::obj;
use std::time::Duration;

pub const UA: &str = "craftbox-panel/1.0 (github.com/Vascon11/craftbox)";

pub const MOJANG_TARGETS: [(&str, &str); 2] = [
    // esperado 204
    ("sessionserver", "https://sessionserver.mojang.com/session/minecraft/hasJoined?username=craftbox&serverId=diag"),
    ("minecraftservices", "https://api.minecraftservices.com/"),
];

/// Resultado do probe no formato do Node:
/// ok → `{ok:true, status, ms}`; falha → `{ok:false, status:null, ms, error, code}`.
pub fn probe_url(url: &str, timeout_ms: u64) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout_ms)))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(UA)
        .build()
        .into();
    let start = now_ms();
    let r = agent.get(url).call();
    let ms = now_ms() - start;
    match r {
        // o corpo não é lido: dropar a resposta equivale ao `r.body.cancel()`
        Ok(resp) => obj! { "ok" => true, "status" => resp.status().as_u16() as u32, "ms" => ms },
        Err(e) => {
            let (code, msg) = node_style_error(&e, url, timeout_ms);
            obj! {
                "ok" => false, "status" => Value::Null, "ms" => ms,
                "error" => format!("{}: {}", code, msg), "code" => code,
            }
        }
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

/// Aproxima o `code`/mensagem que o undici (fetch do Node) daria. O front só
/// exibe `error`; o `code` serve pra diagnóstico. Os casos comuns (DNS, recusa,
/// timeout, reset, rede inalcançável) usam os mesmos códigos do Node.
fn node_style_error(e: &ureq::Error, url: &str, timeout_ms: u64) -> (String, String) {
    let host = host_of(url);
    let hostname = host.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or(host.clone());
    let with_port = if host.contains(':') {
        host.clone()
    } else if url.starts_with("https") {
        format!("{}:443", host)
    } else {
        format!("{}:80", host)
    };
    match e {
        ureq::Error::Timeout(_) => {
            ("ETIMEDOUT".into(), format!("sem resposta em {}s", crate::json::num_to_string(timeout_ms as f64 / 1000.0)))
        }
        ureq::Error::HostNotFound => ("ENOTFOUND".into(), format!("getaddrinfo ENOTFOUND {}", hostname)),
        ureq::Error::Io(io) => {
            use std::io::ErrorKind as K;
            let s = io.to_string();
            if s.contains("Temporary failure in name resolution") {
                return ("EAI_AGAIN".into(), format!("getaddrinfo EAI_AGAIN {}", hostname));
            }
            if s.contains("failed to lookup address") || s.contains("Name or service not known") {
                return ("ENOTFOUND".into(), format!("getaddrinfo ENOTFOUND {}", hostname));
            }
            let code = match io.kind() {
                K::ConnectionRefused => "ECONNREFUSED",
                K::ConnectionReset => "ECONNRESET",
                K::ConnectionAborted => "ECONNABORTED",
                K::TimedOut => "ETIMEDOUT",
                K::NetworkUnreachable => "ENETUNREACH",
                K::HostUnreachable => "EHOSTUNREACH",
                K::BrokenPipe => "EPIPE",
                K::UnexpectedEof => "UND_ERR_SOCKET",
                _ => "EIO",
            };
            if code == "ETIMEDOUT" {
                return (
                    code.into(),
                    format!("sem resposta em {}s", crate::json::num_to_string(timeout_ms as f64 / 1000.0)),
                );
            }
            (code.into(), format!("connect {} {}", code, with_port))
        }
        ureq::Error::ConnectionFailed => ("ECONNREFUSED".into(), format!("connect ECONNREFUSED {}", with_port)),
        ureq::Error::Tls(m) => ("ERR_TLS".into(), m.to_string()),
        ureq::Error::Rustls(r) => ("ERR_TLS".into(), r.to_string()),
        other => ("Error".into(), other.to_string()),
    }
}

pub fn diag_mojang(srv_dir: &str) -> Value {
    let handles: Vec<_> = MOJANG_TARGETS
        .iter()
        .map(|(name, url)| {
            let (name, url) = (name.to_string(), url.to_string());
            std::thread::spawn(move || {
                let mut o = obj! { "name" => name, "url" => url.clone() };
                if let (Value::Obj(m), Value::Obj(p)) = (&mut o, probe_url(&url, 6000)) {
                    for (k, v) in p.iter() {
                        m.insert(k.clone(), v.clone());
                    }
                }
                o
            })
        })
        .collect();
    let results: Vec<Value> = handles.into_iter().map(|h| h.join().unwrap_or(Value::Null)).collect();
    let props = read_props(srv_dir);
    let online_mode = match props.get("online-mode") {
        Some(Value::Str(s)) => Value::Bool(s != "false"),
        _ => Value::Null,
    };
    let all_ok = results.iter().all(|r| r.get("ok") == Some(&Value::Bool(true)));
    obj! { "ok" => all_ok, "onlineMode" => online_mode, "targets" => results }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn any_http_status_is_ok_and_redirect_not_followed() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 1024];
            let _ = s.read(&mut b);
            let _ = s.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n");
        });
        let r = probe_url(&format!("http://127.0.0.1:{}/", port), 6000);
        assert_eq!(r.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(r.get("status"), Some(&Value::Num(302.0)));
    }

    #[test]
    fn refused_and_timeout_shapes() {
        // porta fechada
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let r = probe_url(&format!("http://127.0.0.1:{}/", port), 6000);
        assert_eq!(r.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(r.get("status"), Some(&Value::Null));
        assert_eq!(r.get("code"), Some(&Value::from("ECONNREFUSED")));
        let keys: Vec<_> = r.as_obj().unwrap().iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, ["ok", "status", "ms", "error", "code"]);
        // aceita e não responde → timeout
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let _keep = std::thread::spawn(move || {
            let _c = l.accept();
            std::thread::sleep(Duration::from_secs(3));
        });
        let r = probe_url(&format!("http://127.0.0.1:{}/", port), 500);
        assert_eq!(r.get("code"), Some(&Value::from("ETIMEDOUT")));
        assert_eq!(r.get("error"), Some(&Value::from("ETIMEDOUT: sem resposta em 0.5s")));
    }
}
