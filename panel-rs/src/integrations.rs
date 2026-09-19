//! Integrações (playit / tailscale / cloudflare) e rede (nmcli), como no
//! server.js. Os túneis rodam como serviços de usuário do systemd (ou pelo
//! runner exec no container), um por servidor: o túnel liga EXATAMENTE a
//! instância selecionada.

use crate::config;
use crate::ctx::{self, Srv};
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::net;
use crate::obj;
use crate::runner::{self, RunResult, RUN_TIMEOUT};
use std::time::Duration;

/// `integrationsDir()` (criada)
pub fn integrations_dir(cfg: &Map) -> String {
    let base = if truthy(cfg.get("integrationsDir")) {
        config::s(cfg, "integrationsDir")
    } else if ctx::multi_enabled(cfg) {
        jsutil::path_join(&[&jsutil::path_dirname(&config::s(cfg, "serversDir")), "craftbox-integrations"])
    } else {
        jsutil::path_join(&[&jsutil::homedir(), ".craftbox-integrations"])
    };
    let _ = std::fs::create_dir_all(&base);
    base
}

fn write_user_unit(unit: &str, desc: &str, exec: &str) {
    let content = format!(
        "[Unit]\nDescription={}\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        desc, exec
    );
    let _ = std::fs::write(jsutil::path_join(&[&runner::user_unit_dir(), &format!("{}.service", unit)]), content);
}

fn u_svc(exec: bool, cfg: &Map, action: &str, unit: &str) -> RunResult {
    if exec {
        return runner::unit_action(cfg, action, unit);
    }
    runner::run("systemctl", &["--user", action, unit], RUN_TIMEOUT)
}

fn u_active(exec: bool, cfg: &Map, unit: &str) -> String {
    if exec {
        return runner::unit_active(cfg, unit).into();
    }
    let r = runner::run("systemctl", &["--user", "is-active", unit], RUN_TIMEOUT);
    let t = jsutil::trim(&r.stdout);
    if t.is_empty() {
        "unknown".into()
    } else {
        t.into()
    }
}

pub fn u_journal(exec: bool, cfg: &Map, unit: &str, lines: usize) -> String {
    if exec {
        return runner::unit_journal(cfg, unit, lines);
    }
    let n = if lines == 0 { 120 } else { lines }.to_string();
    runner::run("journalctl", &["--user", "-u", unit, "-n", &n, "--no-pager", "-o", "cat"], RUN_TIMEOUT).stdout
}

/// `intgSrvDir()`: diretório de runtime da integração por servidor
pub fn intg_srv_dir(cfg: &Map, s: &Srv) -> String {
    let base = integrations_dir(cfg);
    if !ctx::multi_enabled(cfg) {
        return base;
    }
    let d = jsutil::path_join(&[&base, &s.id]);
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn intg_unit(cfg: &Map, s: &Srv, name: &str) -> Option<String> {
    let base = match name {
        "playit" => "craftbox-playit",
        "cloudflare" => "craftbox-cloudflared",
        _ => return None,
    };
    Some(if ctx::multi_enabled(cfg) { format!("{}-{}", base, s.id) } else { base.into() })
}

fn exists(p: &str) -> bool {
    std::path::Path::new(p).exists()
}

fn playit_exec(bin: &str, toml: &str, dir: &str) -> String {
    format!("{} --secret-path {} --socket-path {}", bin, toml, jsutil::path_join(&[dir, "playit.sock"]))
}

/// liga os túneis junto com o servidor (se a instância tiver onStart)
pub fn maybe_start_tunnels(exec: bool, cfg: &Map, s: &Srv) {
    let meta = ctx::read_instance_meta(&s.dir);
    if !ctx::multi_enabled(cfg) {
        return;
    }
    let dir = intg_srv_dir(cfg, s);
    let idir = integrations_dir(cfg);
    let name = s.name_str();
    if truthy(meta.get("playitOnStart")) {
        let bin = jsutil::path_join(&[&idir, "playit"]);
        let toml = jsutil::path_join(&[&dir, "playit.toml"]);
        if exists(&bin) && exists(&toml) {
            if let Some(u) = intg_unit(cfg, s, "playit") {
                if u_active(exec, cfg, &u) != "active" {
                    write_user_unit(&u, &format!("craftbox — playit.gg agent ({})", name), &playit_exec(&bin, &toml, &dir));
                    runner::daemon_reload(exec);
                    u_svc(exec, cfg, "start", &u);
                }
            }
        }
    }
    if truthy(meta.get("cfOnStart")) {
        let bin = jsutil::path_join(&[&idir, "cloudflared"]);
        let tok = jsutil::path_join(&[&dir, "cloudflared.token"]);
        if exists(&bin) && exists(&tok) {
            if let Some(u) = intg_unit(cfg, s, "cloudflare") {
                if u_active(exec, cfg, &u) != "active" {
                    if let Ok(t) = std::fs::read_to_string(&tok) {
                        write_user_unit(
                            &u,
                            &format!("craftbox — Cloudflare Tunnel ({})", name),
                            &format!("{} tunnel --no-autoupdate run --token {}", bin, jsutil::trim(&t)),
                        );
                        runner::daemon_reload(exec);
                        u_svc(exec, cfg, "start", &u);
                    }
                }
            }
        }
    }
}

fn arch_is_arm() -> bool {
    std::env::consts::ARCH == "aarch64"
}

/// controla o tailscale: tenta direto (usuário operador) e cai pra 'sudo -n' (sudoers do appliance)
fn ts_ctl(args: &[&str]) -> RunResult {
    let t = Duration::from_secs(20);
    let r = runner::run("tailscale", args, t);
    if r.code == 0 {
        return r;
    }
    let mut a = vec!["-n", "tailscale"];
    a.extend_from_slice(args);
    let s = runner::run("sudo", &a, t);
    if s.code == 0 || format!("{}{}", s.stdout, s.stderr).contains("login.tailscale.com") {
        return s;
    }
    r // mantém o erro original (com a dica de operador)
}

fn ts_perm_msg(out: &str) -> Option<String> {
    let l = out.to_lowercase();
    ["operator", "access denied", "prefs write", "a password is required", "sudo:"].iter().any(|k| l.contains(k)).then(|| {
        "Sem permissão pra controlar o Tailscale por aqui. No craftbox instalado isso já vem liberado; neste desktop, rode UMA vez: sudo tailscale set --operator=$USER".to_string()
    })
}

/// `/[a-z0-9-]+\.(?:craft\.)?playit\.gg(?::\d+)?|\d+\.tcp\.playit\.gg(?::\d+)?/i`
fn playit_address(log: &str) -> Option<String> {
    let lower = log.to_ascii_lowercase();
    let b = lower.as_bytes();
    let is_host = |c: u8| c.is_ascii_alphanumeric() || c == b'-';
    let mut best: Option<(usize, String)> = None;
    for pat in [".playit.gg", ".craft.playit.gg", ".tcp.playit.gg"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(pat) {
            let at = from + i;
            let mut st = at;
            while st > 0 && is_host(b[st - 1]) {
                st -= 1;
            }
            if st < at {
                let label = &lower[st..at];
                let ok = pat != ".tcp.playit.gg" || label.bytes().all(|c| c.is_ascii_digit());
                if ok {
                    let mut end = at + pat.len();
                    if b.get(end) == Some(&b':') && b.get(end + 1).is_some_and(|c| c.is_ascii_digit()) {
                        end += 1;
                        while b.get(end).is_some_and(|c| c.is_ascii_digit()) {
                            end += 1;
                        }
                    }
                    if best.as_ref().map_or(true, |(p, _)| st < *p) {
                        best = Some((st, log[st..end].to_string()));
                    }
                }
            }
            from = at + 1;
        }
    }
    best.map(|x| x.1)
}

pub fn integrations_status(exec: bool, cfg: &Map, s: &Srv) -> Value {
    let dir = integrations_dir(cfg);
    let srv_dir = intg_srv_dir(cfg, s);
    let meta = ctx::read_instance_meta(&s.dir);
    let playit_bin = jsutil::path_join(&[&dir, "playit"]);
    let cf_bin = jsutil::path_join(&[&dir, "cloudflared"]);
    // playit (daemon roda com um secret key criado no playit.gg — por servidor)
    let p_inst = exists(&playit_bin);
    let mut p_run = false;
    let mut p_addr = Value::Null;
    if let Some(u) = intg_unit(cfg, s, "playit").filter(|_| p_inst) {
        if u_active(exec, cfg, &u) == "active" {
            p_run = true;
            if let Some(a) = playit_address(&u_journal(exec, cfg, &u, 150)) {
                p_addr = Value::from(a);
            }
        }
    }
    let playit = obj! {
        "installed" => p_inst, "hasSecret" => exists(&jsutil::path_join(&[&srv_dir, "playit.toml"])),
        "running" => p_run, "address" => p_addr, "onStart" => truthy(meta.get("playitOnStart")),
    };
    // cloudflared
    let c_inst = exists(&cf_bin);
    let host = std::fs::read_to_string(jsutil::path_join(&[&srv_dir, "cloudflared.host"]))
        .map(|t| jsutil::trim(&t).to_string())
        .unwrap_or_default();
    let c_run = intg_unit(cfg, s, "cloudflare").filter(|_| c_inst).is_some_and(|u| u_active(exec, cfg, &u) == "active");
    let cloudflare = obj! {
        "installed" => c_inst, "running" => c_run,
        "hasToken" => exists(&jsutil::path_join(&[&srv_dir, "cloudflared.token"])), "hostname" => host,
        "onStart" => truthy(meta.get("cfOnStart")),
    };
    // tailscale (binário do sistema)
    let tv = runner::run("tailscale", &["version"], RUN_TIMEOUT);
    let (mut t_run, mut t_ip, mut t_state) = (false, Value::Null, String::new());
    if tv.code == 0 {
        let st = runner::run("tailscale", &["status", "--json"], RUN_TIMEOUT);
        if st.code == 0 {
            if let Ok(j) = json::parse(&st.stdout) {
                t_state = jsutil::to_string_or_empty(j.get("BackendState"));
                t_run = t_state == "Running";
                if let Some(ips) = j.get("Self").and_then(|s| s.get("TailscaleIPs")).and_then(|i| i.as_arr()).filter(|a| !a.is_empty()) {
                    t_ip = ips.iter().find(|x| x.as_str().is_some_and(|s| s.contains('.'))).unwrap_or(&ips[0]).clone();
                }
            }
        }
    }
    let tailscale = obj! { "installed" => tv.code == 0, "running" => t_run, "ip" => t_ip, "state" => t_state };
    obj! {
        "dir" => dir, "systemctlUser" => cfg.get("systemctlUser").cloned().unwrap_or(Value::Undef),
        "server" => obj!{ "id" => s.id.clone(), "name" => s.name.clone(), "port" => s.port.clone().unwrap_or(Value::Undef) },
        "playit" => playit, "cloudflare" => cloudflare, "tailscale" => tailscale,
    }
}

fn chmod(p: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

pub fn integration_install(cfg: &Map, name: &str) -> Result<Value, String> {
    let dir = integrations_dir(cfg);
    match name {
        "playit" => {
            let bin = jsutil::path_join(&[&dir, "playit"]);
            let a = if arch_is_arm() { "aarch64" } else { "amd64" };
            net::download(&format!("https://github.com/playit-cloud/playit-agent/releases/latest/download/playit-linux-{}", a), &bin)?;
            chmod(&bin, 0o755);
            Ok(obj! { "ok" => true })
        }
        "cloudflare" => {
            let bin = jsutil::path_join(&[&dir, "cloudflared"]);
            let a = if arch_is_arm() { "arm64" } else { "amd64" };
            net::download(&format!("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-{}", a), &bin)?;
            chmod(&bin, 0o755);
            Ok(obj! { "ok" => true })
        }
        "tailscale" => {
            if runner::run("tailscale", &["version"], RUN_TIMEOUT).code == 0 {
                return Ok(obj! { "ok" => true, "note" => "Tailscale já está instalado no sistema." });
            }
            Err("Tailscale precisa ser instalado no sistema (root): no Fedora, \"sudo dnf install tailscale && sudo systemctl enable --now tailscaled\".".into())
        }
        _ => Err("integração desconhecida".into()),
    }
}

pub fn integration_start(exec: bool, cfg: &Map, s: &Srv, name: &str) -> Result<Value, String> {
    let dir = integrations_dir(cfg);
    let srv_dir = intg_srv_dir(cfg, s);
    let sname = s.name_str();
    match name {
        "playit" => {
            let bin = jsutil::path_join(&[&dir, "playit"]);
            let toml = jsutil::path_join(&[&srv_dir, "playit.toml"]);
            if !exists(&bin) {
                return Err("playit não instalado".into());
            }
            if !exists(&toml) {
                return Err(format!("cole o secret key do playit.gg primeiro (servidor {})", sname));
            }
            let u = intg_unit(cfg, s, "playit").unwrap();
            write_user_unit(&u, &format!("craftbox — playit.gg agent ({})", sname), &playit_exec(&bin, &toml, &srv_dir));
            runner::daemon_reload(exec);
            let r = u_svc(exec, cfg, "start", &u);
            if r.code != 0 {
                return Err(if r.stderr.is_empty() { "falha ao iniciar playit".into() } else { r.stderr });
            }
            Ok(obj! { "ok" => true })
        }
        "cloudflare" => {
            let bin = jsutil::path_join(&[&dir, "cloudflared"]);
            let tok = jsutil::path_join(&[&srv_dir, "cloudflared.token"]);
            if !exists(&bin) {
                return Err("cloudflared não instalado".into());
            }
            if !exists(&tok) {
                return Err(format!("configure o token do túnel Cloudflare primeiro (servidor {})", sname));
            }
            let token = std::fs::read_to_string(&tok).map_err(|e| jsutil::fs_err(&e, "open", &tok))?;
            let u = intg_unit(cfg, s, "cloudflare").unwrap();
            write_user_unit(
                &u,
                &format!("craftbox — Cloudflare Tunnel ({})", sname),
                &format!("{} tunnel --no-autoupdate run --token {}", bin, jsutil::trim(&token)),
            );
            runner::daemon_reload(exec);
            let r = u_svc(exec, cfg, "start", &u);
            if r.code != 0 {
                return Err(if r.stderr.is_empty() { "falha ao iniciar cloudflared".into() } else { r.stderr });
            }
            Ok(obj! { "ok" => true })
        }
        "tailscale" => {
            let r = ts_ctl(&["up", "--accept-routes"]);
            let out = format!("{}{}", r.stdout, r.stderr);
            if let Some(i) = out.find("https://login.tailscale.com/") {
                let url: String = out[i..].chars().take_while(|c| !c.is_whitespace()).collect();
                if url.len() > "https://login.tailscale.com/".len() {
                    return Ok(obj! { "ok" => true, "loginUrl" => url });
                }
            }
            if r.code == 0 {
                return Ok(obj! { "ok" => true });
            }
            let fb = if !r.stderr.is_empty() { r.stderr.clone() } else if !r.stdout.is_empty() { r.stdout.clone() } else { "falha".into() };
            Err(ts_perm_msg(&out).unwrap_or_else(|| crate::servers::tail_chars(&fb, 200)))
        }
        _ => Err("integração desconhecida".into()),
    }
}

pub fn integration_stop(exec: bool, cfg: &Map, s: &Srv, name: &str) -> Result<Value, String> {
    if name == "tailscale" {
        let r = ts_ctl(&["down"]);
        if r.code == 0 {
            return Ok(obj! { "ok" => true });
        }
        let out = format!("{}{}", r.stderr, r.stdout);
        let tail = crate::servers::tail_chars(&out, 200);
        return Err(ts_perm_msg(&out).unwrap_or(if tail.is_empty() { "falha ao desconectar".into() } else { tail }));
    }
    let unit = intg_unit(cfg, s, name).ok_or("integração desconhecida")?;
    u_svc(exec, cfg, "stop", &unit);
    Ok(obj! { "ok" => true })
}

// ---------------------------------------------------------------------------
// Rede / Wi-Fi (via nmcli) — status, scan e conectar
// ---------------------------------------------------------------------------
/// nmcli -t escapa ':' como '\:'
fn nm_parse(line: &str) -> Vec<String> {
    line.replace("\\:", " ").split(':').map(|p| p.replace(' ', ":")).collect()
}

pub fn net_status() -> Value {
    let dev = runner::run("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE", "dev"], RUN_TIMEOUT);
    // /(^|\n)[^:]*:wifi:/
    let has_wifi = dev.stdout.split('\n').any(|l| l.split_once(':').is_some_and(|(_, rest)| rest.starts_with("wifi:")));
    let ipr = runner::run(
        "bash",
        &["-lc", "ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1"],
        RUN_TIMEOUT,
    );
    let g = runner::run("nmcli", &["-t", "-f", "CONNECTIVITY", "general"], RUN_TIMEOUT);
    let mut ssid = Value::Null;
    if has_wifi {
        let w = runner::run("nmcli", &["-t", "-f", "ACTIVE,SSID", "dev", "wifi"], RUN_TIMEOUT);
        if let Some(line) = w.stdout.split('\n').find(|l| l.starts_with("yes:")) {
            let p = nm_parse(line);
            if let Some(s) = p.get(1).filter(|s| !s.is_empty()) {
                ssid = Value::from(s.clone());
            }
        }
    }
    let ip = jsutil::trim(&ipr.stdout);
    obj! {
        "hasWifi" => has_wifi, "ip" => if ip.is_empty() { Value::Null } else { Value::from(ip) }, "ssid" => ssid,
        "connectivity" => jsutil::trim(&g.stdout),
    }
}

pub fn wifi_scan() -> Value {
    let r = runner::run(
        "nmcli",
        &["-t", "-f", "SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", "yes"],
        Duration::from_secs(30),
    );
    let mut seen: Vec<String> = Vec::new();
    let mut nets: Vec<(f64, Value)> = Vec::new();
    for l in r.stdout.split('\n') {
        if jsutil::trim(l).is_empty() {
            continue;
        }
        let p = nm_parse(l);
        let ssid = p[0].clone();
        if ssid.is_empty() || seen.contains(&ssid) {
            continue;
        }
        seen.push(ssid.clone());
        let sig = p.get(1).filter(|s| !s.is_empty()).map(|s| jsutil::to_number(s).unwrap_or(f64::NAN)).unwrap_or(0.0);
        let sec = p.get(2).map(|s| jsutil::trim(s).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "aberta".into());
        nets.push((sig, obj! { "ssid" => ssid, "signal" => sig, "security" => sec }));
    }
    // sort estável por sinal decrescente (NaN fica onde está, como no V8 com comparador NaN)
    nets.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Value::Arr(nets.into_iter().map(|x| x.1).collect())
}

pub fn wifi_connect(ssid: Option<&Value>, password: Option<&Value>) -> Result<Value, String> {
    if !truthy(ssid) {
        return Err("rede não informada".into());
    }
    let ssid = jsutil::to_string(ssid);
    let pw = jsutil::to_string_or_empty(password);
    let mut args = vec!["-n", "nmcli", "dev", "wifi", "connect", ssid.as_str()];
    if truthy(password) {
        args.push("password");
        args.push(&pw);
    }
    let r = runner::run("sudo", &args, Duration::from_secs(45));
    if r.code != 0 {
        let out = format!("{}{}", r.stderr, r.stdout);
        let l = out.to_lowercase();
        if l.contains("a password is required") || l.contains("sudo:") {
            return Err("sem permissão pra usar o nmcli aqui (no craftbox instalado já vem liberado).".into());
        }
        let t = crate::servers::tail_chars(jsutil::trim(&out), 200);
        return Err(if t.is_empty() { "falha ao conectar no Wi-Fi".into() } else { t });
    }
    Ok(obj! { "ok" => true })
}

/// Teste de velocidade de download (Cloudflare). Sob demanda — é um download real.
pub fn speed_test() -> Result<Value, String> {
    let start = jsutil::now_ms();
    let received = net::count_download("https://speed.cloudflare.com/__down?bytes=25000000", Duration::from_secs(30))? as f64;
    let secs = (jsutil::now_ms() - start) / 1000.0;
    let mbps = if secs > 0.0 { received * 8.0 / secs / 1e6 } else { 0.0 };
    Ok(obj! { "mbps" => jsutil::to_fixed(mbps, 1), "bytes" => received, "secs" => jsutil::to_fixed(secs, 2) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_parsing() {
        assert_eq!(nm_parse("yes:Casa\\: 5G"), vec!["yes", "Casa::5G"]); // o Node faz o mesmo
        assert_eq!(nm_parse("Minha Rede:80:WPA2"), vec!["Minha:Rede", "80", "WPA2"]); // o bug do Node, reproduzido
    }

    #[test]
    fn playit_addr() {
        assert_eq!(playit_address("tunnel at abc-def.craft.playit.gg:12345 ok").as_deref(), Some("abc-def.craft.playit.gg:12345"));
        assert_eq!(playit_address("x 147.tcp.playit.gg:9 y").as_deref(), Some("147.tcp.playit.gg:9"));
        assert_eq!(playit_address("nada aqui"), None);
    }
}
