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
            loader: Value::Null,
            mc_version: Value::Null,
            port: None,
            modpack: Value::Null,
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

#[cfg(test)]
mod tests {
    use super::*;

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
