//! Log de auditoria (JSON por linha), igual ao `audit()` do server.js:
//! `{ts, ip, user, server, action, detail}`, append, e rotação simples
//! (passou de 1 MiB → mantém as últimas 500 linhas). Erros são ignorados.

use crate::config;
use crate::json::{self, Map};
use crate::jsutil;
use crate::obj;
use std::io::Write;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

pub fn audit_path(cfg: &Map) -> String {
    if jsutil::truthy(cfg.get("auditLog")) {
        return jsutil::to_string(cfg.get("auditLog"));
    }
    let base = if jsutil::truthy(cfg.get("serversDir")) {
        jsutil::path_dirname(&config::s(cfg, "serversDir"))
    } else {
        jsutil::homedir()
    };
    jsutil::path_join(&[&base, "craftbox-audit.log"])
}

pub struct Who<'a> {
    pub ip: &'a str,
    pub user: &'a str,
    pub server: &'a str,
}

pub fn audit(cfg: &Map, who: &Who, action: &str, detail: &str) {
    let entry = obj! {
        "ts" => jsutil::now_ms(),
        "ip" => if who.ip.is_empty() { "-" } else { who.ip },
        "user" => if who.user.is_empty() { "-" } else { who.user },
        "server" => if who.server.is_empty() { "-" } else { who.server },
        "action" => action,
        "detail" => detail,
    };
    let p = audit_path(cfg);
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ok = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .and_then(|mut f| f.write_all(format!("{}\n", json::stringify(&entry)).as_bytes()));
    if ok.is_err() {
        return;
    }
    if let Ok(md) = std::fs::metadata(&p) {
        if md.len() > 1048576 {
            if let Ok(txt) = std::fs::read(&p) {
                let txt = String::from_utf8_lossy(&txt);
                let lines: Vec<&str> = jsutil::trim(&txt).split('\n').collect();
                let keep = &lines[lines.len().saturating_sub(500)..];
                let _ = std::fs::write(&p, format!("{}\n", keep.join("\n")));
            }
        }
    }
}
