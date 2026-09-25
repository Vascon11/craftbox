//! Conteúdo da instância (mods/plugins via Modrinth), como no server.js:
//! detecção de loader/versão, busca, versões, projeto, instalação com
//! dependências obrigatórias (até 3 níveis), manifesto `.craftbox-content.json`,
//! lista de instalados (manifesto × arquivos reais), atualizações, Bedrock
//! (Geyser/Floodgate) e login de jogadores (EasyAuth/AuthMe + offline).

use crate::config;
use crate::ctx::{self, Srv};
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::net::{self, enc};
use crate::obj;

pub const SORTS: [&str; 5] = ["relevance", "downloads", "follows", "newest", "updated"];
const MODS_LIKE: [&str; 4] = ["fabric", "quilt", "forge", "neoforge"];

/// nomes de "categorias" que na verdade são loaders/ambiente — escondidos na UI
const LOADER_TAGS: [&str; 16] = [
    "fabric", "quilt", "forge", "neoforge", "paper", "purpur", "spigot", "bukkit", "folia", "velocity", "sponge",
    "bungeecord", "waterfall", "datapack", "client", "server",
];

pub fn is_loader_tag(c: &str) -> bool {
    LOADER_TAGS.contains(&c)
}

pub fn is_mods_like(loader: &str) -> bool {
    MODS_LIKE.contains(&loader)
}

/// `categories.filter(c => !LOADER_TAGS.has(c))` (mantém não-strings, como o Set)
pub fn filter_categories(v: Option<&Value>) -> Value {
    let arr = v.and_then(|x| x.as_arr()).cloned().unwrap_or_default();
    Value::Arr(arr.into_iter().filter(|c| !c.as_str().is_some_and(is_loader_tag)).collect())
}

/// `h.display_categories || h.categories || []`
pub fn display_categories(h: &Value) -> Value {
    let pick = if truthy(h.get("display_categories")) { h.get("display_categories") } else { h.get("categories") };
    filter_categories(pick)
}

/// `detectLoader()`
pub fn detect_loader(cfg: &Map, s: &Srv) -> String {
    if truthy(Some(&s.loader)) {
        return jsutil::to_string(Some(&s.loader));
    }
    if s.legacy && truthy(cfg.get("loader")) {
        return config::s(cfg, "loader");
    }
    if let Ok(m) = std::fs::read_to_string(jsutil::path_join(&[&s.dir, ".craftbox-loader"])) {
        let m = jsutil::trim(&m);
        if !m.is_empty() {
            return m.to_string();
        }
    }
    let has_mods = std::path::Path::new(&jsutil::path_join(&[&s.dir, "mods"])).exists();
    let has_plugins = std::path::Path::new(&jsutil::path_join(&[&s.dir, "plugins"])).exists();
    if has_mods && !has_plugins {
        return "fabric".into();
    }
    if has_plugins && !has_mods {
        return "paper".into();
    }
    if has_mods { "fabric" } else { "paper" }.into()
}

/// `contentFolder(loader)` (cria a pasta)
pub fn content_folder(s: &Srv, loader: &str) -> String {
    let dir = jsutil::path_join(&[&s.dir, if is_mods_like(loader) { "mods" } else { "plugins" }]);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn loader_categories(loader: &str) -> Vec<&str> {
    if is_mods_like(loader) {
        vec![loader]
    } else {
        vec!["paper", "spigot", "bukkit"]
    }
}

/// loaders aceitos ao listar versões de um projeto (mais amplo que a busca)
fn version_loaders(loader: &str) -> Vec<&str> {
    match loader {
        "fabric" => vec!["fabric", "quilt"],
        "quilt" | "forge" | "neoforge" => vec![loader],
        _ => vec!["paper", "spigot", "bukkit", "purpur", "folia"],
    }
}

/// `[\w.\-]+` a partir do início de `s`
fn ver_token(s: &str) -> &str {
    let n = s.bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')).count();
    &s[..n]
}

/// `/minecraft server version\s+([\w.\-]+)/i || /\(MC:\s*([\w.\-]+)\)/`
fn mc_version_from_log(log: &str) -> Option<String> {
    let lower = log.to_ascii_lowercase();
    let pat = "minecraft server version";
    let mut from = 0;
    while let Some(i) = lower[from..].find(pat) {
        let j = from + i + pat.len();
        let rest = &log[j..];
        let ws = rest.len() - rest.trim_start_matches(jsutil::is_js_ws).len();
        if ws > 0 {
            let t = ver_token(&rest[ws..]);
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        from = j;
    }
    let mut from = 0;
    while let Some(i) = log[from..].find("(MC:") {
        let j = from + i + 4;
        let rest = log[j..].trim_start_matches(jsutil::is_js_ws);
        let t = ver_token(rest);
        if !t.is_empty() && rest[t.len()..].starts_with(')') {
            return Some(t.to_string());
        }
        from = j;
    }
    None
}

/// `detectMcVersion()` (None = null)
pub fn detect_mc_version(cfg: &Map, s: &Srv) -> Option<String> {
    if truthy(Some(&s.mc_version)) {
        return Some(jsutil::to_string(Some(&s.mc_version)));
    }
    if s.legacy && truthy(cfg.get("mcVersion")) {
        return Some(config::s(cfg, "mcVersion"));
    }
    let log = std::fs::read(jsutil::path_join(&[&s.dir, "logs", "latest.log"])).ok()?;
    mc_version_from_log(&String::from_utf8_lossy(&log))
}

// ---------------------------------------------------------------------------
// Modrinth
// ---------------------------------------------------------------------------
pub fn modrinth_search(query: &str, loader: &str, version: Option<&str>, sort: &str, category: &str, offset: &str) -> Result<Value, String> {
    let cats: Vec<String> = loader_categories(loader).iter().map(|c| format!("\"categories:{}\"", c)).collect();
    let mut facets = vec![format!("[{}]", cats.join(","))];
    if let Some(v) = version.filter(|v| !v.is_empty()) {
        facets.push(format!("[\"versions:{}\"]", v));
    }
    if !category.is_empty() {
        facets.push(format!("[\"categories:{}\"]", category));
    }
    let index = if SORTS.contains(&sort) { sort } else { "relevance" };
    let limit = 24.0;
    let offset = jsutil::parse_int(offset).filter(|n| !n.is_nan()).unwrap_or(0.0).max(0.0);
    let url = format!(
        "https://api.modrinth.com/v2/search?limit=24&offset={}&index={}&query={}&facets={}",
        json::num_to_string(offset),
        index,
        enc(query),
        enc(&format!("[{}]", facets.join(",")))
    );
    let data = net::get_json(&url)?;
    let results: Vec<Value> = data
        .get("hits")
        .and_then(|h| h.as_arr())
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|h| {
            obj! {
                "slug" => h.fld("slug"), "projectId" => h.fld("project_id"),
                "title" => h.fld("title"), "author" => h.fld("author"),
                "description" => h.fld("description"), "downloads" => h.fld("downloads"),
                "follows" => h.fld("follows"), "icon" => h.fld("icon_url"),
                "type" => h.fld("project_type"), "categories" => display_categories(h),
            }
        })
        .collect();
    let total = if truthy(data.get("total_hits")) { data.get("total_hits").cloned().unwrap() } else { Value::Num(0.0) };
    Ok(obj! { "results" => results, "total" => total, "offset" => offset, "limit" => limit })
}

pub struct Version {
    pub id: String,
    pub version_number: Value,
    pub version_type: Value,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub compatible: bool,
    pub dependencies: Vec<Value>,
    pub json: Value,
}

/// lista todas as versões compatíveis com o loader; marca .compatible pra versão do MC
pub fn modrinth_versions(slug: &str, loader: &str, mc: Option<&str>) -> Vec<Version> {
    let ls: Vec<Value> = version_loaders(loader).into_iter().map(Value::from).collect();
    let u = format!(
        "https://api.modrinth.com/v2/project/{}/version?loaders={}",
        enc(slug),
        enc(&json::stringify(&Value::Arr(ls)))
    );
    let vers = net::get_json(&u).unwrap_or(Value::Arr(vec![]));
    let arr = vers.as_arr().cloned().unwrap_or_default();
    arr.into_iter()
        .map(|v| {
            let files = v.get("files").and_then(|f| f.as_arr()).cloned().unwrap_or_default();
            let file = files
                .iter()
                .find(|f| truthy(f.get("primary")))
                .or_else(|| files.first())
                .cloned();
            let gv = v.get("game_versions").filter(|x| truthy(Some(x))).cloned().unwrap_or(Value::Arr(vec![]));
            let compatible = match mc {
                Some(m) if !m.is_empty() => gv.as_arr().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(m))),
                _ => true,
            };
            let filename = file.as_ref().map(|f| jsutil::path_basename(&jsutil::to_string(f.get("filename"))));
            let url = file.as_ref().and_then(|f| f.get("url")).map(|u| jsutil::to_string(Some(u)));
            let deps = v.get("dependencies").and_then(|d| d.as_arr()).cloned().unwrap_or_default();
            let json = obj! {
                "id" => v.fld("id"), "name" => v.fld("name"),
                "versionNumber" => v.fld("version_number"), "gameVersions" => gv.clone(),
                "versionType" => v.fld("version_type"), "datePublished" => v.fld("date_published"),
                "downloads" => v.fld("downloads"),
                "filename" => filename.clone(), "url" => url.clone(),
                "compatible" => compatible, "dependencies" => Value::Arr(deps.clone()),
            };
            Version {
                id: jsutil::to_string(v.get("id")),
                version_number: v.get("version_number").cloned().unwrap_or(Value::Null),
                version_type: v.get("version_type").cloned().unwrap_or(Value::Null),
                filename,
                url,
                compatible,
                dependencies: deps,
                json,
            }
        })
        .collect()
}

pub fn modrinth_project(slug_or_id: &str) -> Result<Value, String> {
    let p = net::get_json(&format!("https://api.modrinth.com/v2/project/{}", enc(slug_or_id)))?;
    let gallery: Vec<Value> = p
        .get("gallery")
        .and_then(|g| g.as_arr())
        .map(|a| a.iter().map(|g| g.get("url").cloned().unwrap_or(Value::Null)).collect())
        .unwrap_or_default();
    Ok(obj! {
        "slug" => p.fld("slug"), "projectId" => p.fld("id"), "title" => p.fld("title"),
        "description" => p.fld("description"), "body" => p.fld("body"),
        "icon" => p.fld("icon_url"), "categories" => filter_categories(p.get("categories")),
        "downloads" => p.fld("downloads"), "follows" => p.fld("followers"), "gallery" => gallery,
        "source" => p.fld("source_url"), "projectType" => p.fld("project_type"),
        "clientSide" => p.fld("client_side"), "serverSide" => p.fld("server_side"),
    })
}

/// escolhe a "melhor" versão: id explícito > compatível+release > compatível > qualquer release > qualquer
pub fn pick_version<'a>(versions: &'a [Version], version_id: Option<&str>) -> Option<&'a Version> {
    if let Some(id) = version_id.filter(|i| !i.is_empty()) {
        if let Some(v) = versions.iter().find(|x| x.id == id) {
            return Some(v);
        }
    }
    let compat: Vec<&Version> = versions.iter().filter(|v| v.compatible && v.url.is_some()).collect();
    let pool: Vec<&Version> =
        if !compat.is_empty() { compat } else { versions.iter().filter(|v| v.url.is_some()).collect() };
    pool.iter().find(|v| v.version_type.as_str() == Some("release")).copied().or_else(|| pool.first().copied())
}

/// wrapper legado (usado pelo Bedrock) — (filename, url)
fn modrinth_resolve(slug: &str, loader: &str, version: Option<&str>) -> Result<(String, String), String> {
    let vers = modrinth_versions(slug, loader, version);
    match pick_version(&vers, None) {
        Some(Version { url: Some(u), filename: Some(f), .. }) => Ok((f.clone(), u.clone())),
        _ => Err("sem versao compativel pra esse loader".into()),
    }
}

// ---- manifesto: sabe QUAIS projetos estão instalados (não só arquivos) ----
fn manifest_path(s: &Srv) -> String {
    jsutil::path_join(&[&s.dir, ".craftbox-content.json"])
}
pub fn read_manifest(s: &Srv) -> Map {
    match std::fs::read(manifest_path(s)).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)).ok()) {
        Some(Value::Obj(m)) => m,
        _ => Map::new(),
    }
}
pub fn write_manifest(s: &Srv, m: &Map) {
    let _ = std::fs::write(manifest_path(s), json::stringify_pretty(&Value::Obj(m.clone())));
}

fn entry_str(man: &Map, slug: &str, k: &str) -> Option<Value> {
    man.get(slug).and_then(|m| m.get(k)).filter(|v| truthy(Some(v))).cloned()
}

/// instala um projeto (versão específica ou melhor) + dependências obrigatórias
pub fn install_project(
    s: &Srv,
    slug: &str,
    loader: &str,
    mc: Option<&str>,
    version_id: Option<&str>,
    visited: &mut Vec<String>,
    depth: u32,
) -> Result<Vec<Value>, String> {
    if visited.iter().any(|v| v == slug) {
        return Ok(vec![]);
    }
    visited.push(slug.to_string());
    let versions = modrinth_versions(slug, loader, mc);
    if versions.is_empty() {
        return Err(format!("sem versão compatível de \"{}\" pra {}", slug, loader));
    }
    let v = match pick_version(&versions, version_id) {
        Some(v) if v.url.is_some() => v,
        _ => return Err(format!("sem arquivo instalável pra \"{}\"", slug)),
    };
    let filename = v.filename.clone().unwrap_or_default();
    let folder = content_folder(s, loader);
    let mut man = read_manifest(s);
    // remove versão antiga do mesmo projeto (ativa ou desativada)
    if let Some(old) = man.get(slug).and_then(|m| m.get("filename")).filter(|f| truthy(Some(f))) {
        let old = jsutil::to_string(Some(old));
        if old != filename {
            let _ = std::fs::remove_file(jsutil::path_join(&[&folder, &old]));
            let _ = std::fs::remove_file(jsutil::path_join(&[&folder, &format!("{}.disabled", old)]));
        }
    }
    net::download(v.url.as_ref().unwrap(), &jsutil::path_join(&[&folder, &filename]))?;
    let proj = modrinth_project(slug).ok();
    let pget = |k: &str| proj.as_ref().and_then(|p| p.get(k)).cloned().unwrap_or(Value::Null);
    let title = if proj.is_some() { pget("title") } else { entry_str(&man, slug, "title").unwrap_or(Value::from(slug)) };
    let entry = obj! {
        "projectId" => if proj.is_some() { pget("projectId") } else { entry_str(&man, slug, "projectId").unwrap_or(Value::Null) },
        "title" => title.clone(),
        "icon" => if proj.is_some() { pget("icon") } else { entry_str(&man, slug, "icon").unwrap_or(Value::Null) },
        "filename" => filename.clone(), "versionId" => v.id.clone(), "versionNumber" => v.version_number.clone(),
        "type" => if is_mods_like(loader) { "mod" } else { "plugin" }, "disabled" => false, "installedAt" => jsutil::now_ms(),
    };
    man.insert(slug, entry);
    write_manifest(s, &man);
    let mut installed = vec![obj! {
        "slug" => slug, "title" => title, "filename" => filename, "version" => v.version_number.clone(), "dep" => depth > 0,
    }];
    if depth < 3 {
        for d in v.dependencies.clone() {
            if d.get("dependency_type").and_then(|x| x.as_str()) != Some("required") || !truthy(d.get("project_id")) {
                continue;
            }
            let pid = jsutil::to_string(d.get("project_id"));
            let dep_slug = modrinth_project(&pid)
                .ok()
                .and_then(|p| p.get("slug").map(|x| jsutil::to_string(Some(x))))
                .unwrap_or(pid);
            if visited.contains(&dep_slug) {
                continue;
            }
            let vid = d.get("version_id").filter(|x| truthy(Some(x))).map(|x| jsutil::to_string(Some(x)));
            if let Ok(more) = install_project(s, &dep_slug, loader, mc, vid.as_deref(), visited, depth + 1) {
                installed.extend(more);
            }
        }
    }
    Ok(installed)
}

/// lista instalados: cruza o manifesto com os arquivos reais na pasta
/// `uploadContent(name, data)`: mod/plugin enviado à mão (corpo cru). Só `.jar`
/// que seja zip de verdade; vai pra `mods/` ou `plugins/` conforme o loader.
/// Substitui um arquivo de mesmo nome (e tira a cópia `.disabled`, se houver).
pub fn upload_content(cfg: &Map, s: &Srv, name: &str, data: &[u8]) -> Result<Value, (u16, String)> {
    let base = jsutil::path_basename(name);
    if base.is_empty() || base.starts_with('.') || !base.to_lowercase().ends_with(".jar") {
        return Err((400, format!("{}: só dá pra enviar arquivos .jar", if base.is_empty() { "arquivo" } else { &base })));
    }
    if !data.starts_with(b"PK\x03\x04") {
        return Err((400, format!("{}: não é um .jar válido", base)));
    }
    let loader = detect_loader(cfg, s);
    let folder = content_folder(s, &loader);
    let dest = jsutil::path_join(&[&folder, &base]);
    let disabled = format!("{}.disabled", dest);
    let replaced = std::path::Path::new(&dest).exists() || std::path::Path::new(&disabled).exists();
    let tmp = format!("{}.part", dest);
    std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &dest)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        (500, e.to_string())
    })?;
    let _ = std::fs::remove_file(&disabled);
    Ok(obj! {
        "ok" => true, "file" => base, "folder" => if is_mods_like(&loader) { "mods" } else { "plugins" }, "replaced" => replaced,
    })
}

/// `modsZip()`: zip com os mods que os jogadores precisam, pra jogar direto
/// na pasta `mods/` do cliente. Entram os `.jar` ativos e os que o painel
/// desativou por serem só de cliente (`strippedMods` do meta, voltando a `.jar`);
/// os desativados à mão ficam de fora. Leva um LEIA-ME.txt com a versão do
/// Minecraft e do loader. Devolve (arquivo já aberto e desvinculado do disco,
/// tamanho, quantos mods).
pub fn mods_zip(cfg: &Map, s: &Srv) -> Result<(std::fs::File, u64, usize), (u16, String)> {
    let loader = detect_loader(cfg, s);
    if !is_mods_like(&loader) {
        return Err((400, "servidor de plugins (Paper/vanilla): os jogadores não precisam de mods".into()));
    }
    let folder = content_folder(s, &loader);
    let meta = crate::ctx::read_instance_meta(&s.dir);
    let stripped: Vec<String> = meta
        .get("strippedMods")
        .and_then(|v| v.as_arr())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut files: Vec<(String, String)> = Vec::new(); // (nome no zip, caminho)
    if let Ok(rd) = std::fs::read_dir(&folder) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".jar") {
                files.push((n.clone(), jsutil::path_join(&[&folder, &n])));
            } else if let Some(base) = n.strip_suffix(".disabled") {
                if base.ends_with(".jar") && stripped.iter().any(|x| x == base) {
                    files.push((base.to_string(), jsutil::path_join(&[&folder, &n])));
                }
            }
        }
    }
    if files.is_empty() {
        return Err((404, "nenhum mod instalado neste servidor".into()));
    }
    files.sort_by_key(|a| a.0.to_lowercase());
    let mc = detect_mc_version(cfg, s).unwrap_or_else(|| "?".into());
    let lv = jsutil::to_string_or_empty(meta.get("loaderVersion"));
    let loader_name = match loader.as_str() {
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        "quilt" => "Quilt",
        _ => "Fabric",
    };
    let mut readme = format!(
        "Mods do servidor \"{}\" (craftbox)\r\n\r\n\
Minecraft {mc}  +  {loader_name}{}\r\n{} mods\r\n\r\n\
Como usar:\r\n \
1. Crie uma instância Minecraft {mc} com {loader_name}{} (Prism Launcher, CurseForge ou o instalador do {loader_name}).\r\n \
2. Extraia estes arquivos .jar dentro da pasta \"mods\" da instância (.minecraft/mods).\r\n \
3. Abra o jogo e entre no servidor. Se aparecer \"mods diferentes\", confira se não sobrou mod antigo na pasta.\r\n",
        s.name_str(),
        if lv.is_empty() { String::new() } else { format!(" {}", lv) },
        files.len(),
        if lv.is_empty() { String::new() } else { format!(" {}", lv) },
    );
    if let Some(mp) = meta.get("modpack").filter(|m| truthy(Some(m))) {
        readme.push_str(&format!(
            "\r\nEste servidor usa o modpack {} {}. Dá pra instalar o pack inteiro (com configs, texturas e shaders) por:\r\n{}\r\n",
            jsutil::to_string_or_empty(mp.get("name")),
            jsutil::to_string_or_empty(mp.get("version")),
            jsutil::to_string_or_empty(mp.get("url")),
        ));
    }
    let tmp = std::env::temp_dir().join(format!("craftbox-mods-{}.zip", crate::crypto::random_hex(6)));
    let tmp_s = tmp.to_string_lossy().into_owned();
    let res = (|| -> std::io::Result<()> {
        let mut out = std::fs::File::create(&tmp)?;
        let mut entries: Vec<(String, crate::zipw::Source)> = vec![("LEIA-ME.txt".into(), crate::zipw::Source::Bytes(readme.as_bytes()))];
        for (n, p) in &files {
            entries.push((n.clone(), crate::zipw::Source::Path(p)));
        }
        crate::zipw::write_stored(&mut out, &entries)
    })();
    let opened = res.and_then(|_| std::fs::File::open(&tmp));
    let _ = std::fs::remove_file(&tmp_s); // o arquivo aberto continua legível até fechar
    let f = opened.map_err(|e| (500, format!("não consegui montar o zip: {}", e)))?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((f, size, files.len()))
}

pub fn list_installed(cfg: &Map, s: &Srv) -> Value {
    let loader = detect_loader(cfg, s);
    let folder = content_folder(s, &loader);
    let man = read_manifest(s);
    let mut files: Vec<String> = std::fs::read_dir(&folder)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    files.sort(); // o scandir do libuv ordena por strcmp (= ordem de bytes do UTF-8)
    // byBase preserva a ordem de inserção (ordem do readdir)
    let mut by_base: Vec<(String, (String, bool, f64))> = Vec::new();
    for f in files.iter().filter(|f| f.ends_with(".jar") || f.ends_with(".jar.disabled")) {
        let disabled = f.ends_with(".disabled");
        let base = if disabled { f[..f.len() - ".disabled".len()].to_string() } else { f.clone() };
        let size_mb = std::fs::metadata(jsutil::path_join(&[&folder, f]))
            .map(|m| jsutil::to_fixed(m.len() as f64 / 1048576.0, 2))
            .unwrap_or(0.0);
        if let Some(slot) = by_base.iter_mut().find(|(b, _)| *b == base) {
            slot.1 = (f.clone(), disabled, size_mb);
        } else {
            by_base.push((base, (f.clone(), disabled, size_mb)));
        }
    }
    let mut items: Vec<(String, Value)> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for (slug, m) in man.iter() {
        let fname = jsutil::to_string(m.get("filename"));
        let Some((_, (file, disabled, size))) = by_base.iter().find(|(b, _)| *b == fname) else { continue };
        used.push(fname.clone());
        let title = m.get("title").cloned().unwrap_or(Value::Null);
        items.push((
            jsutil::to_string(Some(&title)),
            obj! {
                "slug" => slug.clone(), "title" => title, "icon" => m.fld("icon"),
                "version" => m.fld("versionNumber"), "versionId" => m.fld("versionId"),
                "file" => file.clone(), "filename" => fname, "disabled" => *disabled, "sizeMB" => *size, "managed" => true,
            },
        ));
    }
    for (base, (file, disabled, size)) in &by_base {
        if used.contains(base) {
            continue;
        }
        let title = base.strip_suffix(".jar").unwrap_or(base).to_string();
        items.push((
            title.clone(),
            obj! {
                "slug" => Value::Null, "title" => title, "icon" => Value::Null, "version" => Value::Null,
                "versionId" => Value::Null, "file" => file.clone(), "filename" => base.clone(),
                "disabled" => *disabled, "sizeMB" => *size, "managed" => false,
            },
        ));
    }
    items.sort_by(|a, b| jsutil::locale_compare(&a.0, &b.0));
    obj! { "loader" => loader, "folder" => folder, "items" => items.into_iter().map(|x| x.1).collect::<Vec<_>>() }
}

/// verifica atualizações dos projetos gerenciados
pub fn check_updates(cfg: &Map, s: &Srv) -> Vec<Value> {
    let loader = detect_loader(cfg, s);
    let mc = detect_mc_version(cfg, s);
    let man = read_manifest(s);
    let mut updates = Vec::new();
    for (slug, m) in man.iter() {
        let vers = modrinth_versions(slug, &loader, mc.as_deref());
        if let Some(latest) = pick_version(&vers, None) {
            if Some(latest.id.as_str()) != m.get("versionId").and_then(|x| x.as_str()) {
                updates.push(obj! {
                    "slug" => slug.clone(), "title" => m.fld("title"), "current" => m.fld("versionNumber"),
                    "latest" => latest.version_number.clone(), "versionId" => latest.id.clone(),
                });
            }
        }
    }
    updates
}

pub fn install_bedrock(cfg: &Map, s: &Srv) -> Result<Value, String> {
    let loader = detect_loader(cfg, s);
    let folder = content_folder(s, &loader);
    let gm = |proj: &str, plat: &str| {
        format!("https://download.geysermc.org/v2/projects/{}/versions/latest/builds/latest/downloads/{}", proj, plat)
    };
    let mut installed: Vec<Value> = Vec::new();
    let dl = |url: &str, name: &str| net::download(url, &jsutil::path_join(&[&folder, name]));
    if loader == "paper" {
        dl(&gm("geyser", "spigot"), "Geyser-Spigot.jar")?;
        installed.push("Geyser-Spigot.jar".into());
        dl(&gm("floodgate", "spigot"), "floodgate-spigot.jar")?;
        installed.push("floodgate-spigot.jar".into());
    } else {
        dl(&gm("geyser", "fabric"), "Geyser-Fabric.jar")?;
        installed.push("Geyser-Fabric.jar".into());
        let ver = detect_mc_version(cfg, s);
        let (f, u) = modrinth_resolve("floodgate", "fabric", ver.as_deref())?;
        dl(&u, &f)?;
        installed.push(f.into());
        if let Ok((f, u)) = modrinth_resolve("fabric-api", "fabric", ver.as_deref()) {
            if dl(&u, &f).is_ok() {
                installed.push(f.into());
            }
        }
    }
    Ok(obj! { "loader" => loader, "folder" => folder, "installed" => installed })
}

/// `ghLatestAsset(repo, match)`: primeiro asset da última release que casa com o filtro
fn gh_latest_asset(repo: &str, is_match: impl Fn(&str) -> bool) -> Result<String, String> {
    let rel = net::get_json(&format!("https://api.github.com/repos/{}/releases/latest", repo))?;
    rel.get("assets")
        .and_then(|a| a.as_arr())
        .and_then(|a| a.iter().find(|x| x.get("name").and_then(|n| n.as_str()).is_some_and(&is_match)))
        .and_then(|a| a.get("browser_download_url"))
        .map(|u| jsutil::to_string(Some(u)))
        .ok_or_else(|| format!("não achei o .jar na release de {}", repo))
}

/// Login de jogadores: offline mode + plugin de auth (pirata usa /register /login; premium = autologin)
pub fn install_player_auth(cfg: &Map, s: &Srv) -> Result<Value, String> {
    let loader = detect_loader(cfg, s);
    let folder = content_folder(s, &loader);
    let mc = detect_mc_version(cfg, s);
    let mut installed: Vec<Value> = Vec::new();
    // AuthMe é plugin Bukkit e o EasyAuth só existe pra Fabric: em Forge/NeoForge/Quilt
    // o AuthMe ia parar na pasta mods sem funcionar
    if !matches!(loader.as_str(), "fabric" | "paper") {
        return Err(format!(
            "login de jogadores ainda não tem suporte a servidores {}: só Paper (AuthMe) e Fabric (EasyAuth). \
             O modo offline sozinho funciona, mas sem senha qualquer um entra com qualquer nick.",
            loader
        ));
    }
    if loader == "fabric" {
        let vers = modrinth_versions("easyauth", &loader, mc.as_deref());
        let v = pick_version(&vers, None).filter(|v| v.compatible).ok_or_else(|| {
            format!(
                "o EasyAuth ainda não tem versão pra MC {}. Use um servidor 1.20/1.21 pra esse recurso.",
                mc.clone().unwrap_or_else(|| "?".into())
            )
        })?;
        let f = v.filename.clone().unwrap_or_default();
        net::download(v.url.as_deref().unwrap_or(""), &jsutil::path_join(&[&folder, &f]))?;
        installed.push(f.into());
        if let Ok((f, u)) = modrinth_resolve("fabric-api", "fabric", mc.as_deref()) {
            if net::download(&u, &jsutil::path_join(&[&folder, &f])).is_ok() {
                installed.push(f.into());
            }
        }
    } else {
        // desde o 6.x a release traz um .jar por plataforma (Paper, Folia, Spigot,
        // Bungee, Velocity); pegar "o primeiro AuthMe*.jar" trazia o módulo do
        // BungeeCord, que não carrega no Paper. Ordem: Paper > Spigot > jar único antigo.
        let pick = |pref: &'static str| {
            move |n: &str| {
                let l = n.to_ascii_lowercase();
                l.starts_with("authme") && l.ends_with(".jar") && l.contains(pref)
            }
        };
        let u = gh_latest_asset("AuthMe/AuthMeReloaded", pick("-paper"))
            .or_else(|_| gh_latest_asset("AuthMe/AuthMeReloaded", pick("-spigot-1.")))
            .or_else(|_| {
                gh_latest_asset("AuthMe/AuthMeReloaded", |n| {
                    let l = n.to_ascii_lowercase();
                    l.starts_with("authme") && l.ends_with(".jar") && !["bungee", "velocity", "folia", "legacy"].iter().any(|x| l.contains(x))
                })
            })?;
        net::download(&u, &jsutil::path_join(&[&folder, "AuthMeReloaded.jar"]))?;
        installed.push("AuthMeReloaded.jar".into());
    }
    ctx::write_props_kv(&s.dir, &[("online-mode", "false")]).map_err(|e| e.to_string())?;
    Ok(obj! { "loader" => loader, "offline" => true, "installed" => installed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mc_version_from_logs() {
        assert_eq!(
            mc_version_from_log("[10:00:00] [Server thread/INFO]: Starting minecraft server version 1.21.1\n").as_deref(),
            Some("1.21.1")
        );
        assert_eq!(mc_version_from_log("This server is running Paper version 1.20 (MC: 1.20.4) x").as_deref(), Some("1.20.4"));
        assert_eq!(mc_version_from_log("nada"), None);
    }

    #[test]
    fn category_filter() {
        let v = json::parse(r#"{"display_categories":["fabric","adventure","server"],"categories":["x"]}"#).unwrap();
        assert_eq!(json::stringify(&display_categories(&v)), r#"["adventure"]"#);
        let v = json::parse(r#"{"display_categories":[],"categories":["forge","magic"]}"#).unwrap();
        assert_eq!(json::stringify(&display_categories(&v)), r#"[]"#);
    }
}
