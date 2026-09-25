//! Modpacks: `.mrpack` do Modrinth e packs de cliente do CurseForge (API v1,
//! chave gratuita), como no server.js: baixa o pack, extrai com `unzip`, baixa
//! os arquivos do lado servidor em paralelo (pool de 6), aplica os overrides,
//! desativa mods client-only (renomeia pra `.disabled`), instala o loader e
//! finaliza a instância. O progresso fica em `servers::PROGRESS`.

use crate::config;
use crate::content;
use crate::ctx;
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::net::{self, enc, HttpError};
use crate::obj;
use crate::runner;
use crate::servers::{self, progress_clear, progress_start, set_phase};
use crate::State;
use std::time::Duration;

fn tmp_dir(prefix: &str, id: &str) -> String {
    jsutil::path_join(&[&std::env::temp_dir().to_string_lossy(), &format!("{}{}", prefix, id)])
}

fn unzip_to(zip: &str, dest: &str) -> Result<(), String> {
    let r = runner::run("unzip", &["-o", "-q", zip, "-d", dest], Duration::from_secs(180));
    if r.code != 0 {
        let out = if r.stderr.is_empty() { &r.stdout } else { &r.stderr };
        return Err(format!("unzip falhou: {}", servers::tail_chars(out, 200)));
    }
    // alguns .mrpack guardam arquivos com modo 000 (perms preservadas) — sem isso o
    // próprio painel não consegue LER o modrinth.index.json nem os overrides (EACCES)
    runner::run("chmod", &["-R", "u+rwX,go+rX", dest], Duration::from_secs(60));
    Ok(())
}

/// Mods client-only conhecidos (renderização/UI/shaders): num servidor dedicado
/// eles tentam carregar classes de cliente e derrubam o boot.
const CLIENT_MOD_TOKENS: &[&str] = &[
    // shaders / renderização
    "oculus", "irisshaders", "iris-", "optifine", "embeddium", "rubidium", "sodium", "indium",
    "entity_texture_features", "entitytexturefeatures", "etf-", "entity_model_features", "entitymodelfeatures",
    "emf-", "citresewn", "continuity", "animatica", "fusion-", "connectedness", "chloride",
    // mapas / HUD / UI
    "xaero", "inventoryhud", "inventoryprofilesnext", "libipn", "armorchroma", "hide-key-binding",
    "hidekeybinding", "legendarytooltips", "controlling", "betterf3", "chat_heads", "chatheads", "chatanimation",
    "chattools", "movesubtitles", "advancementinfo", "better-selection", "i18nupdate", "jecharacters",
    "justenoughcharacters", "itemzoom", "justzoom", "imblocker", "clienttweaks", "durabilitytooltip",
    "enhancedvisuals", "fastscrolling", "fogoverrides", "forgeconfigscreens", "lcchatlogfilter", "sound-physics",
    "soundphysics",
    // skins / animação / câmera
    "3dskinlayers", "skinlayers", "customskinloader", "betterthirdperson", "notenoughanimations", "firstperson",
    "capes", "eatinganimation",
    // luzes / partículas / som (client)
    "dynamiclights", "lambdynamiclights", "ryoamiclights", "particlerain", "presencefootsteps", "soundphysics",
    "ambientsounds", "extrasounds", "visuality",
    // zoom / mouse / carregamento / diversos client
    "zoomify", "okzoomer", "ok-zoomer", "ok_zoomer", "zume", "mousetweaks", "drippyloadingscreen", "fancymenu",
    "konkrete", "entityculling", "cullleaves", "cull-less-leaves", "moreculling", "reeses", "immediatelyfast",
    "badoptimizations", "chunksfadein", "smoothscroll", "fast-ip-ping", "emiffect", "emienchants",
];

/// `fabric.mod.json`/`quilt.mod.json` com environment "client"
fn jar_env_client(jar: &str) -> bool {
    for meta in ["fabric.mod.json", "quilt.mod.json"] {
        let r = runner::run("unzip", &["-p", jar, meta], Duration::from_secs(15));
        if r.code != 0 || r.stdout.is_empty() {
            continue;
        }
        if let Ok(j) = json::parse(&r.stdout) {
            let env = if truthy(j.get("environment")) {
                j.get("environment").cloned()
            } else {
                j.get("quilt_loader").and_then(|q| q.get("minecraft")).and_then(|m| m.get("environment")).cloned()
            };
            if let Some(e) = env.filter(|e| truthy(Some(e))) {
                if jsutil::to_string(Some(&e)).to_lowercase() == "client" {
                    return true;
                }
            }
        }
    }
    false
}

/// Desativa (renomeia pra .disabled, sem apagar) os mods client-only de um servidor.
pub fn strip_client_mods(dir: &str) -> Vec<Value> {
    let mods = jsutil::path_join(&[dir, "mods"]);
    let Ok(rd) = std::fs::read_dir(&mods) else { return vec![] };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut disabled = Vec::new();
    for name in names {
        let low = name.to_lowercase();
        if !low.ends_with(".jar") {
            continue;
        }
        let path = jsutil::path_join(&[&mods, &name]);
        let client = CLIENT_MOD_TOKENS.iter().any(|t| low.contains(t)) || jar_env_client(&path);
        if client && std::fs::rename(&path, format!("{}.disabled", path)).is_ok() {
            disabled.push(Value::from(name));
        }
    }
    disabled
}

/// `runPool(items, 6, worker)`: rejeita no primeiro erro.
fn run_pool<T: Sync>(items: &[T], conc: usize, worker: impl Fn(&T) -> Result<(), String> + Sync) -> Result<(), String> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let first_err: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    std::thread::scope(|sc| {
        for _ in 0..conc.min(items.len()).max(1) {
            sc.spawn(|| loop {
                if first_err.lock().unwrap().is_some() {
                    return;
                }
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(it) = items.get(i) else { return };
                if let Err(e) = worker(it) {
                    first_err.lock().unwrap().get_or_insert(e);
                    return;
                }
            });
        }
    });
    match first_err.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Modrinth
// ---------------------------------------------------------------------------
pub fn modpack_search(query: &str, loader_filter: &str, offset_arg: &str) -> Result<Value, String> {
    let mut facets = vec!["[\"project_type:modpack\"]".to_string()];
    if !loader_filter.is_empty() {
        facets.push(format!("[\"categories:{}\"]", loader_filter));
    }
    let offset = jsutil::parse_int(offset_arg).filter(|n| *n != 0.0).unwrap_or(0.0).max(0.0);
    let url = format!(
        "https://api.modrinth.com/v2/search?limit=24&offset={}&index=relevance&query={}&facets={}",
        json::num_to_string(offset),
        enc(query),
        enc(&format!("[{}]", facets.join(",")))
    );
    let data = net::get_json(&url)?;
    let results: Vec<Value> = data
        .get("hits")
        .and_then(|h| h.as_arr())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|h| {
            obj! {
                "slug" => h.fld("slug"), "projectId" => h.fld("project_id"), "title" => h.fld("title"),
                "author" => h.fld("author"), "description" => h.fld("description"), "downloads" => h.fld("downloads"),
                "follows" => h.fld("follows"), "icon" => h.fld("icon_url"), "categories" => content::display_categories(h),
            }
        })
        .collect();
    let total = if truthy(data.get("total_hits")) { data.fld("total_hits") } else { Value::Num(0.0) };
    Ok(obj! { "results" => results, "total" => total, "offset" => offset, "limit" => 24 })
}

pub fn modpack_versions(slug: &str) -> Result<Vec<Value>, String> {
    let vers = net::get_json(&format!("https://api.modrinth.com/v2/project/{}/version", enc(slug)))?;
    Ok(vers
        .as_arr()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|v| {
            let files = v.get("files").and_then(|f| f.as_arr()).cloned().unwrap_or_default();
            let f = files.iter().find(|x| truthy(x.get("primary"))).or_else(|| files.first());
            let or_arr = |k: &str| if truthy(v.get(k)) { v.fld(k) } else { Value::Arr(vec![]) };
            obj! {
                "id" => v.fld("id"), "name" => v.fld("name"), "versionNumber" => v.fld("version_number"),
                "gameVersions" => or_arr("game_versions"), "loaders" => or_arr("loaders"),
                "datePublished" => v.fld("date_published"), "versionType" => v.fld("version_type"),
                "url" => f.map(|f| f.fld("url")).unwrap_or(Value::Null),
            }
        })
        .collect())
}

/// `pickModpackVersion(versions, versionId)`
fn pick_modpack_version(versions: &[Value], version_id: Option<&str>) -> Option<Value> {
    if let Some(id) = version_id.filter(|s| !s.is_empty()) {
        if let Some(v) = versions.iter().find(|x| x.get("id").and_then(|i| i.as_str()) == Some(id)) {
            return Some(v.clone());
        }
    }
    versions
        .iter()
        .find(|v| v.get("versionType").and_then(|t| t.as_str()) == Some("release"))
        .or_else(|| versions.first())
        .cloned()
}

fn cleanup(dir: &str, tmp: &str) {
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(tmp);
}

pub fn create_from_modpack(st: &State, cfg: &Map, name: &str, slug: &str, version_id: Option<&str>) -> Result<Value, String> {
    if !ctx::multi_enabled(cfg) {
        return Err("multi-servidor não está ativo".into());
    }
    progress_start(if name.is_empty() { slug } else { name });
    let versions = modpack_versions(slug).inspect_err(|_| progress_clear())?;
    let v = match pick_modpack_version(&versions, version_id) {
        Some(v) if truthy(v.get("url")) => v,
        _ => {
            progress_clear();
            return Err("versão do modpack não encontrada".into());
        }
    };
    let id = servers::unique_id(cfg, &jsutil::slugify_id(if name.is_empty() { slug } else { name }));
    let dir = jsutil::path_join(&[&config::s(cfg, "serversDir"), &id]);
    let tmp = tmp_dir("craftbox-mrpack-", &id);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(&tmp);
    let res = (|| -> Result<(String, String, String, Vec<Value>), String> {
        set_phase("Baixando o modpack…", &[]);
        let pack = jsutil::path_join(&[&tmp, "pack.mrpack"]);
        net::download(&jsutil::to_string(v.get("url")), &pack)?;
        unzip_to(&pack, &tmp)?;
        let idx_path = jsutil::path_join(&[&tmp, "modrinth.index.json"]);
        let idx_txt = std::fs::read_to_string(&idx_path).map_err(|e| jsutil::fs_err(&e, "open", &idx_path))?;
        let idx = json::parse(&idx_txt).map_err(|e| e.0)?;
        let deps = idx.get("dependencies").cloned().unwrap_or(obj! {});
        let mc = jsutil::to_string_or_empty(deps.get("minecraft"));
        let (loader, lv) = [("fabric-loader", "fabric"), ("quilt-loader", "quilt"), ("forge", "forge"), ("neoforge", "neoforge")]
            .iter()
            .find(|(k, _)| truthy(deps.get(k)))
            .map(|(k, l)| (l.to_string(), jsutil::to_string(deps.get(k))))
            .ok_or("loader do modpack não reconhecido")?;
        // arquivos do lado servidor (baixados em paralelo — pool de 6)
        let mut jobs: Vec<(String, String)> = Vec::new();
        for f in idx.get("files").and_then(|f| f.as_arr()).cloned().unwrap_or_default() {
            if f.get("env").and_then(|e| e.get("server")).and_then(|s| s.as_str()) == Some("unsupported") {
                continue;
            }
            let rel = jsutil::to_string_or_empty(f.get("path")).replace('\\', "/");
            if rel.is_empty() || rel.contains("..") || rel.starts_with('/') {
                continue;
            }
            let Some(dl) = f.get("downloads").and_then(|d| d.as_arr()).and_then(|a| a.first()) else { continue };
            if !truthy(Some(dl)) {
                continue;
            }
            let dest = jsutil::path_join(&[&dir, &rel]);
            let _ = std::fs::create_dir_all(jsutil::path_dirname(&dest));
            jobs.push((jsutil::to_string(Some(dl)), dest));
        }
        set_phase("Baixando os mods…", &[("total", Value::from(jobs.len())), ("done", Value::Num(0.0))]);
        run_pool(&jobs, 6, |(u, d)| {
            net::download(u, d)?;
            servers::progress_inc_done();
            Ok(())
        })?;
        // overrides (server-overrides tem prioridade)
        set_phase("Aplicando arquivos do pack…", &[]);
        for ov in ["overrides", "server-overrides"] {
            let src = jsutil::path_join(&[&tmp, ov]);
            if std::path::Path::new(&src).exists() {
                servers::copy_tree(std::path::Path::new(&src), std::path::Path::new(&dir)).map_err(|e| e.to_string())?;
            }
        }
        set_phase("Desativando mods client-only…", &[]);
        let stripped = strip_client_mods(&dir);
        set_phase(&format!("Instalando o {}… (pode demorar)", loader), &[]);
        servers::install_loader(&dir, &loader, &mc, &lv)?;
        Ok((loader, mc, lv, stripped))
    })();
    let (loader, mc, lv, stripped) = match res {
        Ok(x) => x,
        Err(e) => {
            progress_clear();
            cleanup(&dir, &tmp);
            return Err(format!("falha ao instalar o modpack: {}", e));
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let proj = content::modrinth_project(slug).ok();
    let title = proj.as_ref().map(|p| p.fld("title")).unwrap_or(Value::from(slug));
    let modpack = obj! {
        "source" => "modrinth", "slug" => slug, "versionId" => v.fld("id"), "version" => v.fld("versionNumber"),
        "name" => title.clone(), "url" => format!("https://modrinth.com/modpack/{}", slug),
    };
    let disp = if name.is_empty() { title } else { Value::from(name) };
    Ok(finish_pack_instance(st, cfg, &id, &dir, disp, &loader, &mc, &lv, modpack, stripped, vec![]))
}

/// parte comum a todo modpack (Modrinth/CurseForge): eula, portas, scripts e metadados
#[allow(clippy::too_many_arguments)]
fn finish_pack_instance(
    st: &State,
    cfg: &Map,
    id: &str,
    dir: &str,
    name: Value,
    loader: &str,
    mc: &str,
    lv: &str,
    modpack: Value,
    stripped: Vec<Value>,
    manual: Vec<Value>,
) -> Value {
    set_phase("Finalizando…", &[]);
    let port = servers::free_port(cfg);
    let rcon_port = port + 10.0;
    let rpass = servers::rcon_pass();
    if !std::path::Path::new(&jsutil::path_join(&[dir, "eula.txt"])).exists() {
        let _ = std::fs::write(jsutil::path_join(&[dir, "eula.txt"]), "eula=true\n");
    }
    let _ = std::fs::write(jsutil::path_join(&[dir, ".craftbox-loader"]), format!("{}\n", loader));
    if !std::path::Path::new(&jsutil::path_join(&[dir, "server.properties"])).exists() {
        servers::write_server_props(dir, port, rcon_port, &rpass, &jsutil::to_string_or_empty(Some(&name)));
    } else {
        let p = json::num_to_string(port);
        let mut up = Map::new();
        up.insert("server-port", Value::from(p.clone()));
        up.insert("query.port", Value::from(p));
        up.insert("enable-rcon", Value::from("true"));
        up.insert("rcon.port", Value::from(json::num_to_string(rcon_port)));
        up.insert("rcon.password", Value::from(rpass));
        let _ = ctx::write_props(dir, &up);
    }
    if loader == "forge" || loader == "neoforge" {
        servers::write_forge_start(dir, mc);
    } else {
        servers::write_start_script(dir, mc);
    }
    servers::write_backup_script(dir);
    let _ = std::fs::create_dir_all(jsutil::path_join(&[dir, "mods"]));
    let mut meta = Map::new();
    meta.insert("name", name.clone());
    meta.insert("loader", Value::from(loader));
    meta.insert("mcVersion", Value::from(mc));
    meta.insert("loaderVersion", Value::from(lv));
    meta.insert("port", Value::Num(port));
    meta.insert("createdAt", Value::Num(jsutil::now_ms()));
    meta.insert("modpack", modpack.clone());
    meta.insert("strippedMods", Value::Arr(stripped.clone()));
    if !manual.is_empty() {
        meta.insert("manualMods", Value::Arr(manual.clone()));
    }
    ctx::write_instance_meta(dir, &meta);
    st.set_cfg(|c| {
        if !truthy(c.get("activeServer")) {
            c.insert("activeServer", Value::from(id));
            true
        } else {
            false
        }
    });
    progress_clear();
    obj! {
        "id" => id, "name" => name, "loader" => loader, "mcVersion" => mc, "port" => port, "modpack" => modpack,
        "strippedMods" => stripped, "manualMods" => manual,
    }
}

/// `manualModUpload(dir, name, data)`: recebe um dos mods que o autor bloqueou
/// pra download automático. Casa pelo SHA-1 (o nome do arquivo baixado pode
/// vir com "(1)" etc.); sem hash registrado, casa pelo nome. Grava na pasta
/// esperada e tira da lista `manualMods` do meta. Retorna (arquivo, restantes).
pub fn manual_mod_upload(dir: &str, name: &str, data: &[u8]) -> Result<(String, Vec<Value>), (u16, String)> {
    let mut meta = ctx::read_instance_meta(dir);
    let list = meta.get("manualMods").and_then(|v| v.as_arr()).cloned().unwrap_or_default();
    if list.is_empty() {
        return Err((400, "esse servidor não tem mods pendentes".into()));
    }
    if data.is_empty() {
        return Err((400, "arquivo vazio".into()));
    }
    let hash = crate::crypto::hex(ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, data).as_ref());
    let base = jsutil::path_basename(name);
    let s = |e: &Value, k: &str| jsutil::to_string_or_empty(e.get(k));
    let idx = match list.iter().position(|e| s(e, "sha1") == hash) {
        Some(i) => i,
        None => match list.iter().position(|e| s(e, "file") == base) {
            Some(i) if s(&list[i], "sha1").is_empty() => i,
            Some(_) => return Err((400, format!("{}: o conteúdo não bate com a versão que o modpack pede (baixe pelo link da lista)", base))),
            None => return Err((400, format!("{} não é nenhum dos mods pendentes deste servidor", base))),
        },
    };
    let e = &list[idx];
    let (file, folder) = (s(e, "file"), s(e, "folder"));
    if file.is_empty() || file.contains('/') || file.contains("..") || folder.contains("..") || folder.starts_with('/') {
        return Err((400, "entrada inválida no meta".into()));
    }
    let dest_dir = jsutil::path_join(&[dir, &folder]);
    std::fs::create_dir_all(&dest_dir).map_err(|e| (500, e.to_string()))?;
    let dest = jsutil::path_join(&[&dest_dir, &file]);
    let tmp = format!("{}.part", dest);
    std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &dest)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        (500, e.to_string())
    })?;
    let rest: Vec<Value> = list.iter().enumerate().filter(|(i, _)| *i != idx).map(|(_, v)| v.clone()).collect();
    if rest.is_empty() {
        meta.remove("manualMods");
    } else {
        meta.insert("manualMods", Value::Arr(rest.clone()));
    }
    ctx::write_instance_meta(dir, &meta);
    Ok((file, rest))
}

// ---------------------------------------------------------------------------
// CurseForge (API v1 — exige chave gratuita do console.curseforge.com)
// ---------------------------------------------------------------------------
const CF_API: &str = "https://api.curseforge.com/v1";
pub const CF_GAME: u32 = 432;
const CF_CLASS_MODPACK: u32 = 4471;
const CF_CLASS_RESOURCEPACK: f64 = 12.0;
const CF_CLASS_SHADER: f64 = 6552.0;
const CF_CLASS_DATAPACK: f64 = 6945.0;

pub fn cf_key(cfg: &Map) -> String {
    jsutil::trim(&config::s(cfg, "curseforgeApiKey")).to_string()
}

/// Erro de chave (ausente/inválida): vira 400 + `needKey:true` nas rotas.
pub struct CfErr {
    pub msg: String,
    pub need_key: bool,
}

impl From<String> for CfErr {
    fn from(msg: String) -> Self {
        CfErr { msg, need_key: false }
    }
}
impl From<&str> for CfErr {
    fn from(msg: &str) -> Self {
        CfErr { msg: msg.into(), need_key: false }
    }
}

pub fn cf(cfg: &Map, method: &str, pathq: &str, body: Option<&Value>, key: Option<&str>) -> Result<Value, CfErr> {
    let k = key.map(String::from).unwrap_or_else(|| cf_key(cfg));
    if k.is_empty() {
        return Err(CfErr { msg: "configure a chave da API do CurseForge".into(), need_key: true });
    }
    net::req_json(method, &format!("{}{}", CF_API, pathq), &[("x-api-key", &k)], body).map_err(|e: HttpError| {
        if matches!(e.status, Some(401 | 403)) {
            CfErr { msg: "chave da API do CurseForge inválida".into(), need_key: true }
        } else {
            CfErr { msg: e.msg, need_key: false }
        }
    })
}

fn cf_loader_type(l: &str) -> Option<u32> {
    match l {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

pub fn cf_modpack_search(cfg: &Map, query: &str, loader: &str, offset_arg: &str) -> Result<Value, CfErr> {
    let limit: f64 = 24.0;
    // a API só pagina até index+pageSize <= 10000
    let offset = (10000.0_f64 - limit).min(jsutil::parse_int(offset_arg).filter(|n| *n != 0.0).unwrap_or(0.0).max(0.0));
    let mut q = format!(
        "/mods/search?gameId={}&classId={}&sortField=2&sortOrder=desc&pageSize=24&index={}",
        CF_GAME,
        CF_CLASS_MODPACK,
        json::num_to_string(offset)
    );
    if !query.is_empty() {
        q.push_str(&format!("&searchFilter={}", enc(query)));
    }
    if let Some(t) = cf_loader_type(loader) {
        q.push_str(&format!("&modLoaderType={}", t));
    }
    let r = cf(cfg, "GET", &q, None, None)?;
    let results: Vec<Value> = r
        .get("data")
        .and_then(|d| d.as_arr())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let author = m.get("authors").and_then(|a| a.as_arr()).and_then(|a| a.first()).and_then(|a| a.get("name"));
            let icon = m.get("logo").filter(|l| truthy(Some(l))).map(|l| {
                if truthy(l.get("thumbnailUrl")) { l.fld("thumbnailUrl") } else { l.fld("url") }
            });
            obj! {
                "source" => "curseforge", "id" => jsutil::to_string(m.get("id")), "slug" => m.fld("slug"),
                "title" => m.fld("name"),
                "author" => if truthy(author) { author.cloned().unwrap() } else { Value::from("") },
                "description" => m.fld("summary"), "downloads" => m.fld("downloadCount"),
                "icon" => icon.unwrap_or(Value::Undef),
                "categories" => Value::Arr(m.get("categories").and_then(|c| c.as_arr()).cloned().unwrap_or_default()
                    .iter().map(|c| c.fld("name")).collect()),
                "url" => m.get("links").map(|l| l.fld("websiteUrl")).unwrap_or(Value::Undef),
            }
        })
        .collect();
    let total = r.get("pagination").and_then(|p| p.get("totalCount")).and_then(|t| t.as_f64()).filter(|n| *n != 0.0).unwrap_or(0.0);
    Ok(obj! { "results" => results, "total" => total.min(10000.0), "offset" => offset, "limit" => limit })
}

fn cf_file_version(f: &Value) -> Value {
    let gv: Vec<Value> = f.get("gameVersions").and_then(|g| g.as_arr()).cloned().unwrap_or_default();
    let is_num = |x: &Value| x.as_str().is_some_and(|s| s.bytes().next().is_some_and(|b| b.is_ascii_digit()));
    let rel = match f.get("releaseType").and_then(|r| r.as_f64()) {
        Some(2.0) => "beta",
        Some(3.0) => "alpha",
        _ => "release",
    };
    obj! {
        "id" => jsutil::to_string(f.get("id")), "name" => f.fld("displayName"),
        "versionNumber" => if truthy(f.get("displayName")) { f.fld("displayName") } else { f.fld("fileName") },
        "gameVersions" => gv.iter().filter(|x| is_num(x)).cloned().collect::<Vec<_>>(),
        "loaders" => gv.iter().filter(|x| !is_num(x)).filter(|x| {
            let s = jsutil::to_string(Some(x)).to_lowercase(); s != "client" && s != "server"
        }).map(|x| Value::from(jsutil::to_string(Some(x)).to_lowercase())).collect::<Vec<_>>(),
        "datePublished" => f.fld("fileDate"), "versionType" => rel, "url" => f.fld("downloadUrl"),
        "serverPackFileId" => if truthy(f.get("serverPackFileId")) { f.fld("serverPackFileId") } else { Value::Null },
    }
}

pub fn cf_modpack_versions(cfg: &Map, project_id: &str) -> Result<Vec<Value>, CfErr> {
    let r = cf(cfg, "GET", &format!("/mods/{}/files?pageSize=50", enc(project_id)), None, None)?;
    let mut files: Vec<Value> = r
        .get("data")
        .and_then(|d| d.as_arr())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !truthy(f.get("isServerPack")))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(jsutil::to_string(f.get("fileDate"))));
    Ok(files.iter().map(cf_file_version).collect())
}

/// arquivos cujo autor desligou o download por terceiros vêm com downloadUrl=null:
/// tenta achar o MESMO arquivo (mesmo sha1) no Modrinth — é o que o Prism faz.
fn modrinth_by_sha1(hashes: &[String]) -> Vec<(String, String)> {
    if hashes.is_empty() {
        return vec![];
    }
    let body = obj! { "hashes" => hashes.iter().map(|h| Value::from(h.as_str())).collect::<Vec<_>>(), "algorithm" => "sha1" };
    let Ok(r) = net::req_json("POST", "https://api.modrinth.com/v2/version_files", &[], Some(&body)) else { return vec![] };
    let mut out = Vec::new();
    if let Value::Obj(m) = r {
        for (h, v) in m.iter() {
            let files = v.get("files").and_then(|f| f.as_arr()).cloned().unwrap_or_default();
            let f = files
                .iter()
                .find(|x| x.get("hashes").and_then(|hh| hh.get("sha1")).and_then(|s| s.as_str()) == Some(h))
                .or_else(|| files.first());
            if let Some(u) = f.and_then(|f| f.get("url")).filter(|u| truthy(Some(u))) {
                out.push((h.clone(), jsutil::to_string(Some(u))));
            }
        }
    }
    out
}

/// (loader, mc, versão do loader, mods desativados, mods pra baixar à mão)
type CfInstalled = (String, String, String, Vec<Value>, Vec<Value>);

struct CfJob {
    url: Option<String>,
    dest: String,
    sha1: Option<String>,
    file_name: String,
    page: String,
}

pub fn create_from_curseforge(st: &State, cfg: &Map, name: &str, project_id: &str, file_id: Option<&str>) -> Result<Value, CfErr> {
    if !ctx::multi_enabled(cfg) {
        return Err("multi-servidor não está ativo".into());
    }
    progress_start(if name.is_empty() { "modpack" } else { name });
    let fetched = (|| -> Result<(Value, Option<Value>), CfErr> {
        let project = cf(cfg, "GET", &format!("/mods/{}", enc(project_id)), None, None)?.fld("data");
        let pf = match file_id.filter(|f| !f.is_empty()) {
            Some(fid) => Some(cf(cfg, "GET", &format!("/mods/{}/files/{}", enc(project_id), enc(fid)), None, None)?.fld("data")),
            None => {
                let vs = cf_modpack_versions(cfg, project_id)?;
                match pick_modpack_version(&vs, None) {
                    Some(v) => Some(
                        cf(cfg, "GET", &format!("/mods/{}/files/{}", enc(project_id), jsutil::to_string(v.get("id"))), None, None)?
                            .fld("data"),
                    ),
                    None => None,
                }
            }
        };
        Ok((project, pf))
    })();
    let (project, pack_file) = match fetched {
        Ok(x) => x,
        Err(e) => {
            progress_clear();
            return Err(e);
        }
    };
    let Some(pack_file) = pack_file.filter(|p| truthy(Some(p))) else {
        progress_clear();
        return Err("versão do modpack não encontrada".into());
    };
    if !truthy(pack_file.get("downloadUrl")) {
        progress_clear();
        return Err("o autor deste modpack bloqueou downloads fora do app do CurseForge".into());
    }
    let title = if truthy(project.get("name")) { jsutil::to_string(project.get("name")) } else { project_id.to_string() };
    servers::progress_set("name", Value::from(if name.is_empty() { title.clone() } else { name.to_string() }));
    let slug = jsutil::to_string_or_empty(project.get("slug"));
    let base = if !name.is_empty() {
        name.to_string()
    } else if !slug.is_empty() {
        slug.clone()
    } else {
        title.clone()
    };
    let id = servers::unique_id(cfg, &jsutil::slugify_id(&base));
    let dir = jsutil::path_join(&[&config::s(cfg, "serversDir"), &id]);
    let tmp = tmp_dir("craftbox-cfpack-", &id);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(&tmp);
    let res = (|| -> Result<CfInstalled, CfErr> {
        set_phase("Baixando o modpack…", &[]);
        let zip = jsutil::path_join(&[&tmp, "pack.zip"]);
        net::download(&jsutil::to_string(pack_file.get("downloadUrl")), &zip)?;
        unzip_to(&zip, &tmp)?;
        let manifest = std::fs::read_to_string(jsutil::path_join(&[&tmp, "manifest.json"]))
            .ok()
            .and_then(|t| json::parse(&t).ok())
            .ok_or("manifest.json não encontrado — não é um modpack de cliente do CurseForge")?;
        let mcv = manifest.get("minecraft").cloned().unwrap_or(Value::Null);
        let mc = jsutil::to_string_or_empty(mcv.get("version"));
        let mls = mcv.get("modLoaders").and_then(|m| m.as_arr()).cloned().unwrap_or_default();
        let ml = mls
            .iter()
            .find(|x| truthy(x.get("primary")))
            .or_else(|| mls.first())
            .map(|x| jsutil::to_string_or_empty(x.get("id")))
            .unwrap_or_default();
        let (loader, lv) = match ml.find('-') {
            Some(i) => (ml[..i].to_lowercase(), ml[i + 1..].to_string()),
            None => (String::new(), String::new()),
        };
        if !["forge", "neoforge", "fabric", "quilt"].contains(&loader.as_str()) || lv.is_empty() {
            return Err(format!("loader do modpack não reconhecido: {}", if ml.is_empty() { "?" } else { &ml }).into());
        }
        if mc.is_empty() {
            return Err("modpack sem versão do Minecraft no manifest".into());
        }
        // resolve os arquivos: metadados dos arquivos + dos projetos (pra saber a classe)
        set_phase("Resolvendo a lista de mods…", &[]);
        let entries: Vec<Value> = manifest
            .get("files")
            .and_then(|f| f.as_arr())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.get("required") != Some(&Value::Bool(false)) && truthy(f.get("fileID")) && truthy(f.get("projectID")))
            .collect();
        let mut file_info: Vec<(f64, Value)> = Vec::new();
        let mut proj_info: Vec<(f64, Value)> = Vec::new();
        for chunk in entries.chunks(500) {
            let fids: Vec<Value> = chunk.iter().map(|f| f.fld("fileID")).collect();
            let mut pids: Vec<Value> = Vec::new();
            for f in chunk {
                let p = f.fld("projectID");
                if !pids.contains(&p) {
                    pids.push(p);
                }
            }
            let fr = cf(cfg, "POST", "/mods/files", Some(&obj! { "fileIds" => fids }), None)?;
            let pr = cf(cfg, "POST", "/mods", Some(&obj! { "modIds" => pids, "filterPcOnly" => true }), None)?;
            for f in fr.get("data").and_then(|d| d.as_arr()).cloned().unwrap_or_default() {
                if let Some(i) = f.get("id").and_then(|x| x.as_f64()) {
                    file_info.push((i, f));
                }
            }
            for p in pr.get("data").and_then(|d| d.as_arr()).cloned().unwrap_or_default() {
                if let Some(i) = p.get("id").and_then(|x| x.as_f64()) {
                    proj_info.push((i, p));
                }
            }
        }
        let mut jobs: Vec<CfJob> = Vec::new();
        let mut blocked: Vec<CfJob> = Vec::new();
        for e in &entries {
            let fid = e.get("fileID").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
            let pid = e.get("projectID").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
            let Some(f) = file_info.iter().find(|x| x.0 == fid).map(|x| &x.1) else { continue };
            let p = proj_info.iter().find(|x| x.0 == pid).map(|x| x.1.clone()).unwrap_or(obj! {});
            let fname = jsutil::to_string_or_empty(f.get("fileName"));
            if fname.is_empty() || fname.contains('/') || fname.contains('\\') || fname.contains("..") {
                continue;
            }
            let class = p.get("classId").and_then(|c| c.as_f64());
            // resource packs e shaders só servem pro cliente
            if class == Some(CF_CLASS_RESOURCEPACK) || class == Some(CF_CLASS_SHADER) {
                continue;
            }
            let folder = if class == Some(CF_CLASS_DATAPACK) { "world/datapacks" } else { "mods" };
            let sha1 = f
                .get("hashes")
                .and_then(|h| h.as_arr())
                .and_then(|a| a.iter().find(|h| h.get("algo").and_then(|x| x.as_f64()) == Some(1.0)))
                .and_then(|h| h.get("value"))
                .map(|v| jsutil::to_string(Some(v)));
            let site = p.get("links").and_then(|l| l.get("websiteUrl")).filter(|u| truthy(Some(u))).map(|u| jsutil::to_string(Some(u)));
            let page = format!(
                "{}/files/{}",
                site.unwrap_or_else(|| format!(
                    "https://www.curseforge.com/minecraft/mc-mods/{}",
                    if truthy(p.get("slug")) { jsutil::to_string(p.get("slug")) } else { json::num_to_string(pid) }
                )),
                json::num_to_string(fid)
            );
            let job = CfJob {
                url: f.get("downloadUrl").filter(|u| truthy(Some(u))).map(|u| jsutil::to_string(Some(u))),
                dest: jsutil::path_join(&[&dir, folder, &fname]),
                sha1,
                file_name: fname,
                page,
            };
            if job.url.is_some() {
                jobs.push(job);
            } else {
                blocked.push(job);
            }
        }
        let mut manual: Vec<Value> = Vec::new();
        if !blocked.is_empty() {
            let hashes: Vec<String> = blocked.iter().filter_map(|b| b.sha1.clone()).collect();
            let found = modrinth_by_sha1(&hashes);
            for mut b in blocked {
                match b.sha1.as_ref().and_then(|h| found.iter().find(|(fh, _)| fh == h)) {
                    Some((_, u)) => {
                        b.url = Some(u.clone());
                        jobs.push(b);
                    }
                    None => {
                        let folder = jsutil::path_dirname(&b.dest);
                        let rel = folder.strip_prefix(&format!("{}/", dir)).unwrap_or(&folder).to_string();
                        let mut e = obj! { "file" => b.file_name, "folder" => rel, "url" => b.page };
                        if let (Value::Obj(m), Some(h)) = (&mut e, b.sha1) {
                            m.insert("sha1", Value::from(h));
                        }
                        manual.push(e);
                    }
                }
            }
        }
        set_phase("Baixando os mods…", &[("total", Value::from(jobs.len())), ("done", Value::Num(0.0))]);
        run_pool(&jobs, 6, |j| {
            let _ = std::fs::create_dir_all(jsutil::path_dirname(&j.dest));
            net::download(j.url.as_deref().unwrap_or(""), &j.dest)?;
            servers::progress_inc_done();
            Ok(())
        })?;
        set_phase("Aplicando arquivos do pack…", &[]);
        let ov_name = if truthy(manifest.get("overrides")) { jsutil::to_string(manifest.get("overrides")) } else { "overrides".into() };
        let ov = jsutil::path_join(&[&tmp, &ov_name.replace("..", "")]);
        if std::path::Path::new(&ov).exists() {
            servers::copy_tree(std::path::Path::new(&ov), std::path::Path::new(&dir)).map_err(|e| e.to_string())?;
        }
        set_phase("Desativando mods client-only…", &[]);
        let stripped = strip_client_mods(&dir);
        set_phase(&format!("Instalando o {} {}… (pode demorar)", loader, lv), &[]);
        servers::install_loader(&dir, &loader, &mc, &lv)?;
        Ok((loader, mc, lv, stripped, manual))
    })();
    let (loader, mc, lv, stripped, manual) = match res {
        Ok(x) => x,
        Err(e) => {
            progress_clear();
            cleanup(&dir, &tmp);
            return Err(CfErr { msg: format!("falha ao instalar o modpack: {}", e.msg), need_key: e.need_key });
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let url = project
        .get("links")
        .and_then(|l| l.get("websiteUrl"))
        .filter(|u| truthy(Some(u)))
        .map(|u| jsutil::to_string(Some(u)))
        .unwrap_or_else(|| format!("https://www.curseforge.com/minecraft/modpacks/{}", slug));
    let modpack = obj! {
        "source" => "curseforge", "projectId" => project_id, "slug" => project.fld("slug"),
        "versionId" => jsutil::to_string(pack_file.get("id")),
        "version" => if truthy(pack_file.get("displayName")) { pack_file.fld("displayName") } else { pack_file.fld("fileName") },
        "name" => title.clone(), "url" => url,
    };
    let disp = Value::from(if name.is_empty() { title } else { name.to_string() });
    Ok(finish_pack_instance(st, cfg, &id, &dir, disp, &loader, &mc, &lv, modpack, stripped, manual))
}

/// valida a chave antes de salvar (chamada barata: dados do jogo Minecraft)
pub fn cf_validate_key(cfg: &Map, key: &str) -> Result<(), CfErr> {
    cf(cfg, "GET", &format!("/games/{}", CF_GAME), None, Some(key)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_stops_on_error() {
        let items: Vec<u32> = (0..50).collect();
        let n = std::sync::atomic::AtomicUsize::new(0);
        let r = run_pool(&items, 6, |i| {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if *i == 3 {
                Err("x".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(r, Err("x".to_string()));
        assert!(n.load(std::sync::atomic::Ordering::SeqCst) < 50);
        assert!(run_pool(&items, 6, |_| Ok(())).is_ok());
    }

    #[test]
    fn cf_file_version_shape() {
        let f = json::parse(r#"{"id":7,"displayName":"Pack 1.0","fileName":"p.zip","gameVersions":["1.20.1","Forge","Client"],"releaseType":2,"fileDate":"2024","downloadUrl":null}"#).unwrap();
        assert_eq!(
            json::stringify(&cf_file_version(&f)),
            r#"{"id":"7","name":"Pack 1.0","versionNumber":"Pack 1.0","gameVersions":["1.20.1"],"loaders":["forge"],"datePublished":"2024","versionType":"beta","url":null,"serverPackFileId":null}"#
        );
    }
}
