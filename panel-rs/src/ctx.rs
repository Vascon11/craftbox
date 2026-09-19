//! Contexto de servidor por requisição (o `SRV()`/AsyncLocalStorage do
//! server.js): modo legado (1 servidor: mcDir + service) ou multi-servidor
//! (cada subpasta de serversDir é uma instância `serviceTemplate + id`).

use crate::config;
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};

#[derive(Clone, Debug)]
#[allow(dead_code)] // campos usados pelas próximas fatias (servidores, conteúdo, integrações)
pub struct Srv {
    pub id: String,
    pub dir: String,
    pub service: String,
    /// `meta.name || id` (pode não ser string se o meta for estranho)
    pub name: Value,
    pub legacy: bool,
    pub loader: Value,
    pub mc_version: Value,
    pub port: Option<Value>,
    pub modpack: Value,
    pub rcon_port: Option<Value>,
    pub rcon_password: Option<Value>,
}

impl Srv {
    #[allow(dead_code)]
    pub fn name_str(&self) -> String {
        jsutil::to_string(Some(&self.name))
    }
}

pub fn multi_enabled(cfg: &Map) -> bool {
    truthy(cfg.get("serversDir"))
}

pub fn instance_meta_path(dir: &str) -> String {
    jsutil::path_join(&[dir, ".craftbox-instance.json"])
}

/// `readInstanceMeta`: JSON do meta ou `{}`.
pub fn read_instance_meta(dir: &str) -> Map {
    match std::fs::read(instance_meta_path(dir)).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)).ok()) {
        Some(Value::Obj(m)) => m,
        _ => Map::new(),
    }
}

/// `listInstanceIds`: subpastas (não segue symlink, como o Dirent) em ordem.
pub fn list_instance_ids(cfg: &Map) -> Vec<String> {
    if !multi_enabled(cfg) {
        return vec!["default".into()];
    }
    let mut ids: Vec<String> = match std::fs::read_dir(config::s(cfg, "serversDir")) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    // Array.prototype.sort compara unidades UTF-16
    ids.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    ids
}

fn or_empty(v: Option<&Value>) -> Value {
    if truthy(v) {
        v.unwrap().clone()
    } else {
        Value::Str(String::new())
    }
}

pub fn server_ctx(cfg: &Map, id: &str) -> Srv {
    if !multi_enabled(cfg) {
        return Srv {
            id: "default".into(),
            dir: config::s(cfg, "mcDir"),
            service: config::s(cfg, "service"),
            name: Value::from("Servidor"),
            legacy: true,
            // no Node o contexto legado não tem essas chaves (undefined → somem do JSON)
            loader: Value::Undef,
            mc_version: Value::Undef,
            port: None,
            modpack: Value::Undef,
            rcon_port: None,
            rcon_password: None,
        };
    }
    let dir = jsutil::path_join(&[&config::s(cfg, "serversDir"), id]);
    let meta = read_instance_meta(&dir);
    Srv {
        id: id.into(),
        service: format!("{}{}", jsutil::to_string(cfg.get("serviceTemplate")), id),
        name: if truthy(meta.get("name")) { meta.get("name").unwrap().clone() } else { Value::from(id) },
        legacy: false,
        loader: or_empty(meta.get("loader")),
        mc_version: or_empty(meta.get("mcVersion")),
        port: meta.get("port").cloned(),
        modpack: if truthy(meta.get("modpack")) { meta.get("modpack").unwrap().clone() } else { Value::Null },
        rcon_port: meta.get("rconPort").cloned(),
        rcon_password: meta.get("rconPassword").cloned(),
        dir,
    }
}

/// `currentId(url)`: ?server= válido > activeServer válido > primeira > 'default'.
pub fn current_id(cfg: &Map, query_server: Option<&str>) -> String {
    if !multi_enabled(cfg) {
        return "default".into();
    }
    let ids = list_instance_ids(cfg);
    if let Some(q) = query_server {
        if !q.is_empty() && ids.iter().any(|i| i == q) {
            return q.into();
        }
    }
    let active = config::s(cfg, "activeServer");
    if !active.is_empty() && ids.contains(&active) {
        return active;
    }
    ids.into_iter().next().unwrap_or_else(|| "default".into())
}

/// `readProps()`: server.properties da instância (sem unescape de Java properties).
pub fn read_props(dir: &str) -> Map {
    let mut out = Map::new();
    if let Ok(b) = std::fs::read(jsutil::path_join(&[dir, "server.properties"])) {
        for line in String::from_utf8_lossy(&b).split('\n') {
            let t = jsutil::trim(line);
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if let Some(i) = t.find('=') {
                out.insert(&t[..i], Value::Str(t[i + 1..].to_string()));
            }
        }
    }
    out
}

/// `writeInstanceMeta(dir, meta)`: cria a pasta e grava com indentação 2; erros ignorados.
pub fn write_instance_meta(dir: &str, meta: &Map) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(instance_meta_path(dir), json::stringify_pretty(&Value::Obj(meta.clone())));
}

/// `writeProps(updates)` / `updatePropsFile(dir, updates)`: atualiza as chaves
/// existentes no lugar (preservando comentários e ordem), acrescenta as novas
/// no fim e grava sem `\n` final. O valor vira `String(v)`.
pub fn write_props(dir: &str, updates: &Map) -> std::io::Result<()> {
    let p = jsutil::path_join(&[dir, "server.properties"]);
    let text = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).ok();
    let mut lines: Vec<String> = match &text {
        Some(t) => t.split('\n').map(String::from).collect(),
        None => Vec::new(),
    };
    let mut seen: Vec<String> = Vec::new();
    for line in lines.iter_mut() {
        let t = jsutil::trim(line).to_string();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some(i) = t.find('=') else { continue };
        let key = &t[..i];
        if let Some(v) = updates.get(key) {
            seen.push(key.to_string());
            *line = format!("{}={}", key, jsutil::to_string(Some(v)));
        }
    }
    for (k, v) in updates.iter() {
        if !seen.iter().any(|s| s == k) {
            lines.push(format!("{}={}", k, jsutil::to_string(Some(v))));
        }
    }
    std::fs::write(&p, lines.join("\n"))
}

/// Atalho: `writeProps({k: v, ...})` com valores string.
pub fn write_props_kv(dir: &str, kv: &[(&str, &str)]) -> std::io::Result<()> {
    let mut m = Map::new();
    for (k, v) in kv {
        m.insert(*k, Value::from(*v));
    }
    write_props(dir, &m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_props_like_node() {
        let d = std::env::temp_dir().join(format!("cbrs-wprops-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("server.properties"), "#c\nmotd=a\n\nmax-players=5\n").unwrap();
        let mut u = Map::new();
        u.insert("max-players", Value::Num(10.0));
        u.insert("nova", Value::Bool(true));
        u.insert("2", Value::from("x")); // chave numérica: o V8 enumera primeiro
        write_props(d.to_str().unwrap(), &u).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("server.properties")).unwrap(),
            "#c\nmotd=a\n\nmax-players=10\n\n2=x\nnova=true"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn props_parse_like_node() {
        let d = std::env::temp_dir().join(format!("cbrs-props-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("server.properties"), "#c\r\nmotd=a=b\r\nkey = v \n\n  online-mode=false\nnovalue\n")
            .unwrap();
        let p = read_props(d.to_str().unwrap());
        assert_eq!(p.get("motd"), Some(&Value::from("a=b")));
        assert_eq!(p.get("key "), Some(&Value::from(" v")));
        assert_eq!(p.get("online-mode"), Some(&Value::from("false")));
        assert_eq!(p.len(), 3);
        let _ = std::fs::remove_dir_all(&d);
    }
}
