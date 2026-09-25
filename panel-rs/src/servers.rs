//! Instâncias (multi-servidor): resolver/baixar Paper, Fabric, Forge, NeoForge
//! e Pumpkin; criar, clonar, apagar e listar; scripts start.sh/backup.sh;
//! server.properties inicial; tamanho do mundo. Mesma lógica do server.js.

use crate::config;
use crate::ctx::{self, Srv};
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::net::{self, enc};
use crate::obj;
use crate::runner::{self, RunOpts};
use crate::State;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Progresso de instalação (um por vez neste appliance) — lido pela tela de
// carregamento via GET /api/servers/create-progress.
// ---------------------------------------------------------------------------
pub static PROGRESS: std::sync::Mutex<Option<Map>> = std::sync::Mutex::new(None);

pub fn progress_start(name: &str) {
    let now = jsutil::now_ms();
    if let Value::Obj(m) = obj! { "name" => name, "phase" => "Preparando…", "done" => 0, "total" => 0, "startedAt" => now, "at" => now } {
        *PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(m);
    }
}
pub fn progress_clear() {
    *PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
pub fn progress_active() -> bool {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}
pub fn progress_get() -> Option<Map> {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
/// `setPhase(phase, extra)`
pub fn set_phase(phase: &str, extra: &[(&str, Value)]) {
    if let Some(p) = PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        p.insert("phase", Value::from(phase));
        p.insert("at", Value::Num(jsutil::now_ms()));
        for (k, v) in extra {
            p.insert(*k, v.clone());
        }
    }
}
pub fn progress_set(k: &str, v: Value) {
    if let Some(p) = PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        p.insert(k, v);
    }
}
pub fn progress_inc_done() {
    if let Some(p) = PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let n = p.get("done").and_then(|v| v.as_f64()).unwrap_or(0.0);
        p.insert("done", Value::Num(n + 1.0));
    }
}

// ---------------------------------------------------------------------------
// Resolvers
// ---------------------------------------------------------------------------
pub struct Resolved {
    pub version: String,
    pub url: String,
}

fn nonempty(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.is_empty())
}

pub fn paper_resolve(version: Option<&str>) -> Result<Resolved, String> {
    let ver = match nonempty(version) {
        Some(v) => v.to_string(),
        None => {
            let proj = net::get_json("https://fill.papermc.io/v3/projects/paper")?;
            let mut all: Vec<Value> = Vec::new();
            if let Some(Value::Obj(vs)) = proj.get("versions") {
                for (_, arr) in vs.iter() {
                    if let Some(a) = arr.as_arr() {
                        all.extend(a.iter().cloned());
                    }
                }
            }
            let pick = all
                .iter()
                .find(|v| !jsutil::to_string(Some(v)).contains('-'))
                .or_else(|| all.first())
                .map(|v| jsutil::to_string(Some(v)))
                .unwrap_or_else(|| "undefined".into());
            pick
        }
    };
    let info = net::get_json(&format!("https://fill.papermc.io/v3/projects/paper/versions/{}/builds/latest", enc(&ver)))?;
    let dl = info
        .get("downloads")
        .and_then(|d| d.get("server:default").filter(|x| truthy(Some(x))).or_else(|| d.get("server:mojmap")));
    match dl.and_then(|d| d.get("url")).filter(|u| truthy(Some(u))) {
        Some(u) => Ok(Resolved { version: ver, url: jsutil::to_string(Some(u)) }),
        None => Err(format!("sem build de Paper pra {}", ver)),
    }
}

fn first_stable(list: &Value) -> Option<Value> {
    let a = list.as_arr()?;
    a.iter().find(|g| truthy(g.get("stable"))).or_else(|| a.first()).cloned()
}

pub fn fabric_resolve_server(version: Option<&str>) -> Result<Resolved, String> {
    let ver = match nonempty(version) {
        Some(v) => v.to_string(),
        None => {
            let games = net::get_json("https://meta.fabricmc.net/v2/versions/game")?;
            jsutil::to_string(first_stable(&games).as_ref().and_then(|g| g.get("version")))
        }
    };
    let loaders = net::get_json("https://meta.fabricmc.net/v2/versions/loader")?;
    let loader = jsutil::to_string(first_stable(&loaders).as_ref().and_then(|g| g.get("version")));
    let insts = net::get_json("https://meta.fabricmc.net/v2/versions/installer")?;
    let inst = jsutil::to_string(first_stable(&insts).as_ref().and_then(|g| g.get("version")));
    let url = format!(
        "https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar",
        enc(&ver),
        enc(&loader),
        enc(&inst)
    );
    Ok(Resolved { version: ver, url })
}

/// compara versões numéricas ("1.20.1" < "1.21", "21.1.77" < "21.1.100")
pub fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<f64> {
        s.split(['.', '-']).map(|x| jsutil::parse_int(x).filter(|n| *n != 0.0).unwrap_or(0.0)).collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let d = pa.get(i).copied().unwrap_or(0.0) - pb.get(i).copied().unwrap_or(0.0);
        if d != 0.0 {
            return if d < 0.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
    }
    std::cmp::Ordering::Equal
}

/// `v` casa com `^\d+(\.\d+)+$`
fn is_plain_version(v: &str) -> bool {
    let p: Vec<&str> = v.split('.').collect();
    p.len() >= 2 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
}

/// Forge: promotions_slim.json tem "<mc>-recommended"/"<mc>-latest" → build do Forge
pub fn forge_resolve(version: Option<&str>) -> Result<(String, String), String> {
    let data = net::get_json("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")?;
    let promos = data.get("promos").and_then(|p| p.as_obj()).cloned().unwrap_or_default();
    let mc = match nonempty(version) {
        Some(v) => v.to_string(),
        None => {
            let mut mcs: Vec<String> = Vec::new();
            for (k, _) in promos.iter() {
                let base = k.strip_suffix("-latest").or_else(|| k.strip_suffix("-recommended")).unwrap_or(k);
                if !mcs.iter().any(|m| m == base) {
                    mcs.push(base.to_string());
                }
            }
            mcs.retain(|v| is_plain_version(v));
            mcs.sort_by(|a, b| cmp_ver(a, b));
            mcs.pop().unwrap_or_else(|| "undefined".into())
        }
    };
    let fv = promos
        .get(&format!("{}-recommended", mc))
        .filter(|x| truthy(Some(x)))
        .or_else(|| promos.get(&format!("{}-latest", mc)).filter(|x| truthy(Some(x))));
    match fv {
        Some(v) => Ok((mc, jsutil::to_string(Some(v)))),
        None => Err(format!("o Forge não tem build pro Minecraft {}", mc)),
    }
}

/// NeoForge numera pela versão do MC: 1.21.1 → 21.1.x | 1.21 → 21.0.x | 26.1.2 → 26.1.2.x | 26.2 → 26.2.0.x
fn neo_prefix(mc: &str) -> String {
    // String(mc).split('.').map(Number): p[i] cru (NaN aparece como "NaN"), `p[i] || 0` troca falsy por 0
    let p: Vec<f64> = mc.split('.').map(|x| jsutil::to_number(x).unwrap_or(f64::NAN)).collect();
    let raw = |i: usize| p.get(i).map(|n| json::num_to_string(*n)).unwrap_or_else(|| "undefined".into());
    let or0 = |i: usize| json::num_to_string(p.get(i).copied().filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(0.0));
    if p.first() == Some(&1.0) {
        format!("{}.{}.", raw(1), or0(2))
    } else {
        format!("{}.{}.{}.", raw(0), or0(1), or0(2))
    }
}

fn neo_to_mc(nv: &str) -> String {
    let p: Vec<f64> = nv.split(['.', '-']).map(|x| jsutil::to_number(x).unwrap_or(f64::NAN)).collect();
    let n = |i: usize| p.get(i).copied().unwrap_or(f64::NAN);
    let s = |x: f64| json::num_to_string(x);
    let t = |x: f64| x != 0.0 && !x.is_nan();
    if n(0) >= 26.0 {
        format!("{}.{}{}", s(n(0)), s(n(1)), if t(n(2)) { format!(".{}", s(n(2))) } else { String::new() })
    } else {
        format!("1.{}{}", s(n(0)), if t(n(1)) { format!(".{}", s(n(1))) } else { String::new() })
    }
}

pub fn neoforge_resolve(version: Option<&str>) -> Result<(String, String), String> {
    let data = net::get_json("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge")?;
    // /^\d+\.\d+\.\d+/
    let starts3 = |v: &str| {
        let p: Vec<&str> = v.splitn(4, '.').collect();
        p.len() >= 3
            && p[..2].iter().all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
            && p[2].bytes().next().is_some_and(|b| b.is_ascii_digit())
    };
    let all: Vec<String> = data
        .get("versions")
        .and_then(|v| v.as_arr())
        .map(|a| a.iter().filter_map(|x| x.as_str()).filter(|x| starts3(x)).map(String::from).collect())
        .unwrap_or_default();
    let cands: Vec<String> = match nonempty(version) {
        Some(v) => {
            let pre = neo_prefix(v);
            all.into_iter().filter(|x| x.starts_with(&pre)).collect()
        }
        None => all,
    };
    if cands.is_empty() {
        return Err(format!("o NeoForge não tem build pro Minecraft {}", version.unwrap_or("undefined")));
    }
    let stable: Vec<String> = cands.iter().filter(|v| !v.contains("-beta") && !v.contains("-alpha")).cloned().collect();
    let mut pool = if stable.is_empty() { cands } else { stable };
    pool.sort_by(|a, b| cmp_ver(a, b));
    let nv = pool.pop().unwrap();
    let mc = match nonempty(version) {
        Some(v) => v.to_string(),
        None => neo_to_mc(&nv),
    };
    Ok((mc, nv))
}

// ---------------------------------------------------------------------------
// Instaladores (Fabric/Quilt/Forge/NeoForge)
// ---------------------------------------------------------------------------
pub fn install_fabric_server(dir: &str, mc: &str, loader_ver: Option<&str>) -> Result<(), String> {
    let insts = net::get_json("https://meta.fabricmc.net/v2/versions/installer")?;
    let inst = jsutil::to_string(first_stable(&insts).as_ref().and_then(|g| g.get("version")));
    let dest = jsutil::path_join(&[dir, "server.jar"]);
    let grab = |lv: &str| {
        net::download(
            &format!("https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar", enc(mc), enc(lv), enc(&inst)),
            &dest,
        )
    };
    let latest_for = || -> Option<String> {
        let ls = net::get_json(&format!("https://meta.fabricmc.net/v2/versions/loader/{}", enc(mc))).ok()?;
        ls.as_arr()?.first()?.get("loader")?.get("version").map(|v| jsutil::to_string(Some(v)))
    };
    let lv = match nonempty(loader_ver) {
        Some(l) => l.to_string(),
        None => latest_for().unwrap_or_else(|| "undefined".into()),
    };
    if let Err(e) = grab(&lv) {
        // o loader fixado pelo pack pode ser velho demais pro endpoint server/jar (dá 400) — cai pro mais recente daquele MC
        match latest_for() {
            Some(latest) if latest != lv => grab(&latest)?,
            _ => return Err(e),
        }
    }
    Ok(())
}

pub fn install_quilt_server(dir: &str, mc: &str, loader_ver: Option<&str>) -> Result<(), String> {
    let insts = net::get_json("https://meta.quiltmc.org/v3/versions/installer")?;
    let iv = jsutil::to_string(insts.as_arr().and_then(|a| a.first()).and_then(|x| x.get("version")));
    let url = format!(
        "https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-installer/{0}/quilt-installer-{0}.jar",
        iv
    );
    let jar = jsutil::path_join(&[dir, "quilt-installer.jar"]);
    net::download(&url, &jar)?;
    let mut args: Vec<String> = vec!["-jar".into(), jar.clone(), "install".into(), "server".into(), mc.into()];
    if let Some(l) = nonempty(loader_ver) {
        args.push(format!("--loader={}", l));
    }
    args.push("--download-server".into());
    args.push(format!("--install-dir={}", dir));
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = runner::run_with(
        &java_bin(mc),
        &a,
        RunOpts {
            timeout: Some(Duration::from_secs(600)),
            cwd: Some(dir),
            env: vec![("CRAFTBOX_JAVA_MAJOR", runner::java_for_mc(mc).to_string())],
        },
    );
    if r.code != 0 {
        return Err(format!("instalador Quilt falhou: {}", tail_chars(&r.output(), 200)));
    }
    let _ = std::fs::remove_file(&jar);
    let launch = jsutil::path_join(&[dir, "quilt-server-launch.jar"]);
    if std::path::Path::new(&launch).exists() {
        let _ = std::fs::copy(&launch, jsutil::path_join(&[dir, "server.jar"]));
    }
    Ok(())
}

/// `str.slice(-n)` (em caracteres)
pub fn tail_chars(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

fn find_args_file(dir: &str) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    fn walk(d: &str, found: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut ents: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let fp = jsutil::path_join(&[d, &e.file_name().to_string_lossy()]);
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                walk(&fp, found);
            } else if e.file_name() == "unix_args.txt" {
                found.push(fp);
            }
        }
    }
    walk(&jsutil::path_join(&[dir, "libraries"]), &mut found);
    let f = found.into_iter().next()?;
    Some(f.strip_prefix(&format!("{}/", dir.trim_end_matches('/'))).unwrap_or(&f).to_string())
}

fn run_installer(dir: &str, url: &str, label: &str, mc: &str) -> Result<(), String> {
    let jar = jsutil::path_join(&[dir, "installer.jar"]);
    net::download(url, &jar)?;
    // no appliance a JDK certa vem do java_bin; no Docker o wrapper 'java' escolhe por CRAFTBOX_JAVA_MAJOR
    let r = runner::run_with(
        &java_bin(mc),
        &["-jar", &jar, "--installServer"],
        RunOpts {
            timeout: Some(Duration::from_secs(900)),
            cwd: Some(dir),
            env: vec![("CRAFTBOX_JAVA_MAJOR", runner::java_for_mc(mc).to_string())],
        },
    );
    let _ = std::fs::remove_file(&jar);
    let _ = std::fs::remove_file(jsutil::path_join(&[dir, "installer.jar.log"]));
    if r.code != 0 {
        return Err(format!("instalador {} falhou: {}", label, tail_chars(&r.output(), 250)));
    }
    Ok(())
}

pub fn install_forge_server(dir: &str, mc: &str, forge_ver: &str) -> Result<(), String> {
    let v = format!("{}-{}", mc, forge_ver);
    run_installer(dir, &format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{0}/forge-{0}-installer.jar", v), "Forge", mc)
}

pub fn install_neoforge_server(dir: &str, mc: &str, neo_ver: &str) -> Result<(), String> {
    // no 1.20.1 o NeoForge ainda publicava como net.neoforged:forge:1.20.1-47.1.x
    if mc == "1.20.1" {
        let v = if neo_ver.starts_with("1.20.1-") { neo_ver.to_string() } else { format!("1.20.1-{}", neo_ver) };
        return run_installer(dir, &format!("https://maven.neoforged.net/releases/net/neoforged/forge/{0}/forge-{0}-installer.jar", v), "NeoForge", mc);
    }
    run_installer(
        dir,
        &format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{0}/neoforge-{0}-installer.jar", neo_ver),
        "NeoForge",
        mc,
    )
}

pub fn install_loader(dir: &str, loader: &str, mc: &str, loader_ver: &str) -> Result<(), String> {
    match loader {
        "fabric" => install_fabric_server(dir, mc, Some(loader_ver)),
        "quilt" => install_quilt_server(dir, mc, Some(loader_ver)),
        "forge" => install_forge_server(dir, mc, loader_ver),
        "neoforge" => install_neoforge_server(dir, mc, loader_ver),
        _ => Err(format!("loader não suportado: {}", loader)),
    }
}

// ---------------------------------------------------------------------------
// Arquivos da instância
// ---------------------------------------------------------------------------
pub fn heap_mb() -> f64 {
    let total_mb = jsutil::math_round(crate::stats::mem_bytes().0 / 1048576.0);
    (total_mb * 0.4).floor().clamp(512.0, 3072.0)
}

fn used_ports(cfg: &Map) -> Vec<f64> {
    let mut v = Vec::new();
    for id in ctx::list_instance_ids(cfg) {
        let m = ctx::read_instance_meta(&jsutil::path_join(&[&config::s(cfg, "serversDir"), &id]));
        if truthy(m.get("port")) {
            let p = match m.get("port") {
                Some(Value::Num(n)) => *n,
                Some(Value::Str(s)) => jsutil::to_number(s).unwrap_or(f64::NAN),
                _ => f64::NAN,
            };
            v.push(p);
        }
    }
    v
}

pub fn free_port(cfg: &Map) -> f64 {
    let used = used_ports(cfg);
    let mut p = 25565.0;
    while used.contains(&p) {
        p += 1.0;
    }
    p
}

pub fn rcon_pass() -> String {
    crate::crypto::random_hex(6)
}

pub fn write_server_props(dir: &str, port: f64, rcon_port: f64, rcon_pass: &str, motd: &str) {
    let motd = if motd.is_empty() { "A craftbox server" } else { motd };
    let p = json::num_to_string(port);
    let props = [
        format!("motd={}", motd),
        format!("server-port={}", p),
        format!("query.port={}", p),
        "enable-rcon=true".into(),
        format!("rcon.port={}", json::num_to_string(rcon_port)),
        format!("rcon.password={}", rcon_pass),
        "online-mode=true".into(),
        "max-players=10".into(),
        "view-distance=8".into(),
        "simulation-distance=6".into(),
        "level-name=world".into(),
        "spawn-protection=0".into(),
        "enable-command-block=true".into(),
    ];
    let _ = std::fs::write(jsutil::path_join(&[dir, "server.properties"]), format!("{}\n", props.join("\n")));
}

fn write_exec(path: &str, content: &str) {
    let _ = std::fs::write(path, content);
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

/// Majors de JDK que o painel sabe usar, do mais velho pro mais novo.
const JAVA_MAJORS: [u32; 5] = [8, 11, 17, 21, 25];

/// Trecho de shell que escolhe a JDK na hora de ligar: a exata do MC
/// (`/usr/lib/jvm/java-N-openjdk`, caminho do Arch e do Fedora), senão a menor
/// instalada acima dela, senão `/usr/bin/java`. No Docker nenhuma dessas pastas
/// existe e o wrapper `/usr/bin/java` escolhe pela `CRAFTBOX_JAVA_MAJOR`.
fn java_pick_sh(mc: &str) -> String {
    let major = runner::java_for_mc(mc);
    let ladder: Vec<String> = JAVA_MAJORS.iter().filter(|v| **v >= major).map(|v| v.to_string()).collect();
    format!(
        "export CRAFTBOX_JAVA_MAJOR={}\nJAVA=/usr/bin/java\nfor v in {}; do\n  [ -x \"/usr/lib/jvm/java-$v-openjdk/bin/java\" ] && {{ JAVA=\"/usr/lib/jvm/java-$v-openjdk/bin/java\"; break; }}\ndone\n",
        major,
        ladder.join(" ")
    )
}

/// Mesma escolha do `java_pick_sh`, pros instaladores rodados pelo painel.
pub fn java_bin(mc: &str) -> String {
    let major = runner::java_for_mc(mc);
    JAVA_MAJORS
        .iter()
        .filter(|v| **v >= major)
        .map(|v| format!("/usr/lib/jvm/java-{}-openjdk/bin/java", v))
        .find(|p| std::path::Path::new(p).exists())
        .unwrap_or_else(|| "/usr/bin/java".into())
}

pub fn write_start_script(dir: &str, mc: &str) {
    let heap = json::num_to_string(heap_mb());
    let jvm = format!(
        "-Xms512M -Xmx{}M -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 \
         -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:+AlwaysPreTouch -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true",
        heap
    );
    let sh = format!(
        "#!/usr/bin/env bash\ncd \"$(dirname \"$0\")\"\n{}exec \"$JAVA\" {} -jar server.jar nogui\n",
        java_pick_sh(mc),
        jvm
    );
    write_exec(&jsutil::path_join(&[dir, "start.sh"]), &sh);
}

pub fn write_backup_script(dir: &str) {
    let sh = "#!/usr/bin/env bash
cd \"$(dirname \"$0\")\"
mkdir -p backups
TS=$(date +%Y%m%d-%H%M%S)
WORLD=$(grep -E '^level-name=' server.properties 2>/dev/null | cut -d= -f2)
WORLD=${WORLD:-world}
tar -czf \"backups/${WORLD}-${TS}.tar.gz\" \"$WORLD\" 2>/dev/null
ls -1t backups/*.tar.gz 2>/dev/null | tail -n +8 | xargs -r rm -f
";
    write_exec(&jsutil::path_join(&[dir, "backup.sh"]), sh);
}

pub fn write_forge_start(dir: &str, mc: &str) {
    let heap = json::num_to_string(heap_mb());
    let _ = std::fs::write(jsutil::path_join(&[dir, "user_jvm_args.txt"]), format!("-Xms512M\n-Xmx{}M\n", heap));
    let execline = match find_args_file(dir) {
        Some(rel) => format!("exec \"$JAVA\" @user_jvm_args.txt @{} nogui", rel),
        // o run.sh do Forge chama `java` do PATH: põe a JDK escolhida na frente
        None if std::path::Path::new(&jsutil::path_join(&[dir, "run.sh"])).exists() => {
            "export PATH=\"$(dirname \"$JAVA\"):$PATH\"\nexec bash run.sh nogui".into()
        }
        None => format!("exec \"$JAVA\" -Xmx{}M -jar server.jar nogui", heap),
    };
    write_exec(
        &jsutil::path_join(&[dir, "start.sh"]),
        &format!("#!/usr/bin/env bash\ncd \"$(dirname \"$0\")\"\n{}{}\n", java_pick_sh(mc), execline),
    );
}

pub fn unique_id(cfg: &Map, base: &str) -> String {
    let existing = ctx::list_instance_ids(cfg);
    let mut id = base.to_string();
    let mut n = 2;
    while existing.contains(&id) {
        id = format!("{}-{}", base, n);
        n += 1;
    }
    id
}

/// baixa e configura um servidor Pumpkin (Rust, binário único, sem Java) — experimental
fn install_pumpkin(dir: &str, port: f64, rcon_port: f64, rcon_pass: &str, motd: &str) -> Result<String, String> {
    let asset = if std::env::consts::ARCH == "aarch64" { "pumpkin-ARM64-Linux-musl" } else { "pumpkin-X64-Linux-musl" };
    let rel = net::get_json("https://api.github.com/repos/Pumpkin-MC/Pumpkin/releases/latest")?;
    let a = rel
        .get("assets")
        .and_then(|a| a.as_arr())
        .and_then(|a| a.iter().find(|x| x.get("name").and_then(|n| n.as_str()) == Some(asset)))
        .ok_or("binário do Pumpkin não encontrado pra esta arquitetura")?;
    let bin = jsutil::path_join(&[dir, "pumpkin"]);
    net::download(&jsutil::to_string(a.get("browser_download_url")), &bin)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
    let motd = if motd.is_empty() { "craftbox pumpkin".to_string() } else { motd.replace('"', "") };
    let toml = format!(
        "# config parcial — o Pumpkin preenche o resto com defaults\n[networking.java]\naddress = \"0.0.0.0:{}\"\nmotd = \"{}\"\nmax_players = 10\n\n[networking.rcon]\nenabled = true\naddress = \"0.0.0.0:{}\"\npassword = \"{}\"\n",
        json::num_to_string(port),
        motd,
        json::num_to_string(rcon_port),
        rcon_pass
    );
    let _ = std::fs::write(jsutil::path_join(&[dir, "pumpkin.toml"]), toml);
    let tag = jsutil::to_string_or_empty(rel.get("tag_name"));
    // /\+(\d+\.\d+(?:\.\d+)?)/
    if let Some(i) = tag.find('+') {
        let rest = &tag[i + 1..];
        let n = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b'.').count();
        let cand = rest[..n].trim_end_matches('.');
        let parts: Vec<&str> = cand.split('.').collect();
        if (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty()) {
            return Ok(cand.to_string());
        }
        if parts.len() > 3 {
            return Ok(parts[..3].join("."));
        }
    }
    Ok(if tag.is_empty() { "dev".into() } else { tag })
}

fn set_active_if_empty(st: &State, id: &str) {
    st.set_cfg(|c| {
        if !truthy(c.get("activeServer")) {
            c.insert("activeServer", Value::from(id));
            true
        } else {
            false
        }
    });
}

/// `createInstance({name, loader, version})`
pub fn create_instance(st: &State, cfg: &Map, name: &str, loader: &str, version: &str) -> Result<Value, String> {
    if !ctx::multi_enabled(cfg) {
        return Err("multi-servidor não está ativo (defina serversDir no config.json)".into());
    }
    let loader = if ["fabric", "forge", "neoforge", "pumpkin"].contains(&loader) { loader } else { "paper" };
    let id = unique_id(cfg, &jsutil::slugify_id(name));
    let dir = jsutil::path_join(&[&config::s(cfg, "serversDir"), &id]);
    let _ = std::fs::create_dir_all(&dir);
    let port = free_port(cfg);
    let rcon_port = port + 10.0;
    let rpass = rcon_pass();
    let disp = if name.is_empty() { id.clone() } else { name.to_string() };

    if loader == "pumpkin" {
        let resolved = install_pumpkin(&dir, port, rcon_port, &rpass, name).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            format!("falha ao baixar o Pumpkin: {}", e)
        })?;
        let _ = std::fs::write(jsutil::path_join(&[&dir, ".craftbox-loader"]), "pumpkin\n");
        write_exec(&jsutil::path_join(&[&dir, "start.sh"]), "#!/usr/bin/env bash\ncd \"$(dirname \"$0\")\"\nexec ./pumpkin\n");
        write_backup_script(&dir);
        let _ = std::fs::create_dir_all(jsutil::path_join(&[&dir, "plugins"]));
        let meta = obj! {
            "name" => disp.clone(), "loader" => "pumpkin", "mcVersion" => resolved.clone(), "port" => port,
            "rconPort" => rcon_port, "rconPassword" => rpass, "createdAt" => jsutil::now_ms(),
        };
        ctx::write_instance_meta(&dir, meta.as_obj().unwrap());
        set_active_if_empty(st, &id);
        return Ok(obj! { "id" => id, "name" => disp, "loader" => "pumpkin", "mcVersion" => resolved, "port" => port });
    }

    if loader == "forge" || loader == "neoforge" {
        progress_start(&disp);
        set_phase("Resolvendo versão…", &[]);
        let res = (|| -> Result<(String, String), String> {
            let (mc, lv) = if loader == "forge" { forge_resolve(Some(version))? } else { neoforge_resolve(Some(version))? };
            set_phase(
                &format!(
                    "Instalando o {} {} (MC {})… pode levar alguns minutos",
                    if loader == "forge" { "Forge" } else { "NeoForge" },
                    lv,
                    mc
                ),
                &[],
            );
            install_loader(&dir, loader, &mc, &lv)?;
            Ok((mc, lv))
        })();
        let (mc, lv) = match res {
            Ok(x) => x,
            Err(e) => {
                progress_clear();
                let _ = std::fs::remove_dir_all(&dir);
                return Err(format!("falha ao instalar o {}: {}", loader, e));
            }
        };
        let _ = std::fs::write(jsutil::path_join(&[&dir, "eula.txt"]), "eula=true\n");
        let _ = std::fs::write(jsutil::path_join(&[&dir, ".craftbox-loader"]), format!("{}\n", loader));
        write_server_props(&dir, port, rcon_port, &rpass, name);
        write_forge_start(&dir, &mc);
        write_backup_script(&dir);
        let _ = std::fs::create_dir_all(jsutil::path_join(&[&dir, "mods"]));
        let meta = obj! {
            "name" => disp.clone(), "loader" => loader, "mcVersion" => mc.clone(), "loaderVersion" => lv.clone(),
            "port" => port, "createdAt" => jsutil::now_ms(),
        };
        ctx::write_instance_meta(&dir, meta.as_obj().unwrap());
        set_active_if_empty(st, &id);
        progress_clear();
        return Ok(obj! { "id" => id, "name" => disp, "loader" => loader, "mcVersion" => mc, "loaderVersion" => lv, "port" => port });
    }

    let r = if loader == "paper" { paper_resolve(Some(version)) } else { fabric_resolve_server(Some(version)) }
        .and_then(|r| net::download(&r.url, &jsutil::path_join(&[&dir, "server.jar"])).map(|_| r.version));
    let resolved = match r {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("falha ao baixar o servidor: {}", e));
        }
    };
    let _ = std::fs::write(jsutil::path_join(&[&dir, "eula.txt"]), "eula=true\n");
    let _ = std::fs::write(jsutil::path_join(&[&dir, ".craftbox-loader"]), format!("{}\n", loader));
    write_server_props(&dir, port, rcon_port, &rpass, name);
    write_start_script(&dir, &resolved);
    write_backup_script(&dir);
    let _ = std::fs::create_dir_all(jsutil::path_join(&[&dir, if loader == "fabric" { "mods" } else { "plugins" }]));
    let meta = obj! { "name" => disp.clone(), "loader" => loader, "mcVersion" => resolved.clone(), "port" => port, "createdAt" => jsutil::now_ms() };
    ctx::write_instance_meta(&dir, meta.as_obj().unwrap());
    set_active_if_empty(st, &id);
    Ok(obj! { "id" => id, "name" => disp, "loader" => loader, "mcVersion" => resolved, "port" => port })
}

/// `fs.cpSync(src, dst, {recursive: true})`: copia arquivos, pastas e symlinks
/// (o link é recriado, não seguido), sobrescrevendo o que existir.
pub fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    if md.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        std::fs::set_permissions(dst, md.permissions())?;
    } else if md.file_type().is_symlink() {
        let target = std::fs::read_link(src)?;
        let _ = std::fs::remove_file(dst);
        std::os::unix::fs::symlink(target, dst)?;
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn clone_instance(cfg: &Map, src_id: &Value, name: Option<&Value>) -> Result<Value, String> {
    if !ctx::multi_enabled(cfg) {
        return Err("multi-servidor não está ativo".into());
    }
    let ids = ctx::list_instance_ids(cfg);
    let src_id_s = match src_id {
        Value::Str(s) if ids.contains(s) => s.clone(),
        _ => return Err("servidor de origem não existe".into()),
    };
    let sd = config::s(cfg, "serversDir");
    let src = jsutil::path_join(&[&sd, &src_id_s]);
    let base = if truthy(name) { jsutil::to_string(name) } else { format!("{}-copia", src_id_s) };
    let id = unique_id(cfg, &jsutil::slugify_id(&base));
    let dir = jsutil::path_join(&[&sd, &id]);
    copy_tree(std::path::Path::new(&src), std::path::Path::new(&dir))
        .map_err(|e| jsutil::fs_err(&e, "cp", &src))?;
    let _ = std::fs::remove_dir_all(jsutil::path_join(&[&dir, "logs"]));
    let mut meta = ctx::read_instance_meta(&dir);
    let port = free_port(cfg);
    let rcon_port = port + 10.0;
    let new_name = if truthy(name) {
        jsutil::to_string(name)
    } else {
        let old = if truthy(meta.get("name")) { jsutil::to_string(meta.get("name")) } else { src_id_s.clone() };
        format!("{} (cópia)", old)
    };
    meta.insert("name", Value::from(new_name.clone()));
    meta.insert("port", Value::Num(port));
    meta.insert("createdAt", Value::Num(jsutil::now_ms()));
    ctx::write_instance_meta(&dir, &meta);
    let mut up = Map::new();
    let p = json::num_to_string(port);
    up.insert("server-port", Value::from(p.clone()));
    up.insert("query.port", Value::from(p));
    up.insert("rcon.port", Value::from(json::num_to_string(rcon_port)));
    ctx::write_props(&dir, &up).map_err(|e| jsutil::fs_err(&e, "open", &jsutil::path_join(&[&dir, "server.properties"])))?;
    Ok(obj! { "id" => id, "name" => new_name, "loader" => meta.get("loader").cloned().unwrap_or(Value::Undef), "mcVersion" => meta.get("mcVersion").cloned().unwrap_or(Value::Undef), "port" => port })
}

/// `deleteInstance(id)` — só ids de instâncias existentes: '' ou '..'
/// resolveriam pra serversDir/pasta-mãe (rm -rf) — correção do bug B1.
pub fn delete_instance(st: &State, cfg: &Map, id: &str) -> Result<Value, String> {
    if !ctx::multi_enabled(cfg) {
        return Err("multi-servidor não está ativo".into());
    }
    if !ctx::list_instance_ids(cfg).iter().any(|i| i == id) {
        return Err("servidor não existe".into());
    }
    let dir = jsutil::path_join(&[&config::s(cfg, "serversDir"), id]);
    let service = format!("{}{}", jsutil::to_string(cfg.get("serviceTemplate")), id);
    let _ = runner::svc_action(st.exec_runner, cfg, "stop", &service);
    std::fs::remove_dir_all(&dir).map_err(|e| jsutil::fs_err(&e, "rm", &dir))?;
    st.set_cfg(|c| {
        if c.get("activeServer").and_then(|v| v.as_str()) == Some(id) {
            let first = ctx::list_instance_ids(c).into_iter().next().unwrap_or_default();
            c.insert("activeServer", Value::from(first));
            true
        } else {
            false
        }
    });
    Ok(obj! { "ok" => true })
}

/// `loaderOf(dir, metaLoader)`
fn loader_of(dir: &str, meta_loader: &Value) -> String {
    if truthy(Some(meta_loader)) {
        return jsutil::to_string(Some(meta_loader));
    }
    if let Ok(m) = std::fs::read_to_string(jsutil::path_join(&[dir, ".craftbox-loader"])) {
        let m = jsutil::trim(&m);
        if !m.is_empty() {
            return m.into();
        }
    }
    if std::path::Path::new(&jsutil::path_join(&[dir, "mods"])).exists() {
        return "fabric".into();
    }
    "paper".into()
}

pub fn list_servers_detailed(st: &State, cfg: &Map, active_id: &str) -> Value {
    let mut servers = Vec::new();
    for id in ctx::list_instance_ids(cfg) {
        let c: Srv = ctx::server_ctx(cfg, &id);
        let active = runner::svc_active_of(st.exec_runner, cfg, &c.service);
        let manual = ctx::read_instance_meta(&c.dir).get("manualMods").filter(|x| truthy(Some(x))).cloned();
        servers.push(obj! {
            "id" => id.clone(), "name" => c.name.clone(), "loader" => loader_of(&c.dir, &c.loader),
            "mcVersion" => c.mc_version.clone(), "port" => c.port.clone().unwrap_or(Value::Undef), "active" => active,
            "selected" => id == active_id, "modpack" => c.modpack.clone(),
            "manualMods" => manual.unwrap_or(Value::Arr(vec![])),
        });
    }
    obj! { "multi" => ctx::multi_enabled(cfg), "activeId" => active_id, "servers" => servers }
}

// ---------------------------------------------------------------------------
// Tamanho do mundo
// ---------------------------------------------------------------------------
fn dir_size_bytes(dir: &str) -> f64 {
    let mut total = 0.0;
    let mut stack = vec![dir.to_string()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            let fp = jsutil::path_join(&[&d, &e.file_name().to_string_lossy()]);
            if t.is_dir() {
                stack.push(fp);
            } else if t.is_file() {
                if let Ok(m) = std::fs::metadata(&fp) {
                    total += m.len() as f64;
                }
            }
        }
    }
    total
}

pub fn world_size(s: &Srv) -> Value {
    let props = ctx::read_props(&s.dir);
    let level = match props.get("level-name") {
        Some(v) if truthy(Some(v)) => jsutil::to_string(Some(v)),
        _ => "world".into(),
    };
    let names = [
        level.clone(),
        format!("{}_nether", level),
        format!("{}_the_end", level),
        "world".into(),
        "world_nether".into(),
        "world_the_end".into(),
    ];
    let mut dirs: Vec<String> = Vec::new();
    for n in names {
        let d = jsutil::path_join(&[&s.dir, &n]);
        if dirs.contains(&d) {
            continue;
        }
        if std::fs::metadata(&d).map(|m| m.is_dir()).unwrap_or(false) {
            dirs.push(d);
        }
    }
    let bytes: f64 = dirs.iter().map(|d| dir_size_bytes(d)).sum();
    obj! { "bytes" => bytes, "dirs" => dirs.iter().map(|d| Value::from(jsutil::path_basename(d))).collect::<Vec<_>>() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_and_neo_mapping() {
        let mut v = vec!["1.9", "1.21.11", "1.21", "26.2", "26.1.2"];
        v.sort_by(|a, b| cmp_ver(a, b));
        assert_eq!(v, ["1.9", "1.21", "1.21.11", "26.1.2", "26.2"]);
        assert_eq!(neo_prefix("1.21.1"), "21.1.");
        assert_eq!(neo_prefix("1.21"), "21.0.");
        assert_eq!(neo_prefix("26.2"), "26.2.0.");
        assert_eq!(neo_prefix("26.1.2"), "26.1.2.");
        assert_eq!(neo_to_mc("21.1.251"), "1.21.1");
        assert_eq!(neo_to_mc("21.0.167"), "1.21");
        assert_eq!(neo_to_mc("26.2.0.88"), "26.2");
        assert_eq!(neo_to_mc("26.1.2.5"), "26.1.2");
        assert!(is_plain_version("1.20.1") && !is_plain_version("1.20.1-pre") && !is_plain_version("26"));
    }

    #[test]
    fn copy_tree_keeps_structure() {
        let base = std::env::temp_dir().join(format!("cbrs-cp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/sub")).unwrap();
        std::fs::write(base.join("a/sub/f"), "x").unwrap();
        std::os::unix::fs::symlink("sub/f", base.join("a/link")).unwrap();
        copy_tree(&base.join("a"), &base.join("b")).unwrap();
        assert_eq!(std::fs::read_to_string(base.join("b/sub/f")).unwrap(), "x");
        assert_eq!(std::fs::read_link(base.join("b/link")).unwrap().to_str(), Some("sub/f"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
