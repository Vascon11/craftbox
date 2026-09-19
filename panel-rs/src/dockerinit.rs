//! `craftbox-panel --docker-init`: o `docker/init.js` do container, pra imagem
//! não precisar de Node. Garante o /data/config.json (cria ou atualiza os
//! campos de caminho), aplica a senha inicial (CRAFTBOX_PASSWORD) só se ainda
//! não houver senha, e semeia os binários de túnel no integrationsDir.

use crate::crypto;
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::obj;

fn env_or(k: &str, def: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| def.to_string())
}

pub fn run() -> i32 {
    let cfg_path = env_or("CRAFTBOX_PANEL_CONFIG", "/data/config.json");
    let servers_dir = env_or("CRAFTBOX_SERVERS_DIR", "/data/servers");
    let intg_dir = env_or("CRAFTBOX_INTEGRATIONS_DIR", "/data/craftbox-integrations");
    let run_dir = env_or("CRAFTBOX_RUN_DIR", "/data/craftbox-run");
    let port = jsutil::parse_int(&env_or("CRAFTBOX_PORT", "8080")).unwrap_or(f64::NAN);
    let password = std::env::var("CRAFTBOX_PASSWORD").unwrap_or_default();
    let tunnel_seed = "/usr/local/lib/craftbox/tunnels";

    for d in [&servers_dir, &intg_dir, &run_dir, &jsutil::path_dirname(&cfg_path)] {
        let _ = std::fs::create_dir_all(d);
    }

    let parsed = std::fs::read(&cfg_path).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)).ok());
    let existed = parsed.is_some();
    let mut cfg = match parsed {
        Some(Value::Obj(m)) => m,
        _ => Map::new(),
    };

    cfg.insert("port", Value::Num(port));
    cfg.insert("host", Value::from("0.0.0.0"));
    cfg.insert("serversDir", Value::from(servers_dir.as_str()));
    cfg.insert("integrationsDir", Value::from(intg_dir.as_str()));
    cfg.insert("runDir", Value::from(run_dir.as_str()));
    if !truthy(cfg.get("serviceTemplate")) {
        cfg.insert("serviceTemplate", Value::from("minecraft@"));
    }
    if !truthy(cfg.get("sessionSecret")) {
        cfg.insert("sessionSecret", Value::from(crypto::random_hex(32)));
    }
    if !truthy(cfg.get("auth")) {
        cfg.insert("auth", obj! { "salt" => "", "hash" => "" });
    }
    // senha inicial: só aplica se ainda não houver senha configurada
    if !truthy(cfg.get("auth").and_then(|a| a.get("hash"))) && !password.is_empty() {
        let (salt, hash) = crypto::hash_password(&password);
        cfg.insert("auth", obj! { "salt" => salt, "hash" => hash });
    }

    if let Err(e) = std::fs::write(&cfg_path, format!("{}\n", json::stringify_pretty(&Value::Obj(cfg.clone())))) {
        eprintln!("[init] ERRO ao gravar {} {}", cfg_path, e);
        return 1;
    }

    // semeia playit/cloudflared no integrationsDir (o painel os detecta ali)
    if let Ok(rd) = std::fs::read_dir(tunnel_seed) {
        use std::os::unix::fs::PermissionsExt;
        for e in rd.flatten() {
            let target = jsutil::path_join(&[&intg_dir, &e.file_name().to_string_lossy()]);
            if !std::path::Path::new(&target).exists() {
                let _ = std::fs::copy(e.path(), &target);
            }
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        }
    }

    println!("[init] config.json: {} {}", cfg_path, if existed { "(atualizado)" } else { "(criado)" });
    println!("[init] servidores: {} | integrações: {} | runner: exec", servers_dir, intg_dir);
    if truthy(cfg.get("auth").and_then(|a| a.get("hash"))) {
        println!("[init] senha do painel configurada");
    } else {
        println!("[init] ATENÇÃO: defina CRAFTBOX_PASSWORD na primeira subida p/ criar a senha");
    }
    0
}
