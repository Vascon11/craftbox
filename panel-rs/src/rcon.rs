//! Cliente RCON (protocolo Source sobre TCP), com a mesma máquina de estados do
//! server.js: pacote AUTH (id 1), o primeiro pacote recebido é tratado como
//! resposta do auth (id -1 = senha errada), depois EXEC (id 2) e o corpo do
//! próximo pacote é a resposta. Timeout de inatividade de 6 s.
//!
//! Divergência deliberada: se o servidor fechar a conexão sem responder, o Node
//! fica com a Promise pendurada pra sempre (não trata 'end'/'close'); aqui vira erro.

use crate::config;
use crate::ctx::{read_props, Srv};
use crate::json::Map;
use crate::jsutil::{self, truthy};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(6);

/// `rconPassword()`: server.properties > meta da instância (pumpkin) > config.
pub fn password(cfg: &Map, srv: &Srv) -> String {
    let props = read_props(&srv.dir);
    if truthy(props.get("rcon.password")) {
        return jsutil::to_string(props.get("rcon.password"));
    }
    if truthy(srv.rcon_password.as_ref()) {
        return jsutil::to_string(srv.rcon_password.as_ref());
    }
    jsutil::to_string_or_empty(config::sub(cfg, "rcon", "password"))
}

/// `rconPort()`: server.properties > meta (pumpkin) > config > 25575.
pub fn port(cfg: &Map, srv: &Srv) -> Option<f64> {
    let props = read_props(&srv.dir);
    if let Some(p) = props.get("rcon.port").and_then(|v| v.as_str()).and_then(jsutil::parse_int) {
        if p != 0.0 {
            return Some(p);
        }
    }
    if truthy(srv.rcon_port.as_ref()) {
        return match srv.rcon_port.as_ref().unwrap() {
            crate::json::Value::Num(n) => Some(*n),
            crate::json::Value::Str(s) => jsutil::to_number(s),
            _ => None,
        };
    }
    match config::sub(cfg, "rcon", "port") {
        Some(v) if truthy(Some(v)) => match v {
            crate::json::Value::Num(n) => Some(*n),
            crate::json::Value::Str(s) => jsutil::to_number(s),
            _ => None,
        },
        _ => Some(25575.0),
    }
}

fn pkt(id: i32, typ: i32, body: &str) -> Vec<u8> {
    let mut b = body.as_bytes().to_vec();
    b.extend_from_slice(&[0, 0]);
    let len = (4 + 4 + b.len()) as i32;
    let mut out = Vec::with_capacity(4 + len as usize);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&typ.to_le_bytes());
    out.extend_from_slice(&b);
    out
}

pub fn command(cfg: &Map, srv: &Srv, cmd: &str) -> Result<String, String> {
    command_timeout(cfg, srv, cmd, TIMEOUT)
}

/// `command` com timeout próprio (a sonda de status usa um curto: servidor
/// travado não pode segurar o /api/status por 6 s).
pub fn command_timeout(cfg: &Map, srv: &Srv, cmd: &str, timeout: Duration) -> Result<String, String> {
    let pw = password(cfg, srv);
    if pw.is_empty() {
        return Err("RCON sem senha (habilite enable-rcon no server.properties)".into());
    }
    let port = port(cfg, srv).ok_or("porta RCON inválida")?;
    if port.fract() != 0.0 || !(0.0..65536.0).contains(&port) {
        return Err(format!("porta RCON inválida: {}", port));
    }
    let host = jsutil::to_string(config::sub(cfg, "rcon", "host"));
    let addr = (host.as_str(), port as u16)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("host RCON não resolvido")?;
    let mut sock = TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            "RCON timeout".to_string()
        } else {
            e.to_string()
        }
    })?;
    sock.set_read_timeout(Some(timeout)).ok();
    sock.set_write_timeout(Some(timeout)).ok();
    sock.write_all(&pkt(1, 3, &pw)).map_err(|e| e.to_string())?;
    let mut buf: Vec<u8> = Vec::new();
    let mut authed = false;
    let mut chunk = [0u8; 4096];
    loop {
        while buf.len() >= 4 {
            let len = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            if !(10..=1 << 20).contains(&len) {
                return Err("pacote RCON inválido".into());
            }
            let len = len as usize;
            if buf.len() < 4 + len {
                break;
            }
            let id = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let body = String::from_utf8_lossy(&buf[12..4 + len - 2]).into_owned();
            buf.drain(..4 + len);
            if !authed {
                if id == -1 {
                    return Err("senha RCON incorreta".into());
                }
                authed = true;
                sock.write_all(&pkt(2, 2, cmd)).map_err(|e| e.to_string())?;
            } else {
                return Ok(body);
            }
        }
        match sock.read(&mut chunk) {
            Ok(0) => return Err("RCON: conexão fechada sem resposta".into()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                return Err("RCON timeout".into())
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `list.match(/(\d+)\s+of a max of\s+(\d+)/i) || list.match(/(\d+)\/(\d+)/)`
pub fn parse_players(list: &str) -> Option<(f64, f64)> {
    let c: Vec<char> = list.chars().collect();
    let digits_at = |i: usize| -> usize { c[i..].iter().take_while(|x| x.is_ascii_digit()).count() };
    let ws_at = |i: usize| -> usize { c[i..].iter().take_while(|x| jsutil::is_js_ws(**x)).count() };
    let num = |a: usize, n: usize| -> f64 { c[a..a + n].iter().collect::<String>().parse().unwrap_or(f64::NAN) };
    let lit: Vec<char> = "of a max of".chars().collect();
    // 1ª regex (case-insensitive no literal)
    for i in 0..c.len() {
        let d1 = digits_at(i);
        if d1 == 0 {
            continue;
        }
        let mut j = i + d1;
        let w1 = ws_at(j);
        if w1 == 0 {
            continue;
        }
        j += w1;
        if c.len() < j + lit.len() || !c[j..j + lit.len()].iter().zip(&lit).all(|(a, b)| a.to_ascii_lowercase() == *b) {
            continue;
        }
        j += lit.len();
        let w2 = ws_at(j);
        if w2 == 0 {
            continue;
        }
        j += w2;
        let d2 = digits_at(j);
        if d2 == 0 {
            continue;
        }
        return Some((num(i, d1), num(j, d2)));
    }
    // 2ª regex
    for i in 0..c.len() {
        let d1 = digits_at(i);
        if d1 == 0 || c.get(i + d1) != Some(&'/') {
            continue;
        }
        let d2 = digits_at(i + d1 + 1);
        if d2 == 0 {
            continue;
        }
        return Some((num(i, d1), num(i + d1 + 1, d2)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_regex() {
        assert_eq!(parse_players("There are 3 of a max of 20 players online: a, b, c"), Some((3.0, 20.0)));
        assert_eq!(parse_players("There are 0 OF A MAX OF 10 players"), Some((0.0, 10.0)));
        assert_eq!(parse_players("There are 1/8 players online:"), Some((1.0, 8.0)));
        assert_eq!(parse_players("12 of a max 5"), None);
        assert_eq!(parse_players("nada"), None);
    }

    #[test]
    fn packet_layout() {
        assert_eq!(pkt(1, 3, "pw"), vec![12, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, b'p', b'w', 0, 0]);
    }
}
