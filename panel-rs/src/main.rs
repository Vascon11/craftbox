//! craftbox-panel (Rust) — backend do painel web, reescrita do panel/server.js
//! com as MESMAS rotas, pra o frontend estático (panel/public) continuar igual.
//!
//! Uso:
//!   craftbox-panel                 # sobe o painel
//!   craftbox-panel --hash SENHA    # gera o hash da senha p/ o config.json
//!   craftbox-panel --init          # cria um config.json inicial
//!   craftbox-panel --docker-init   # prepara o /data do container (o antigo docker/init.js)
//!   craftbox-panel --version
//!
//! Config: config.json ao lado do binário, ou CRAFTBOX_PANEL_CONFIG.
//! Frontend: CRAFTBOX_PUBLIC_DIR, ou `public/` ao lado do binário.

mod audit;
mod auth;
mod config;
mod content;
mod crypto;
mod ctx;
mod diag;
mod dockerinit;
mod http;
mod integrations;
mod json;
mod jsutil;
mod mcp;
mod modpacks;
mod net;
mod rcon;
mod routes;
mod runner;
mod servers;
mod stats;
mod zipw;

use json::{Map, Value};
use std::net::TcpListener;
use std::sync::{Arc, RwLock};

pub struct State {
    pub config_path: String,
    pub public_dir: String,
    pub exec_runner: bool,
    cfg: RwLock<Map>,
}

impl State {
    /// Cópia do config atual (é pequeno; evita segurar lock durante a requisição).
    pub fn cfg(&self) -> Map {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Altera o config em memória e grava o arquivo inteiro (`saveConfig()`)
    /// quando `f` devolve true. O lock serializa as gravações concorrentes.
    pub fn set_cfg(&self, f: impl FnOnce(&mut Map) -> bool) {
        let mut g = self.cfg.write().unwrap_or_else(|e| e.into_inner());
        if f(&mut g) {
            config::save(&self.config_path, &g);
        }
    }
}

fn exe_dir() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".into())
}

fn absolute(p: &str) -> String {
    if p.starts_with('/') {
        jsutil::path_normalize(p)
    } else {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
        jsutil::path_join(&[&cwd, p])
    }
}

fn public_dir(root: &str) -> String {
    if let Ok(p) = std::env::var("CRAFTBOX_PUBLIC_DIR") {
        if !p.is_empty() {
            return absolute(&p).trim_end_matches('/').to_string();
        }
    }
    let beside = jsutil::path_join(&[root, "public"]);
    // durante o desenvolvimento o binário fica em panel-rs/target/<perfil>/
    let dev = jsutil::path_join(&[root, "../../../panel/public"]);
    if !std::path::Path::new(&beside).is_dir() && std::path::Path::new(&dev).is_dir() {
        return dev;
    }
    beside
}

/// Bloqueia SIGTERM/SIGINT em todas as threads (herdado pelas que forem
/// criadas depois) e devolve a máscara pra thread que faz `sigwait`.
fn block_signals() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

/// Shutdown gracioso (igual ao server.js): no runner exec, para cada unidade
/// com PID file (SIGTERM → espera → SIGKILL) pra os mundos salvarem; no modo
/// systemd os serviços são independentes do painel e só saímos.
fn graceful_shutdown(st: &State) -> ! {
    if st.exec_runner {
        let cfg = st.cfg();
        for u in runner::units_with_pidfile(&cfg) {
            runner::unit_stop(&cfg, &u);
        }
    }
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = exe_dir();
    let config_path = config::config_path(&root);

    // ---- CLI ----
    if matches!(args.get(1).map(String::as_str), Some("--version" | "-V")) {
        println!("craftbox-panel {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--hash") {
        let Some(pw) = args.get(2) else {
            eprintln!("uso: craftbox-panel --hash SENHA");
            std::process::exit(1);
        };
        let (salt, hash) = crypto::hash_password(pw);
        println!("Cole isto no \"auth\" do config.json:");
        println!("{}", json::stringify_pretty(&obj! { "salt" => salt, "hash" => hash }));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--docker-init") {
        std::process::exit(dockerinit::run());
    }
    if args.get(1).map(String::as_str) == Some("--init") {
        if std::path::Path::new(&config_path).exists() {
            eprintln!("config.json ja existe.");
            std::process::exit(1);
        }
        let mut cfg = config::default_config();
        cfg.insert("sessionSecret", Value::Str(crypto::random_hex(32)));
        if let Err(e) = std::fs::write(&config_path, json::stringify_pretty(&Value::Obj(cfg))) {
            eprintln!("falha ao gravar {}: {}", config_path, e);
            std::process::exit(1);
        }
        println!("config.json criado em {} \nDefina a senha com: craftbox-panel --hash SUA_SENHA", config_path);
        return;
    }

    let sigset = block_signals();
    let cfg = config::load(&config_path);
    let exec_runner = runner::detect_exec_runner();
    let st = Arc::new(State { config_path, public_dir: public_dir(&root), exec_runner, cfg: RwLock::new(cfg.clone()) });

    let host = config::host(&cfg);
    let Some(port) = config::port(&cfg) else {
        eprintln!("porta inválida no config: {}", json::stringify(cfg.get("port").unwrap_or(&Value::Null)));
        std::process::exit(1);
    };
    let listener = match TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("erro ao escutar em {}:{}: {}", host, port, e);
            std::process::exit(1);
        }
    };

    // thread de sinais
    {
        let st = st.clone();
        std::thread::spawn(move || loop {
            let mut sig: libc::c_int = 0;
            if unsafe { libc::sigwait(&sigset, &mut sig) } == 0 && (sig == libc::SIGTERM || sig == libc::SIGINT) {
                graceful_shutdown(&st);
            }
        });
    }

    println!("craftbox-panel ouvindo em http://{}:{}", host, port);
    if !jsutil::truthy(config::sub(&cfg, "auth", "salt")) || !jsutil::truthy(config::sub(&cfg, "auth", "hash")) {
        println!("AVISO: senha nao configurada. Rode:  craftbox-panel --hash SUA_SENHA  e cole no config.json");
    }
    if st.exec_runner {
        println!("[runner] modo container ativo (gerenciando processos em {})", runner::run_dir(&cfg));
        let st2 = st.clone();
        std::thread::spawn(move || {
            routes::autostart_servers(&st2);
            println!("[runner] auto-start de servidores concluído");
        });
    }

    let auth_state = st.clone();
    http::set_upload_auth(move |cookie| auth::is_authed(&auth_state.cfg(), cookie));
    let handler_state = st.clone();
    http::serve(listener, Arc::new(move |req: &http::Request| routes::handle(&handler_state, req)));
}
