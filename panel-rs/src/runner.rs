//! Execução de processos e os dois gerenciadores de serviço do server.js:
//! - systemd: `systemctl [--user] ...` (hosts físicos / ISO), ações via
//!   `systemctl --user` (rootless) ou `sudo -n systemctl` (sudoers do appliance);
//! - runner exec: PID files e logs em `runDir` (container, sem systemd), com o
//!   painel dono dos processos (`bash -lc <start.sh | ExecStart da unidade>`).
//!
//! O modo é decidido uma vez na subida, como no Node:
//! `CRAFTBOX_RUNNER=exec` ou ausência de `/run/systemd/system`.
//!
//! Todo filho nasce com a máscara de sinais limpa: o `main` bloqueia
//! SIGTERM/SIGINT em todas as threads (pra `sigwait`), e a máscara é herdada
//! no fork — sem limpar, o servidor Minecraft ignoraria o SIGTERM do "parar"
//! (e não salvaria o mundo) e o timeout do `run()` não mataria nada.

use crate::config;
use crate::ctx;
use crate::json::Map;
use crate::jsutil::{self, to_number};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn detect_exec_runner() -> bool {
    std::env::var("CRAFTBOX_RUNNER").map(|v| v == "exec").unwrap_or(false)
        || !std::path::Path::new("/run/systemd/system").exists()
}

pub struct RunResult {
    /// 0 = sucesso; qualquer outra coisa = falha (inclui ENOENT e timeout)
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl RunResult {
    pub fn ok() -> Self {
        RunResult { code: 0, stdout: String::new(), stderr: String::new() }
    }
    /// `r.stderr || r.stdout`
    pub fn output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else {
            self.stderr.clone()
        }
    }
}

/// Opções do `execFile` usadas pelo server.js.
#[derive(Default)]
pub struct RunOpts<'a> {
    pub timeout: Option<Duration>,
    pub cwd: Option<&'a str>,
    pub env: Vec<(&'a str, String)>,
}

/// Desbloqueia todos os sinais no filho (ver doc do módulo).
fn clear_sigmask(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            Ok(())
        });
    }
}

/// `run(cmd, args, {timeout})` do server.js (execFile; nunca rejeita).
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> RunResult {
    run_with(cmd, args, RunOpts { timeout: Some(timeout), ..Default::default() })
}

pub fn run_with(cmd: &str, args: &[&str], o: RunOpts) -> RunResult {
    let timeout = o.timeout.unwrap_or(RUN_TIMEOUT);
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = o.cwd {
        c.current_dir(d);
    }
    for (k, v) in &o.env {
        c.env(k, v);
    }
    clear_sigmask(&mut c);
    let mut child = match c.spawn() {
        Ok(c) => c,
        Err(e) => return RunResult { code: -1, stdout: String::new(), stderr: e.to_string() },
    };
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    // execFile manda SIGTERM no timeout
                    unsafe {
                        libc::kill(child.id() as i32, libc::SIGTERM);
                    }
                    timed_out = true;
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned();
    let code = match status {
        _ if timed_out => 1,
        Some(s) => s.code().unwrap_or(1),
        None => 1,
    };
    RunResult { code, stdout, stderr }
}

pub const RUN_TIMEOUT: Duration = Duration::from_secs(15);

/// `scArgs`: prefixa `--user` no modo rootless.
pub fn sc_args<'a>(cfg: &Map, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = Vec::new();
    if config::systemctl_user(cfg) {
        v.push("--user");
    }
    v.extend_from_slice(rest);
    v
}

/// `runDir()` (cria a pasta, como o Node).
pub fn run_dir(cfg: &Map) -> String {
    let rd = config::s(cfg, "runDir");
    let base = if !rd.is_empty() {
        rd
    } else if ctx::multi_enabled(cfg) {
        jsutil::path_join(&[&jsutil::path_dirname(&config::s(cfg, "serversDir")), "craftbox-run"])
    } else {
        jsutil::path_join(&[&jsutil::homedir(), ".craftbox-run"])
    };
    let _ = std::fs::create_dir_all(&base);
    base
}

/// `userUnitDir()`: ~/.config/systemd/user (criada).
pub fn user_unit_dir() -> String {
    let d = jsutil::path_join(&[&jsutil::homedir(), ".config", "systemd", "user"]);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn run_unit_file(unit: &str) -> String {
    jsutil::path_join(&[&user_unit_dir(), &format!("{}.service", jsutil::path_basename(unit))])
}

fn run_pid_file(cfg: &Map, unit: &str) -> String {
    jsutil::path_join(&[&run_dir(cfg), &format!("{}.pid", jsutil::path_basename(unit))])
}

fn run_log_file(cfg: &Map, unit: &str) -> String {
    jsutil::path_join(&[&run_dir(cfg), &format!("{}.log", jsutil::path_basename(unit))])
}

/// `runInfo(unit)`: `{pid, startedAt}` do PID file (linha 1 = pid, 2 = epoch ms).
pub fn run_info(cfg: &Map, unit: &str) -> (f64, f64) {
    match std::fs::read(run_pid_file(cfg, unit)) {
        Ok(b) => {
            let txt = String::from_utf8_lossy(&b);
            let t = jsutil::trim(&txt);
            let mut ln = t.split('\n');
            let num = |s: Option<&str>| s.and_then(to_number).filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(0.0);
            (num(ln.next()), num(ln.next()))
        }
        Err(_) => (0.0, 0.0),
    }
}

/// `procAlive(pid)`: `process.kill(pid, 0)` sem lançar (EPERM conta como morto, igual ao Node).
pub fn proc_alive(pid: f64) -> bool {
    if pid == 0.0 || pid.fract() != 0.0 || pid.abs() > i32::MAX as f64 {
        return false;
    }
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn unit_active(cfg: &Map, unit: &str) -> &'static str {
    if proc_alive(run_info(cfg, unit).0) {
        "active"
    } else {
        "inactive"
    }
}

/// `unitExecFor(unit)`: comando e cwd de uma unidade no runner exec.
fn unit_exec_for(cfg: &Map, unit: &str) -> (Option<String>, String) {
    let unit = jsutil::path_basename(unit);
    let tpl = config::s(cfg, "serviceTemplate");
    // servidor Minecraft: usa o start.sh da instância (o script já faz cd e exec do java)
    if !tpl.is_empty() && unit.starts_with(&tpl) {
        let id = &unit[tpl.len()..];
        let dir = if ctx::multi_enabled(cfg) {
            jsutil::path_join(&[&config::s(cfg, "serversDir"), id])
        } else {
            config::s(cfg, "mcDir")
        };
        let has = std::path::Path::new(&jsutil::path_join(&[&dir, "start.sh"])).exists();
        return (has.then(|| "bash start.sh".to_string()), dir);
    }
    if unit == jsutil::to_string(cfg.get("service")) {
        let dir = config::s(cfg, "mcDir");
        let has = std::path::Path::new(&jsutil::path_join(&[&dir, "start.sh"])).exists();
        return (has.then(|| "bash start.sh".to_string()), dir);
    }
    // túneis/integrações: usa o ExecStart do arquivo de unidade gerado pelo painel
    if let Ok(content) = std::fs::read_to_string(run_unit_file(&unit)) {
        if let Some(line) = content.lines().find_map(|l| l.strip_prefix("ExecStart=")) {
            let c = jsutil::trim(line);
            if !c.is_empty() {
                return (Some(c.to_string()), crate::integrations::integrations_dir(cfg));
            }
        }
    }
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
    (None, cwd)
}

/// JDK por versão do Minecraft (o wrapper 'java' da imagem tem 8/17/21/25):
///   AA.x (calendário, 26.1+) → 25 | 1.20.5+ e 1.21+ → 21 | 1.17–1.20.4 → 17 | ≤1.16 → 8
/// Snapshots semanais antigos (AAwSSx) usam o ano/semana em que a Mojang subiu o
/// requisito. Desconhecido → 21.
pub fn java_for_mc(ver: &str) -> u32 {
    let s = jsutil::trim(ver).to_lowercase();
    let b = s.as_bytes();
    // ^(\d{2})w(\d{2})[a-z]$
    if b.len() == 6
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[2] == b'w'
        && b[3..5].iter().all(u8::is_ascii_digit)
        && b[5].is_ascii_lowercase()
    {
        let y: u32 = s[..2].parse().unwrap();
        let wk: u32 = s[3..5].parse().unwrap();
        if y > 24 || (y == 24 && wk >= 14) {
            return 21;
        }
        if y > 21 || (y == 21 && wk >= 19) {
            return 17;
        }
        return 8;
    }
    // ^(\d+)\.(\d+)(?:\.(\d+))?
    let lead = |t: &str| -> Option<(f64, usize)> {
        let n = t.bytes().take_while(u8::is_ascii_digit).count();
        (n > 0).then(|| (t[..n].parse::<f64>().unwrap_or(f64::INFINITY), n))
    };
    let Some((maj, n1)) = lead(&s) else { return 21 };
    let rest = &s[n1..];
    let Some(rest) = rest.strip_prefix('.') else { return 21 };
    let Some((min, n2)) = lead(rest) else { return 21 };
    let rest = &rest[n2..];
    let patch = rest.strip_prefix('.').and_then(lead).map(|x| x.0).unwrap_or(0.0);
    if maj >= 26.0 {
        return 25;
    }
    if maj != 1.0 {
        return 21;
    }
    if min >= 21.0 || (min == 20.0 && patch >= 5.0) {
        return 21;
    }
    if min >= 17.0 {
        return 17;
    }
    8
}

/// `spawnUnit`: `bash -lc <cmd>` desanexado (novo grupo de sessão), saída
/// anexada ao log da unidade, PID file `pid\nms\n`; ao sair, o PID file some.
fn spawn_unit(cfg: &Map, unit: &str, cmd: &str, cwd: &str, java: u32) -> Result<(), String> {
    let unit = jsutil::path_basename(unit);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_log_file(cfg, &unit))
        .map_err(|e| e.to_string())?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    let mut c = Command::new("/bin/bash");
    c.args(["-lc", cmd])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .env("CRAFTBOX_JAVA_MAJOR", java.to_string()); // o wrapper 'java' escolhe a JDK certa
    clear_sigmask(&mut c);
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    let pidf = run_pid_file(cfg, &unit);
    let _ = std::fs::write(&pidf, format!("{}\n{}\n", child.id(), crate::json::num_to_string(jsutil::now_ms())));
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = std::fs::remove_file(&pidf);
    });
    Ok(())
}

/// `unitStop`: SIGTERM, espera até ~12,5 s (25 × 500 ms) e SIGKILL.
pub fn unit_stop(cfg: &Map, unit: &str) {
    let (pid, _) = run_info(cfg, unit);
    if pid == 0.0 || !proc_alive(pid) {
        return;
    }
    let pid_i = pid as i32;
    unsafe {
        libc::kill(pid_i, libc::SIGTERM);
    }
    let mut n = 0;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        n += 1;
        // até 10 min: salvar o mundo num HD leva minutos (medido: 220 s num
        // Fabric 1.20.1 recém-gerado); SIGKILL antes disso corrompe o mundo
        if !proc_alive(pid) || n > 1200 {
            break;
        }
    }
    if proc_alive(pid) {
        unsafe {
            libc::kill(pid_i, libc::SIGKILL);
        }
    }
}

fn unit_start(cfg: &Map, unit: &str) -> RunResult {
    if unit_active(cfg, unit) == "active" {
        return RunResult::ok();
    }
    let (cmd, cwd) = unit_exec_for(cfg, unit);
    let Some(cmd) = cmd else {
        return RunResult {
            code: 1,
            stdout: String::new(),
            stderr: format!(
                "sem comando de execução para a unidade {} (falta start.sh ou o ExecStart da unidade)",
                unit
            ),
        };
    };
    let java = java_for_mc(&jsutil::to_string_or_empty(ctx::read_instance_meta(&cwd).get("mcVersion")));
    match spawn_unit(cfg, unit, &cmd, &cwd, java) {
        Ok(()) => RunResult::ok(),
        // no Node o spawn assíncrono não falha aqui (vira 'error' no filho); aqui avisamos
        Err(e) => RunResult { code: 1, stdout: String::new(), stderr: e },
    }
}

/// `unitAction(action, unit)`
pub fn unit_action(cfg: &Map, action: &str, unit: &str) -> RunResult {
    match action {
        "start" => unit_start(cfg, unit),
        "restart" => {
            let was = unit_active(cfg, unit);
            unit_stop(cfg, unit);
            if was == "active" {
                unit_start(cfg, unit)
            } else {
                RunResult::ok()
            }
        }
        _ => {
            unit_stop(cfg, unit);
            RunResult::ok()
        }
    }
}

/// `unitJournal(unit, lines)`: últimas linhas do log do runner.
pub fn unit_journal(cfg: &Map, unit: &str, lines: usize) -> String {
    match std::fs::read(run_log_file(cfg, unit)) {
        Ok(b) => {
            let data = String::from_utf8_lossy(&b);
            let v: Vec<&str> = data.split('\n').collect();
            let s = v[v.len().saturating_sub(if lines == 0 { 120 } else { lines })..].join("\n");
            if s.is_empty() {
                "(sem logs)".into()
            } else {
                s
            }
        }
        Err(_) => "(sem logs)".into(),
    }
}

/// `unitJournal(unit, lines)` com o `lines` cru do `/api/logs` (NaN/0 → 120,
/// negativo → `slice(-n)` corta do começo, como no JS).
pub fn unit_journal_js(cfg: &Map, unit: &str, lines: Option<f64>) -> String {
    let n = lines.filter(|n| *n != 0.0).unwrap_or(120.0);
    let Ok(b) = std::fs::read(run_log_file(cfg, unit)) else { return "(sem logs)".into() };
    let data = String::from_utf8_lossy(&b);
    let v: Vec<&str> = data.split('\n').collect();
    let len = v.len() as f64;
    let start = if n > 0.0 { (len - n).max(0.0) } else { (-n).min(len) };
    let s = v[start as usize..].join("\n");
    if s.is_empty() {
        "(sem logs)".into()
    } else {
        s
    }
}

/// Unidades com PID file no runDir (usado no shutdown gracioso).
pub fn units_with_pidfile(cfg: &Map) -> Vec<String> {
    match std::fs::read_dir(run_dir(cfg)) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".pid").map(String::from))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `svcActiveOf(service)`
pub fn svc_active_of(exec_runner: bool, cfg: &Map, service: &str) -> String {
    if exec_runner {
        return unit_active(cfg, service).into();
    }
    let r = run("systemctl", &sc_args(cfg, &["is-active", service]), RUN_TIMEOUT);
    let t = jsutil::trim(&r.stdout);
    if t.is_empty() {
        "unknown".into()
    } else {
        t.into()
    }
}

/// `svcAction(action, service)`: Err = "acao invalida" (o Node lança).
pub fn svc_action(exec_runner: bool, cfg: &Map, action: &str, service: &str) -> Result<RunResult, String> {
    if !matches!(action, "start" | "stop" | "restart") {
        return Err("acao invalida".into());
    }
    if exec_runner {
        return Ok(unit_action(cfg, action, service));
    }
    // rootless: systemctl --user <action> <svc> | sistema: sudoers permitindo systemctl <action> <svc>
    // stop sem bloquear: salvar o mundo leva minutos e o systemctl estouraria o
    // RUN_TIMEOUT; a UI acompanha pelo estado "deactivating"
    let mut args = vec![action, service];
    if action == "stop" {
        args.push("--no-block");
    }
    Ok(if config::systemctl_user(cfg) {
        let mut a = vec!["--user"];
        a.extend(args);
        run("systemctl", &a, RUN_TIMEOUT)
    } else {
        let mut a = vec!["-n", "systemctl"];
        a.extend(args);
        run("sudo", &a, RUN_TIMEOUT)
    })
}

/// Parada de emergência: pede a parada sem esperar a unidade terminar.
/// `force` = SIGKILL direto (perde o que não foi salvo), pra quando a máquina
/// está travada em swap e o servidor nem responde ao SIGTERM.
pub fn svc_stop_nowait(exec_runner: bool, cfg: &Map, service: &str, force: bool) -> RunResult {
    if exec_runner {
        let (pid, _) = run_info(cfg, service);
        if pid == 0.0 || !proc_alive(pid) {
            return RunResult::ok();
        }
        if force {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        } else {
            let (cfg, service) = (cfg.clone(), service.to_string());
            std::thread::spawn(move || unit_stop(&cfg, &service));
        }
        return RunResult::ok();
    }
    // forçado: enfileira o stop antes do SIGKILL, senão o Restart=on-failure religa o servidor
    let mut steps: Vec<Vec<&str>> = vec![vec!["stop", service, "--no-block"]];
    if force {
        steps.push(vec!["kill", "--signal=SIGKILL", service]);
    }
    let mut last = RunResult::ok();
    for args in steps {
        let mut a: Vec<&str> = if config::systemctl_user(cfg) { vec!["systemctl", "--user"] } else { vec!["sudo", "-n", "systemctl"] };
        a.extend(args);
        last = run(a[0], &a[1..], RUN_TIMEOUT);
        if last.code != 0 {
            break;
        }
    }
    last
}

/// `daemonReload()`
pub fn daemon_reload(exec_runner: bool) -> RunResult {
    if exec_runner {
        return RunResult::ok();
    }
    run("systemctl", &["--user", "daemon-reload"], RUN_TIMEOUT)
}

/// `svcUptime()`: segundos desde a subida, ou None (null).
pub fn svc_uptime(exec_runner: bool, cfg: &Map, service: &str) -> Option<f64> {
    let now = jsutil::now_ms();
    if exec_runner {
        let (_, started) = run_info(cfg, service);
        return if started != 0.0 { Some(((now - started) / 1000.0).floor().max(0.0)) } else { None };
    }
    let r =
        run("systemctl", &sc_args(cfg, &["show", service, "--property=ActiveEnterTimestamp", "--value"]), RUN_TIMEOUT);
    let t = parse_systemd_timestamp(jsutil::trim(&r.stdout))?;
    Some(((now - t) / 1000.0).floor().max(0.0))
}

/// Subconjunto do `Date.parse` do V8 para o formato do systemd
/// ("Thu 2026-09-18 17:02:03 -03"). Replica inclusive o que o V8 NÃO entende:
/// abreviações como "BRT"/"CEST" viram NaN (null) no Node, e aqui também.
pub fn parse_systemd_timestamp(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let (date, time, tz) = match parts.as_slice() {
        [_wd, d, t] => (*d, *t, None),
        [_wd, d, t, z] => (*d, *t, Some(*z)),
        _ => return None,
    };
    let dp: Vec<&str> = date.split('-').collect();
    let tp: Vec<&str> = time.split(':').collect();
    if dp.len() != 3 || !(2..=3).contains(&tp.len()) {
        return None;
    }
    let num = |x: &str| -> Option<i64> {
        if x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            x.parse().ok()
        }
    };
    let (y, mo, d) = (num(dp[0])?, num(dp[1])?, num(dp[2])?);
    let (h, mi) = (num(tp[0])?, num(tp[1])?);
    let sec = if tp.len() == 3 { num(tp[2])? } else { 0 };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || sec > 59 {
        return None;
    }
    let offset_min: Option<i64> = match tz {
        None => None, // hora local
        Some(z) => Some(match z.to_ascii_uppercase().as_str() {
            "UT" | "UTC" | "GMT" | "Z" => 0,
            "EST" => -300,
            "EDT" => -240,
            "CST" => -360,
            "CDT" => -300,
            "MST" => -420,
            "MDT" => -360,
            "PST" => -480,
            "PDT" => -420,
            other => {
                let (sign, rest) = match other.as_bytes().first() {
                    Some(b'+') => (1, &other[1..]),
                    Some(b'-') => (-1, &other[1..]),
                    _ => return None, // abreviação desconhecida → NaN no V8
                };
                let rest = rest.replace(':', "");
                let n = num(&rest)?;
                let (hh, mm) = match rest.len() {
                    1 | 2 => (n, 0),
                    3 | 4 => (n / 100, n % 100),
                    _ => return None,
                };
                sign * (hh * 60 + mm)
            }
        }),
    };
    let days = days_from_civil(y, mo, d);
    let secs_naive = days * 86400 + h * 3600 + mi * 60 + sec;
    let secs = match offset_min {
        Some(off) => secs_naive - off * 60,
        None => {
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            tm.tm_year = (y - 1900) as i32;
            tm.tm_mon = (mo - 1) as i32;
            tm.tm_mday = d as i32;
            tm.tm_hour = h as i32;
            tm.tm_min = mi as i32;
            tm.tm_sec = sec as i32;
            tm.tm_isdst = -1;
            let t = unsafe { libc::mktime(&mut tm) };
            if t == -1 {
                return None;
            }
            t
        }
    };
    Some(secs as f64 * 1000.0)
}

/// Dias desde 1970-01-01 (algoritmo de Howard Hinnant).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_timestamps_like_v8() {
        // valores conferidos com `node -e "Date.parse(...)"`
        assert_eq!(parse_systemd_timestamp("Thu 2026-09-18 17:02:03 -03"), Some(1789761723000.0));
        assert_eq!(parse_systemd_timestamp("Thu 2026-09-18 17:02:03 UTC"), Some(1789750923000.0));
        assert_eq!(parse_systemd_timestamp("Thu 2026-09-18 17:02:03 EST"), Some(1789768923000.0));
        assert_eq!(parse_systemd_timestamp("Thu 2026-09-18 17:02:03 BRT"), None);
        assert_eq!(parse_systemd_timestamp("Thu 2026-09-18 17:02:03 CEST"), None);
        assert_eq!(parse_systemd_timestamp(""), None);
        assert_eq!(parse_systemd_timestamp("n/a"), None);
    }

    #[test]
    fn run_captures_and_times_out() {
        let r = run("sh", &["-c", "echo out; echo err >&2; exit 3"], RUN_TIMEOUT);
        assert_eq!((r.code, r.stdout.as_str(), r.stderr.as_str()), (3, "out\n", "err\n"));
        let r = run("sleep", &["5"], Duration::from_millis(200));
        assert_ne!(r.code, 0);
        let r = run("/nao/existe", &[], RUN_TIMEOUT);
        assert_ne!(r.code, 0);
    }

    #[test]
    fn child_signal_mask_is_clear() {
        // bloqueia SIGTERM nesta thread (como o main) e confere que o filho não herda
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
        let t0 = Instant::now();
        let r = run("sleep", &["5"], Duration::from_millis(300));
        assert_ne!(r.code, 0);
        assert!(t0.elapsed() < Duration::from_secs(3), "SIGTERM do timeout não matou o filho");
        let r = run("sh", &["-c", "grep SigBlk /proc/self/status"], RUN_TIMEOUT);
        assert!(r.stdout.contains("0000000000000000"), "{}", r.stdout);
    }

    #[test]
    fn pid_liveness() {
        assert!(proc_alive(std::process::id() as f64));
        assert!(!proc_alive(0.0));
        assert!(!proc_alive(1.5));
    }

    #[test]
    fn java_versions_like_node() {
        for (v, j) in [
            ("26.1", 25),
            ("26.3", 25),
            ("1.21.1", 21),
            ("1.20.5", 21),
            ("1.20.4", 17),
            ("1.17", 17),
            ("1.16.5", 8),
            ("1.20.5-pre1", 21),
            ("24w14a", 21),
            ("24w13a", 17),
            ("21w19a", 17),
            ("21w18a", 8),
            ("", 21),
            ("abc", 21),
            ("2.0", 21),
        ] {
            assert_eq!(java_for_mc(v), j, "{}", v);
        }
    }
}
