//! Carga/gravação do config.json com a mesma semântica do `loadConfig()` do
//! server.js: defaults + spread raso do arquivo (chaves desconhecidas são
//! preservadas e regravadas), merge de `rcon` e `auth`, overrides por env
//! (CRAFTBOX_SERVERS_DIR/INTEGRATIONS_DIR/RUN_DIR/PORT) e geração do
//! `sessionSecret` quando vazio (o que regrava o arquivo, já com os overrides).

use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::obj;

pub fn default_config() -> Map {
    match obj! {
        "port" => 8080,
        "host" => "0.0.0.0",
        "mcDir" => "/opt/minecraft",
        "service" => "minecraft",
        "rcon" => obj!{ "host" => "127.0.0.1", "port" => 25575, "password" => "" },
        "auth" => obj!{ "salt" => "", "hash" => "" },
        "users" => Vec::<Value>::new(),
        "sessionSecret" => "",
        "loader" => "",
        "mcVersion" => "",
        "serversDir" => "",
        "serviceTemplate" => "minecraft@",
        "activeServer" => "",
        "systemctlUser" => false,
        "integrationsDir" => "",
        "runDir" => "",
        "curseforgeApiKey" => "",
    } {
        Value::Obj(m) => m,
        _ => unreachable!(),
    }
}

/// `{ ...a, ...b }` (b só contribui se for objeto).
fn spread(a: &Map, b: Option<&Value>) -> Map {
    let mut out = a.clone();
    if let Some(Value::Obj(bm)) = b {
        for (k, v) in bm.iter() {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// Caminho do config: CRAFTBOX_PANEL_CONFIG ou `<dir do binário>/config.json`
/// (no Node era `<dir do server.js>/config.json`).
pub fn config_path(root: &str) -> String {
    match std::env::var("CRAFTBOX_PANEL_CONFIG") {
        Ok(p) if !p.is_empty() => p,
        _ => jsutil::path_join(&[root, "config.json"]),
    }
}

fn env_nonempty(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

pub fn load(path: &str) -> Map {
    let mut cfg = default_config();
    if let Ok(txt) = std::fs::read(path) {
        if let Ok(raw) = json::parse(&String::from_utf8_lossy(&txt)) {
            // JSON.parse('null') faz `raw.rcon` lançar TypeError: o catch mantém os defaults
            if !raw.is_null() {
                let d = default_config();
                let mut merged = spread(&cfg, Some(&raw));
                let rcon = spread(d.get("rcon").unwrap().as_obj().unwrap(), raw.get("rcon"));
                let auth = spread(d.get("auth").unwrap().as_obj().unwrap(), raw.get("auth"));
                merged.insert("rcon", Value::Obj(rcon));
                merged.insert("auth", Value::Obj(auth));
                cfg = merged;
            }
        }
    }
    if let Some(v) = env_nonempty("CRAFTBOX_SERVERS_DIR") {
        cfg.insert("serversDir", Value::Str(v));
    }
    if let Some(v) = env_nonempty("CRAFTBOX_INTEGRATIONS_DIR") {
        cfg.insert("integrationsDir", Value::Str(v));
    }
    if let Some(v) = env_nonempty("CRAFTBOX_RUN_DIR") {
        cfg.insert("runDir", Value::Str(v));
    }
    if let Some(v) = env_nonempty("CRAFTBOX_PORT") {
        if let Some(p) = jsutil::parse_int(&v) {
            if p > 0.0 {
                cfg.insert("port", Value::Num(p));
            }
        }
    }
    if !truthy(cfg.get("sessionSecret")) {
        cfg.insert("sessionSecret", Value::Str(crate::crypto::random_hex(32)));
        save(path, &cfg);
    }
    cfg
}

/// `fs.writeFileSync(CONFIG_PATH, JSON.stringify(cfg, null, 2))`, erros ignorados.
pub fn save(path: &str, cfg: &Map) {
    let _ = std::fs::write(path, json::stringify_pretty(&Value::Obj(cfg.clone())));
}

// ---------------------------------------------------------------------------
// Acessores com a semântica de uso no server.js
// ---------------------------------------------------------------------------

/// `CONFIG.x` usado como string com fallback em falsy (ex.: `CONFIG.runDir || ...`).
pub fn s(cfg: &Map, k: &str) -> String {
    jsutil::to_string_or_empty(cfg.get(k))
}

pub fn sub<'a>(cfg: &'a Map, k: &str, sk: &str) -> Option<&'a Value> {
    cfg.get(k).and_then(|v| v.get(sk))
}

/// Porta de escuta (número ou string numérica, como aceita o `server.listen`).
pub fn port(cfg: &Map) -> Option<u16> {
    let n = match cfg.get("port") {
        Some(Value::Num(n)) => *n,
        Some(Value::Str(s)) => jsutil::to_number(s)?,
        _ => return None,
    };
    if n.fract() == 0.0 && (0.0..=65535.0).contains(&n) {
        Some(n as u16)
    } else {
        None
    }
}

pub fn host(cfg: &Map) -> String {
    match cfg.get("host") {
        Some(Value::Str(h)) if !h.is_empty() => h.clone(),
        _ => "0.0.0.0".into(),
    }
}

pub fn systemctl_user(cfg: &Map) -> bool {
    truthy(cfg.get("systemctlUser"))
}

pub fn users(cfg: &Map) -> Vec<Value> {
    match cfg.get("users") {
        Some(Value::Arr(a)) => a.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("cbrs-cfg-{}-{}", std::process::id(), name));
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn merge_preserves_order_and_unknown_keys() {
        let p = tmp("merge");
        std::fs::write(&p, r#"{"extra":1,"rcon":{"password":"x"},"port":9000,"sessionSecret":"abc"}"#).unwrap();
        let c = load(&p);
        let keys: Vec<_> = c.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys.first(), Some(&"port"));
        assert_eq!(keys.last(), Some(&"extra"));
        assert_eq!(json::stringify(c.get("rcon").unwrap()), r#"{"host":"127.0.0.1","port":25575,"password":"x"}"#);
        assert_eq!(port(&c), Some(9000));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_secret_is_generated_and_saved() {
        let p = tmp("secret");
        std::fs::write(&p, r#"{"auth":{"salt":"a","hash":"b"}}"#).unwrap();
        let c = load(&p);
        let secret = s(&c, "sessionSecret");
        assert_eq!(secret.len(), 64);
        let saved = std::fs::read_to_string(&p).unwrap();
        assert!(saved.starts_with("{\n  \"port\": 8080,"));
        assert!(saved.contains(&secret));
        assert!(!saved.ends_with('\n'));
        let _ = std::fs::remove_file(&p);
    }
}
