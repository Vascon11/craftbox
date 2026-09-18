//! `systemStats()` do server.js. As fontes são as mesmas que o libuv usa por
//! baixo dos `os.*` do Node no Linux:
//! - os.totalmem/freemem → /proc/meminfo (MemTotal / MemAvailable)
//! - os.loadavg          → /proc/loadavg
//! - os.cpus().length    → linhas `cpuN` de /proc/stat
//! - os.uptime           → /proc/uptime
//! - fs.statfsSync       → statfs(2), com `bsize = f_bsize` e `bavail`.

use crate::json::Value;
use crate::jsutil::{self, math_round, to_fixed};
use crate::obj;

/// Abaixo disso o dashboard avisa (mundo/logs podem falhar ao gravar).
pub const DISK_LOW_GB: f64 = 2.0;

fn read_num(p: &str) -> Option<f64> {
    let s = std::fs::read(p).ok()?;
    jsutil::parse_int(jsutil::trim(&String::from_utf8_lossy(&s)))
}

/// `readNum(p) ? Math.round(v / 1000) : null` (0 e NaN viram null)
fn khz_to_mhz(v: Option<f64>) -> Value {
    match v {
        Some(n) if n != 0.0 => Value::Num(math_round(n / 1000.0)),
        _ => Value::Null,
    }
}

pub fn cpu_freq() -> Value {
    obj! {
        "curMHz" => khz_to_mhz(read_num("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")),
        "maxMHz" => khz_to_mhz(read_num("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")),
    }
}

pub fn cpu_temp() -> Value {
    for z in ["/sys/class/thermal/thermal_zone0/temp", "/sys/class/thermal/thermal_zone1/temp"] {
        if let Some(v) = read_num(z) {
            if v != 0.0 && v > 1000.0 {
                return Value::Num(math_round(v / 1000.0));
            }
        }
    }
    Value::Null
}

fn meminfo_kb(key: &str, txt: &str) -> Option<f64> {
    txt.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        rest.split_whitespace().next()?.parse::<f64>().ok()
    })
}

/// (total, livre) em bytes, como os.totalmem()/os.freemem().
pub fn mem_bytes() -> (f64, f64) {
    let txt = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total = meminfo_kb("MemTotal", &txt).unwrap_or(0.0) * 1024.0;
    let free = meminfo_kb("MemAvailable", &txt).or_else(|| meminfo_kb("MemFree", &txt)).unwrap_or(0.0) * 1024.0;
    (total, free)
}

pub fn loadavg() -> Vec<Value> {
    let txt = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut v: Vec<Value> =
        txt.split_whitespace().take(3).map(|x| Value::Num(to_fixed(x.parse::<f64>().unwrap_or(0.0), 2))).collect();
    while v.len() < 3 {
        v.push(Value::Num(0.0));
    }
    v
}

pub fn cpu_count() -> usize {
    let txt = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let n =
        txt.lines().filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(|b| b.is_ascii_digit())).count();
    if n > 0 {
        n
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    }
}

pub fn host_uptime() -> f64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0)
        .floor()
}

pub struct Statfs {
    pub bsize: f64,
    pub blocks: f64,
    pub bavail: f64,
}

pub fn statfs(dir: &str) -> Option<Statfs> {
    let c = std::ffi::CString::new(dir).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(Statfs { bsize: s.f_bsize as f64, blocks: s.f_blocks as f64, bavail: s.f_bavail as f64 })
}

pub fn disk(dir: &str) -> Value {
    let Some(s) = statfs(dir) else { return Value::Null };
    let total_gb = (s.blocks * s.bsize) / 1073741824.0;
    // bavail (não bfree): bfree inclui os ~5% reservados ao root no ext4
    let free_gb = (s.bavail * s.bsize) / 1073741824.0;
    obj! {
        "totalGB" => to_fixed(total_gb, 1),
        "usedGB" => to_fixed(total_gb - free_gb, 1),
        "freeGB" => to_fixed(free_gb, 1),
        "low" => free_gb < DISK_LOW_GB,
        "lowGB" => DISK_LOW_GB,
    }
}

pub fn system_stats(dir: &str) -> Value {
    let (total, free) = mem_bytes();
    let total_mb = math_round(total / 1048576.0);
    let free_mb = math_round(free / 1048576.0);
    obj! {
        "load" => loadavg(),
        "cpus" => cpu_count(),
        "cpu" => cpu_freq(),
        "tempC" => cpu_temp(),
        "mem" => obj!{ "total" => total_mb, "free" => free_mb, "used" => total_mb - free_mb },
        "disk" => disk(dir),
        "uptimeHost" => host_uptime(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_shape() {
        let d = disk("/");
        let o = d.as_obj().expect("statfs em / deve funcionar");
        let keys: Vec<_> = o.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["totalGB", "usedGB", "freeGB", "low", "lowGB"]);
        assert!(disk("/nao/existe/mesmo").is_null());
    }

    #[test]
    fn meminfo_parse() {
        let t = "MemTotal:       16000000 kB\nMemFree:  100 kB\nMemAvailable:   8000000 kB\n";
        assert_eq!(meminfo_kb("MemTotal", t), Some(16000000.0));
        assert_eq!(meminfo_kb("MemAvailable", t), Some(8000000.0));
        assert_eq!(meminfo_kb("Mem", t), None);
    }
}
