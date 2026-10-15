//! Whitelist do servidor (`whitelist.json` + `white-list`/`enforce-whitelist`).
//!
//! A pegadinha: o `whitelist add` do Minecraft resolve o UUID pelo nick na
//! Mojang se o nick existir lá, mas um servidor em `online-mode=false` usa o
//! UUID OFFLINE (md5 de `OfflinePlayer:<nick>`, versão 3). Com o UUID errado o
//! jogador pirata cai em "not whitelisted". Por isso o painel grava o arquivo
//! direto, escolhendo o UUID conforme o `online-mode` do servidor.

use crate::ctx::{self, Srv};
use crate::json::{self, Map, Value};
use crate::jsutil;
use crate::net;
use crate::obj;
use crate::rcon;
use crate::runner;

/// MD5 (RFC 1321): só serve pro UUID offline, não é uso criptográfico.
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11,
        16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn fmt_uuid(b: &[u8; 16]) -> String {
    let h: String = b.iter().map(|x| format!("{:02x}", x)).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `UUID.nameUUIDFromBytes("OfflinePlayer:" + nick)` do Java
pub fn offline_uuid(name: &str) -> String {
    let mut h = md5(format!("OfflinePlayer:{}", name).as_bytes());
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    fmt_uuid(&h)
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 16 && n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
}

fn file(s: &Srv) -> String {
    jsutil::path_join(&[&s.dir, "whitelist.json"])
}

fn read_list(s: &Srv) -> Vec<(String, String)> {
    let Ok(t) = std::fs::read_to_string(file(s)) else { return Vec::new() };
    let Ok(Value::Arr(a)) = json::parse(&t) else { return Vec::new() };
    a.iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_string();
            let uuid = e.get("uuid").and_then(|u| u.as_str()).unwrap_or("").to_string();
            Some((name, uuid))
        })
        .collect()
}

fn write_list(s: &Srv, list: &[(String, String)]) -> Result<(), String> {
    let arr: Vec<Value> = list.iter().map(|(n, u)| obj! { "uuid" => u.clone(), "name" => n.clone() }).collect();
    let f = file(s);
    let tmp = format!("{}.tmp", f);
    std::fs::write(&tmp, json::stringify_pretty(&Value::Arr(arr))).map_err(|e| jsutil::fs_err(&e, "open", &tmp))?;
    std::fs::rename(&tmp, &f).map_err(|e| jsutil::fs_err(&e, "rename", &f))
}

fn online_mode(s: &Srv) -> bool {
    ctx::read_props(&s.dir).get("online-mode").and_then(|v| v.as_str()) != Some("false")
}

/// UUID da conta original (Mojang), no formato com traços
fn mojang_uuid(name: &str) -> Result<String, String> {
    let v = net::get_json(&format!("https://api.mojang.com/users/profiles/minecraft/{}", net::enc(name))).map_err(|e| {
        if e.status == Some(404) || e.status == Some(204) {
            format!("o nick \"{}\" não existe como conta original na Mojang — o servidor está em modo online, então só contas originais entram", name)
        } else {
            format!("não consegui consultar a Mojang: {}", e.msg)
        }
    })?;
    let id = v.get("id").and_then(|x| x.as_str()).filter(|x| x.len() == 32).ok_or("resposta inesperada da Mojang")?;
    Ok(format!("{}-{}-{}-{}-{}", &id[0..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32]))
}

fn reload_if_running(exec: bool, cfg: &Map, s: &Srv, cmd: &str) {
    if runner::svc_active_of(exec, cfg, &s.service) == "active" {
        let _ = rcon::command(cfg, s, cmd);
    }
}

pub fn status(s: &Srv) -> Value {
    let props = ctx::read_props(&s.dir);
    let on = |k: &str| props.get(k).and_then(|v| v.as_str()) == Some("true");
    let online = online_mode(s);
    let players: Vec<Value> = read_list(s)
        .into_iter()
        .map(|(n, u)| {
            // UUID que NÃO bate com o modo atual = o jogador não vai conseguir entrar
            let expected_offline = offline_uuid(&n);
            let wrong = !online && !u.eq_ignore_ascii_case(&expected_offline);
            obj! { "name" => n, "uuid" => u, "offlineMismatch" => wrong, "expectedUuid" => expected_offline }
        })
        .collect();
    obj! { "enabled" => on("white-list"), "enforce" => on("enforce-whitelist"), "onlineMode" => online, "players" => players }
}

pub fn add(exec: bool, cfg: &Map, s: &Srv, name: &str) -> Result<Value, String> {
    let name = jsutil::trim(name);
    if !valid_name(name) {
        return Err("nick inválido (1 a 16 letras, números, _ ou .)".into());
    }
    let uuid = if online_mode(s) { mojang_uuid(name)? } else { offline_uuid(name) };
    let mut list = read_list(s);
    list.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    list.push((name.to_string(), uuid.clone()));
    write_list(s, &list)?;
    reload_if_running(exec, cfg, s, "whitelist reload");
    Ok(obj! { "ok" => true, "name" => name, "uuid" => uuid })
}

pub fn remove(exec: bool, cfg: &Map, s: &Srv, name: &str) -> Result<Value, String> {
    let mut list = read_list(s);
    let before = list.len();
    list.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    if list.len() == before {
        return Err("esse nick não está na whitelist".into());
    }
    write_list(s, &list)?;
    reload_if_running(exec, cfg, s, "whitelist reload");
    Ok(obj! { "ok" => true })
}

/// Troca os UUIDs que não batem com o modo offline atual (conserta quem foi
/// adicionado com `whitelist add` por RCON e ficou com o UUID da conta original)
pub fn fix_uuids(exec: bool, cfg: &Map, s: &Srv) -> Result<Value, String> {
    if online_mode(s) {
        return Err("o servidor está em modo online: os UUIDs da Mojang já são os certos".into());
    }
    let mut n = 0;
    let list: Vec<(String, String)> = read_list(s)
        .into_iter()
        .map(|(name, u)| {
            let right = offline_uuid(&name);
            if !u.eq_ignore_ascii_case(&right) {
                n += 1;
            }
            (name, right)
        })
        .collect();
    write_list(s, &list)?;
    reload_if_running(exec, cfg, s, "whitelist reload");
    Ok(obj! { "ok" => true, "fixed" => n })
}

pub fn set_enabled(exec: bool, cfg: &Map, s: &Srv, enabled: bool) -> Result<Value, String> {
    let v = if enabled { "true" } else { "false" };
    ctx::write_props_kv(&s.dir, &[("white-list", v), ("enforce-whitelist", v)])
        .map_err(|e| jsutil::fs_err(&e, "open", &jsutil::path_join(&[&s.dir, "server.properties"])))?;
    reload_if_running(exec, cfg, s, if enabled { "whitelist on" } else { "whitelist off" });
    Ok(obj! { "ok" => true, "enabled" => enabled })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_vectors() {
        let h = |s: &str| md5(s.as_bytes()).iter().map(|x| format!("{:02x}", x)).collect::<String>();
        assert_eq!(h(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(h("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h("12345678901234567890123456789012345678901234567890123456789012345678901234567890"), "57edf4a22be3c955ac49da2e2107b67a");
    }

    #[test]
    fn offline_uuids_do_servidor_real() {
        assert_eq!(offline_uuid("Vascon1"), "b28a9b50-f134-3545-a2be-61afb6fccc31");
        assert_eq!(offline_uuid("Hatori77"), "ac8c800b-c7c6-3067-a88b-71be01a45c02");
        assert_eq!(offline_uuid("LordKun"), "6955eb50-8403-3514-88ec-8cfef570bad5");
    }

    #[test]
    fn nicks() {
        assert!(valid_name("Vascon1") && valid_name("a_b.c"));
        assert!(!valid_name("") && !valid_name("nome muito longo demais") && !valid_name("a b"));
    }
}
