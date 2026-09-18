//! Sessão e contas, compatíveis com o server.js a ponto de um cookie emitido
//! pelo Node valer no Rust e vice-versa (mesmo `sessionSecret`):
//!
//!   cookie  `cbsession=<valor>.<mac>; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`
//!   valor   `s:<expira em ms>:<encodeURIComponent(usuário)>`
//!   mac     HMAC-SHA256(sessionSecret, valor) em base64url
//!
//! Replica também a leitura de cookie do Node (split em ';', decodeURIComponent
//! no valor), inclusive o bug documentado no ROUTES.md: usuários com caracteres
//! que o encodeURIComponent escapa (ex. "@", "+", acentos) não conseguem manter a
//! sessão, porque o valor é decodificado antes da verificação do MAC.

use crate::config;
use crate::crypto;
use crate::json::{Map, Value};
use crate::jsutil::{self, now_ms};

pub const SESSION_MS: f64 = 12.0 * 3600.0 * 1000.0;

pub fn sign(secret: &str, value: &str) -> String {
    format!("{}.{}", value, crypto::hmac_b64url(secret, value))
}

/// `verify(token)`: devolve o valor assinado se o MAC confere e não expirou.
pub fn verify(secret: &str, token: &str) -> Option<String> {
    let i = token.rfind('.')?;
    let (value, mac) = (&token[..i], &token[i + 1..]);
    let expect = crypto::hmac_b64url(secret, value);
    if !crypto::ct_eq(mac.as_bytes(), expect.as_bytes()) {
        return None;
    }
    let exp = jsutil::parse_int(value.split(':').nth(1).unwrap_or(""))?;
    if exp == 0.0 || now_ms() > exp {
        return None;
    }
    Some(value.to_string())
}

pub fn make_session(secret: &str, user: &str) -> String {
    let u = if user.is_empty() { "admin" } else { user };
    sign(
        secret,
        &format!("s:{}:{}", crate::json::num_to_string(now_ms() + SESSION_MS), jsutil::encode_uri_component(u)),
    )
}

pub fn session_cookie(secret: &str, user: &str) -> String {
    format!("cbsession={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200", make_session(secret, user))
}

pub const LOGOUT_COOKIE: &str = "cbsession=; HttpOnly; Path=/; Max-Age=0";

/// `parseCookies(req)`. Divergência deliberada: no Node um valor com '%'
/// malformado lança URIError FORA do try (derruba o processo); aqui o cookie é ignorado.
pub fn parse_cookies(header: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for c in header.split(';') {
        let Some(i) = c.find('=') else { continue };
        let k = jsutil::trim(&c[..i]).to_string();
        let Ok(v) = jsutil::decode_uri_component(jsutil::trim(&c[i + 1..])) else { continue };
        if let Some(slot) = out.iter_mut().find(|(kk, _)| *kk == k) {
            slot.1 = v;
        } else {
            out.push((k, v));
        }
    }
    out
}

fn session_value(cfg: &Map, cookie_header: &str) -> Option<String> {
    let cookies = parse_cookies(cookie_header);
    let tok = cookies.iter().find(|(k, _)| k == "cbsession").map(|(_, v)| v.as_str())?;
    if tok.is_empty() {
        return None;
    }
    verify(&config::s(cfg, "sessionSecret"), tok)
}

pub fn is_authed(cfg: &Map, cookie_header: &str) -> bool {
    session_value(cfg, cookie_header).is_some()
}

/// `sessionUser(req)`: usuário da sessão válida, ou None.
pub fn session_user(cfg: &Map, cookie_header: &str) -> Option<String> {
    let v = session_value(cfg, cookie_header)?;
    let raw = v.split(':').nth(2).filter(|s| !s.is_empty()).unwrap_or("admin");
    Some(jsutil::decode_uri_component(raw).unwrap_or_else(|_| raw.to_string()))
}

// ---- contas (legado = senha única; `users` com itens = multiusuário) ----

pub fn multi_user(cfg: &Map) -> bool {
    !config::users(cfg).is_empty()
}

/// `findUser(name)` (comparação case-insensitive).
pub fn find_user(cfg: &Map, name: Option<&Value>) -> Option<Map> {
    let n = jsutil::to_string_or_empty(name).to_lowercase();
    config::users(cfg).into_iter().find_map(|u| match u {
        Value::Obj(m) if jsutil::to_string_or_empty(m.get("user")).to_lowercase() == n => Some(m),
        _ => None,
    })
}

pub fn is_admin(cfg: &Map, name: Option<&str>) -> bool {
    if !multi_user(cfg) {
        return true;
    }
    let v = name.map(Value::from);
    matches!(find_user(cfg, v.as_ref()), Some(u) if u.get("role") == Some(&Value::from("admin")))
}

/// `checkPassword(pw, salt, hash)` com os tipos soltos do JS.
pub fn check_password_js(pw: Option<&Value>, salt: Option<&Value>, hash: Option<&Value>) -> bool {
    if !jsutil::truthy(salt) || !jsutil::truthy(hash) {
        return false;
    }
    let Some(Value::Str(hash)) = hash else { return false };
    crypto::check_password(&jsutil::to_string_or_empty(pw), &jsutil::to_string(salt), hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_token_roundtrip() {
        let secret = "abc";
        let tok = make_session(secret, "admin");
        assert!(tok.starts_with("s:"));
        assert!(verify(secret, &tok).is_some());
        assert!(verify("outro", &tok).is_none());
        let expired = sign(secret, "s:1000:admin");
        assert!(verify(secret, &expired).is_none());
        assert!(verify(secret, "semponto").is_none());
    }

    #[test]
    fn cookie_parsing_like_node() {
        let c = parse_cookies("a=1; cbsession=s%3A1%3Ax.mac ; bad=%E0; a=2; semigual");
        assert_eq!(c, vec![("a".into(), "2".into()), ("cbsession".into(), "s:1:x.mac".into())]);
    }

    #[test]
    fn user_with_at_sign_cannot_keep_session_like_node() {
        // o bug do server.js, reproduzido de propósito (ver ROUTES.md)
        let mut cfg = Map::new();
        cfg.insert("sessionSecret", Value::from("k"));
        let tok = make_session("k", "a@b");
        assert!(tok.contains("a%40b"));
        assert!(!is_authed(&cfg, &format!("cbsession={}", tok)));
        let tok = make_session("k", "joao");
        assert_eq!(session_user(&cfg, &format!("cbsession={}", tok)).as_deref(), Some("joao"));
    }
}
