//! Execução de processos e os dois gerenciadores de serviço do server.js:
//! - systemd: `systemctl [--user] is-active|show ...` (hosts físicos / ISO);
//! - runner exec: PID files em `runDir` (container, sem systemd).
//!
//! O modo é decidido uma vez na subida, como no Node:
//! `CRAFTBOX_RUNNER=exec` ou ausência de `/run/systemd/system`.
//!
//! Fase 1: leitura de estado (is-active/uptime) e o `unitStop` usado no
//! shutdown gracioso. `unitStart`/`spawnUnit` entram com a fatia de energia.

use crate::config;
use crate::json::Map;
use crate::jsutil::{self, to_number};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn detect_exec_runner() -> bool {
    std::env::var("CRAFTBOX_RUNNER").map(|v| v == "exec").unwrap_or(false)
        || !std::path::Path::new("/run/systemd/system").exists()
}

#[allow(dead_code)] // code/stderr entram com power/backups/integrações
pub struct RunResult {
    /// 0 = sucesso; qualquer outra coisa = falha (inclui ENOENT e timeout)
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// `run(cmd, args, {timeout})` do server.js (execFile; nunca rejeita).
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> RunResult {
    let mut child =
        match Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
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
    } else if crate::ctx::multi_enabled(cfg) {
        jsutil::path_join(&[&jsutil::path_dirname(&config::s(cfg, "serversDir")), "craftbox-run"])
    } else {
        jsutil::path_join(&[&jsutil::homedir(), ".craftbox-run"])
    };
    let _ = std::fs::create_dir_all(&base);
    base
}

fn run_pid_file(cfg: &Map, unit: &str) -> String {
    jsutil::path_join(&[&run_dir(cfg), &format!("{}.pid", jsutil::path_basename(unit))])
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
        if !proc_alive(pid) || n > 24 {
            break;
        }
    }
    if proc_alive(pid) {
        unsafe {
            libc::kill(pid_i, libc::SIGKILL);
        }
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
    fn pid_liveness() {
        assert!(proc_alive(std::process::id() as f64));
        assert!(!proc_alive(0.0));
        assert!(!proc_alive(1.5));
    }
}
