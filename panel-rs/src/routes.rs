//! Roteador: mesma ordem de decisão do `http.createServer` do server.js.
//!
//!  1. /api/login, /api/logout, /api/authcheck (sem sessão)
//!  2. estáticos (/, /public/*, /app.js, /style.css, /logo.png, /favicon.png, /logos/*)
//!  3. /api/* exige sessão → 401 {"error":"nao autenticado"}
//!  4. rotas (todas migradas); resto → 404 JSON
//!  5. fora de /api → 404 "not found" (texto puro)
//!
//! Erro não tratado numa rota vira `500 {error: <mensagem>}`, como o catch
//! global do Node; as mensagens de `TypeError` de corpo `null` são reproduzidas.

use crate::audit::{self, Who};
use crate::auth;
use crate::config;
use crate::content;
use crate::crypto;
use crate::ctx::{self, Srv};
use crate::diag;
use crate::http::{Request, Response};
use crate::integrations as intg;
use crate::json::{self, Map, Value};
use crate::jsutil::{self, truthy};
use crate::modpacks;
use crate::obj;
use crate::rcon;
use crate::runner;
use crate::servers;
use crate::stats;
use crate::State;
use std::time::Duration;

pub fn json(code: u16, v: &Value) -> Response {
    let body = json::stringify(v);
    Response {
        status: code,
        headers: vec![
            ("Content-Type".into(), "application/json; charset=utf-8".into()),
            ("Content-Length".into(), body.len().to_string()),
        ],
        body: body.into_bytes(),
        stream: None,
    }
}

fn err(code: u16, msg: &str) -> Response {
    json(code, &obj! { "error" => msg })
}

/// `readBody(req)`: JSON.parse(body || '{}'), `{}` se inválido.
fn read_body(req: &Request) -> Value {
    let txt = String::from_utf8_lossy(&req.body);
    let txt = if txt.is_empty() { "{}".into() } else { txt };
    json::parse(&txt).unwrap_or(Value::Obj(Map::new()))
}

/// `const { k } = await readBody(req)` — corpo `null` lança TypeError no Node.
fn destr<'a>(b: &'a Value, k: &str) -> Result<Option<&'a Value>, String> {
    match b {
        Value::Null => Err(format!("Cannot destructure property '{}' of '(intermediate value)' as it is null.", k)),
        v => Ok(v.get(k)),
    }
}

/// `b.k` com `b` vindo do readBody — `null.k` lança TypeError no Node.
fn prop<'a>(b: &'a Value, k: &str) -> Result<Option<&'a Value>, String> {
    match b {
        Value::Null => Err(format!("Cannot read properties of null (reading '{}')", k)),
        v => Ok(v.get(k)),
    }
}

/// Contexto por requisição (o que o server.js guarda no AsyncLocalStorage).
pub struct ReqCtx {
    pub srv: Srv,
    pub ip: String,
    /// `sessionUser(req) || '-'`
    pub user: String,
}

impl ReqCtx {
    fn who(&self) -> Who<'_> {
        Who { ip: &self.ip, user: &self.user, server: &self.srv.id }
    }
}

// ---------------------------------------------------------------------------
// Estáticos
// ---------------------------------------------------------------------------
fn mime(ext: &str) -> &'static str {
    // path.extname é sensível a maiúsculas: ".PNG" cai no octet-stream, como no Node
    match ext {
        ".html" => "text/html; charset=utf-8",
        ".js" => "text/javascript; charset=utf-8",
        ".css" => "text/css; charset=utf-8",
        ".svg" => "image/svg+xml",
        ".ico" => "image/x-icon",
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn serve_static(public: &str, file: &str) -> Response {
    let full = jsutil::path_join(&[public, file]);
    if !full.starts_with(public) {
        return Response::plain(403, "forbidden");
    }
    match std::fs::read(&full) {
        Ok(data) => Response {
            status: 200,
            headers: vec![("Content-Type".into(), mime(&jsutil::path_extname(&full)).into())],
            body: data,
            stream: None,
        },
        Err(_) => Response::plain(404, "not found"),
    }
}

fn server_action_route(p: &str) -> Option<(&str, &str)> {
    // ^/api/servers/([^/]+)/(start|stop|restart|status)$
    let rest = p.strip_prefix("/api/servers/")?;
    let mut it = rest.split('/');
    match (it.next(), it.next(), it.next()) {
        (Some(id), Some(a @ ("start" | "stop" | "restart" | "status")), None) if !id.is_empty() => Some((id, a)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------
pub fn handle(st: &State, req: &Request) -> Response {
    if req.path == "/mcp" {
        return crate::mcp::handle(st, req);
    }
    let cfg = st.cfg();
    let cookie = req.header("cookie").unwrap_or_default();
    let srv = ctx::server_ctx(&cfg, &ctx::current_id(&cfg, req.query_get("server").as_deref()));
    let rc = ReqCtx { srv, ip: req.ip.clone(), user: auth::session_user(&cfg, &cookie).unwrap_or_else(|| "-".into()) };
    match route(st, &cfg, req, &rc, &cookie) {
        Ok(r) => r,
        Err(msg) => err(500, &msg),
    }
}

/// `Math.min(max, parseInt(q || def, 10))` e o `slice(-n)` que vem depois
/// (NaN → arquivo inteiro; negativo → corta do começo, como no JS).
fn js_tail<'a>(v: &'a [&'a str], n: Option<f64>) -> &'a [&'a str] {
    let len = v.len() as f64;
    let start = match n {
        None => 0.0,
        Some(n) => {
            let s = -n;
            if s < 0.0 {
                (len + s).max(0.0)
            } else {
                s.min(len)
            }
        }
    };
    &v[start as usize..]
}

fn query_lines(req: &Request, def: &str, max: f64) -> Option<f64> {
    let q = req.query_get("lines").filter(|s| !s.is_empty()).unwrap_or_else(|| def.to_string());
    jsutil::parse_int(&q).map(|n| n.min(max))
}

fn mtime_ms(m: &std::fs::Metadata) -> f64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1e6)
        .unwrap_or(0.0)
}

/// Sonda de 2 s: servidor travado não pode segurar o /api/status.
fn players_of(cfg: &Map, s: &Srv) -> Value {
    if let Ok(list) = rcon::command_timeout(cfg, s, "list", Duration::from_secs(2)) {
        if let Some((online, max)) = rcon::parse_players(&list) {
            return obj! { "online" => online, "max" => max };
        }
    }
    Value::Null
}

const UPDATER: &str = "/usr/local/bin/craftbox-update";
const UPDATE_LOG: &str = "/var/log/craftbox-update.log";

/// `systemctl is-active craftbox-update` (a unidade transitória do systemd-run)
fn update_running() -> bool {
    let r = runner::run("systemctl", &["is-active", "craftbox-update"], runner::RUN_TIMEOUT);
    matches!(jsutil::trim(&r.stdout), "active" | "activating")
}

/// Rotas que leem/alteram a instância selecionada (ver guarda de "nenhum servidor").
fn acts_on_server(p: &str, m: &str) -> bool {
    let write = m != "GET";
    matches!(p, "/api/power" | "/api/rcon" | "/api/content/install" | "/api/content/upload" | "/api/content/toggle"
        | "/api/modpacks/manual-upload" | "/api/backups/restore")
        || p.starts_with("/api/compat/")
        || (write && matches!(p, "/api/properties" | "/api/backups" | "/api/content/installed"))
}

/// Depois disso sem o RCON responder, "iniciando" vira "sem resposta".
const STALL_SECS: f64 = 15.0 * 60.0;

/// Estado de verdade do servidor. O systemd diz `active` assim que o start.sh
/// roda, mas o Minecraft só aceita jogadores depois do "Done", que é quando o
/// RCON sobe. Então `active` só vale com o RCON respondendo; antes disso é
/// `activating`, e se passar de STALL_SECS vira `stalled` (travado, em geral
/// swap). Sem RCON habilitado, cai no "Done (" do logs/latest.log.
fn live_state(exec: bool, cfg: &Map, s: &Srv) -> (String, Value, Option<f64>) {
    let active = runner::svc_active_of(exec, cfg, &s.service);
    let uptime = runner::svc_uptime(exec, cfg, &s.service);
    if active != "active" {
        return (active, Value::Null, uptime);
    }
    let players = players_of(cfg, s);
    if !matches!(players, Value::Null) {
        return (active, players, uptime);
    }
    let rcon_off = ctx::read_props(&s.dir).get("enable-rcon").and_then(|v| v.as_str()) == Some("false");
    if rcon_off {
        let log = jsutil::path_join(&[&s.dir, "logs", "latest.log"]);
        let fresh = std::fs::metadata(&log)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| uptime.is_none_or(|u| age.as_secs_f64() <= u + 5.0))
            .unwrap_or(false);
        let done = fresh && std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).contains("Done (")).unwrap_or(false);
        return (if done { active } else { "activating".into() }, Value::Null, uptime);
    }
    let st = if uptime.is_some_and(|u| u > STALL_SECS) { "stalled" } else { "activating" };
    (st.into(), Value::Null, uptime)
}

/// Desligar: com o servidor respondendo, manda `stop` pelo RCON (o log mostra o
/// salvamento e a JVM sai com 0); senão (subindo/travado) cai no systemctl stop.
fn graceful_stop(exec: bool, cfg: &Map, s: &Srv) -> runner::RunResult {
    if runner::svc_active_of(exec, cfg, &s.service) == "active" && rcon::command(cfg, s, "stop").is_ok() {
        return runner::RunResult::ok();
    }
    runner::svc_stop_nowait(exec, cfg, &s.service, false)
}

fn run_json(code_ok: bool, output: String) -> Response {
    json(if code_ok { 200 } else { 500 }, &obj! { "ok" => code_ok, "output" => output })
}

fn route(st: &State, cfg: &Map, req: &Request, rc: &ReqCtx, cookie: &str) -> Result<Response, String> {
    let p = req.path.as_str();
    let m = req.method.as_str();
    let s = &rc.srv;
    let who = rc.who();
    let exec = st.exec_runner;

    // --- login (não exige sessão) ---
    if p == "/api/login" && m == "POST" {
        return Ok(login(cfg, req, rc));
    }
    if p == "/api/logout" && m == "POST" {
        return Ok(json(200, &obj! { "ok" => true }).header_first("Set-Cookie", auth::LOGOUT_COOKIE));
    }
    if p == "/api/authcheck" {
        let user = auth::session_user(cfg, cookie);
        let authed = auth::is_authed(cfg, cookie);
        let configured = auth::multi_user(cfg)
            || (truthy(config::sub(cfg, "auth", "salt")) && truthy(config::sub(cfg, "auth", "hash")));
        let is_admin = authed && auth::is_admin(cfg, user.as_deref());
        return Ok(json(
            200,
            &obj! {
                "authed" => authed, "configured" => configured, "multiUser" => auth::multi_user(cfg),
                "user" => user, "isAdmin" => is_admin,
            },
        ));
    }

    // --- estáticos + página ---
    let public = st.public_dir.as_str();
    if p == "/" {
        return Ok(serve_static(public, "index.html"));
    }
    if let Some(rest) = p.strip_prefix("/public/") {
        return Ok(serve_static(public, rest));
    }
    if p == "/app.js"
        || p == "/style.css"
        || p == "/logo.png"
        || p == "/favicon.png"
        || (p.starts_with("/logos/") && !p.contains(".."))
    {
        return Ok(serve_static(public, &p[1..]));
    }

    // --- daqui pra baixo exige sessão ---
    if !p.starts_with("/api/") {
        return Ok(Response::plain(404, "not found"));
    }
    if !auth::is_authed(cfg, cookie) {
        return Ok(err(401, "nao autenticado"));
    }

    // sem nenhum servidor criado (instalação "Nenhum agora"), o contexto cai num id
    // fantasma ("default"): ligar, instalar plugin etc. criariam uma pasta que depois
    // apareceria como servidor. Recusa tudo que age num servidor até existir um.
    if ctx::multi_enabled(cfg) && ctx::list_instance_ids(cfg).is_empty() && acts_on_server(p, m) {
        return Ok(err(409, "nenhum servidor criado ainda: crie o primeiro na aba Servidores (ou em Modpacks)"));
    }

    // ================= painel e servidor =================
    if p == "/api/status" {
        let (active, players, uptime) = live_state(exec, cfg, s);
        return Ok(json(
            200,
            &obj! { "active" => active, "uptime" => uptime, "players" => players, "system" => stats::system_stats(&s.dir) },
        ));
    }
    if p == "/api/power" && m == "POST" {
        let b = read_body(req);
        let action = destr(&b, "action")?.cloned();
        let a = jsutil::to_string(action.as_ref());
        let r = if a == "stop" && matches!(action, Some(Value::Str(_))) {
            graceful_stop(exec, cfg, s)
        } else {
            runner::svc_action(exec, cfg, if matches!(action, Some(Value::Str(_))) { &a } else { "" }, &s.service)?
        };
        audit::audit(cfg, &who, "power", &format!("{} → {}", a, s.name_str()));
        if a == "start" && r.code == 0 {
            intg::maybe_start_tunnels(exec, cfg, s);
        }
        return Ok(run_json(r.code == 0, r.output()));
    }
    if p == "/api/properties" && m == "GET" {
        return Ok(json(200, &obj! { "properties" => ctx::read_props(&s.dir) }));
    }
    if p == "/api/properties" && m == "PUT" {
        let b = read_body(req);
        let upd = match &b {
            Value::Obj(o) => o.clone(),
            Value::Arr(a) => {
                let mut o = Map::new();
                for (i, v) in a.iter().enumerate() {
                    o.insert(i.to_string(), v.clone());
                }
                o
            }
            _ => return Ok(err(400, "payload invalido")),
        };
        let pp = jsutil::path_join(&[&s.dir, "server.properties"]);
        ctx::write_props(&s.dir, &upd).map_err(|e| jsutil::fs_err(&e, "open", &pp))?;
        audit::audit(cfg, &who, "config", &format!("server.properties ({} campos) → {}", upd.len(), s.name_str()));
        return Ok(json(200, &obj! { "ok" => true, "properties" => ctx::read_props(&s.dir) }));
    }
    if p == "/api/rcon" && m == "POST" {
        let b = read_body(req);
        let c = destr(&b, "command")?;
        if !truthy(c) {
            return Ok(err(400, "comando vazio"));
        }
        return Ok(match rcon::command(cfg, s, &jsutil::to_string(c)) {
            Ok(r) => json(200, &obj! { "response" => r }),
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/logs" {
        let n = query_lines(req, "200", 1000.0);
        let lp = jsutil::path_join(&[&s.dir, "logs", "latest.log"]);
        let text = match std::fs::read(&lp) {
            Ok(b) => {
                let data = String::from_utf8_lossy(&b);
                let v: Vec<&str> = data.split('\n').collect();
                js_tail(&v, n).join("\n")
            }
            Err(_) => {
                if exec {
                    runner::unit_journal_js(cfg, &s.service, n)
                } else {
                    let ns = n.map(json::num_to_string).unwrap_or_else(|| "NaN".into());
                    let out = runner::run(
                        "journalctl",
                        &["-u", &s.service, "-n", &ns, "--no-pager", "-o", "cat"],
                        runner::RUN_TIMEOUT,
                    )
                    .stdout;
                    if out.is_empty() {
                        "(sem logs)".into()
                    } else {
                        out
                    }
                }
            }
        };
        return Ok(json(200, &obj! { "log" => text }));
    }

    // ================= backups =================
    let bdir = jsutil::path_join(&[&s.dir, "backups"]);
    if p == "/api/backups" && m == "GET" {
        let mut items: Vec<(f64, Value, f64)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&bdir) {
            let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            for f in names.into_iter().filter(|f| f.ends_with(".tar.gz")) {
                let Ok(md) = std::fs::metadata(jsutil::path_join(&[&bdir, &f])) else {
                    items.clear(); // statSync lança dentro do try → lista vazia, como no Node
                    break;
                };
                let size = jsutil::to_fixed(md.len() as f64 / 1048576.0, 1);
                let mt = mtime_ms(&md);
                items.push((mt, obj! { "name" => f, "sizeMB" => size, "mtime" => mt }, size));
            }
        }
        items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let total: f64 = items.iter().fold(0.0, |acc, x| acc + x.2);
        return Ok(json(200, &obj! { "backups" => items.into_iter().map(|x| x.1).collect::<Vec<_>>(), "totalMB" => total }));
    }
    if p == "/api/backups" && m == "POST" {
        let r = runner::run("bash", &[&jsutil::path_join(&[&s.dir, "backup.sh"])], Duration::from_secs(120));
        audit::audit(cfg, &who, "backup", &format!("manual → {}", s.name_str()));
        let out = r.output();
        return Ok(run_json(r.code == 0, if out.is_empty() { "backup executado".into() } else { out }));
    }
    if p == "/api/backups/download" && m == "GET" {
        let name = jsutil::path_basename(&req.query_get("name").unwrap_or_default());
        let f = jsutil::path_join(&[&bdir, &name]);
        if name.is_empty() || !name.ends_with(".tar.gz") || !f.starts_with(&bdir) {
            return Ok(err(404, "não encontrado"));
        }
        let Ok(file) = std::fs::File::open(&f) else { return Ok(err(404, "não encontrado")) };
        return Ok(Response {
            status: 200,
            headers: vec![
                ("Content-Type".into(), "application/gzip".into()),
                ("Content-Disposition".into(), format!("attachment; filename=\"{}\"", name)),
            ],
            body: Vec::new(),
            stream: Some(file),
        });
    }
    if p == "/api/backups/restore" && m == "POST" {
        let b = read_body(req);
        let name = destr(&b, "name")?;
        let safe = jsutil::path_basename(&jsutil::to_string_or_empty(name));
        let f = jsutil::path_join(&[&bdir, &safe]);
        if safe.is_empty() || !safe.ends_with(".tar.gz") || !f.starts_with(&bdir) || std::fs::metadata(&f).is_err() {
            return Ok(err(404, "não encontrado"));
        }
        if runner::svc_active_of(exec, cfg, &s.service) == "active" {
            return Ok(err(409, "pare o servidor antes de restaurar (os mundos são sobrescritos)"));
        }
        let r = runner::run("bash", &["-c", "tar -xzf \"$1\" -C \"$2\"", "cb-restore", &f, &s.dir], Duration::from_secs(180));
        if r.code != 0 {
            return Ok(json(502, &obj! { "ok" => false, "output" => servers::tail_chars(&r.output(), 300) }));
        }
        audit::audit(cfg, &who, "backup-restaurar", &format!("{} → {}", safe, s.name_str()));
        return Ok(json(200, &obj! { "ok" => true }));
    }

    // ================= conteúdo (mods/plugins via Modrinth) =================
    if p == "/api/content/info" {
        let loader = content::detect_loader(cfg, s);
        let online = ctx::read_props(&s.dir).get("online-mode").and_then(|v| v.as_str()) != Some("false");
        return Ok(json(
            200,
            &obj! {
                "loader" => loader.clone(), "kind" => if loader == "fabric" { "mods" } else { "plugins" },
                "mcVersion" => content::detect_mc_version(cfg, s), "onlineMode" => online,
                "sorts" => content::SORTS.iter().map(|x| Value::from(*x)).collect::<Vec<_>>(),
            },
        ));
    }
    if p == "/api/content/search" {
        let q = req.query_get("q").unwrap_or_default();
        let sort = req.query_get("sort").filter(|x| !x.is_empty()).unwrap_or_else(|| "relevance".into());
        let cat = req.query_get("category").unwrap_or_default();
        let off = req.query_get("offset").filter(|x| !x.is_empty()).unwrap_or_else(|| "0".into());
        let loader = content::detect_loader(cfg, s);
        let mc = content::detect_mc_version(cfg, s);
        return Ok(match content::modrinth_search(&q, &loader, mc.as_deref(), &sort, &cat, &off) {
            Ok(v) => json(200, &v),
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/content/project" {
        let slug = req.query_get("slug").unwrap_or_default();
        if slug.is_empty() {
            return Ok(err(400, "slug vazio"));
        }
        return Ok(match content::modrinth_project(&slug) {
            Ok(v) => json(200, &obj! { "project" => v }),
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/content/versions" {
        let slug = req.query_get("slug").unwrap_or_default();
        if slug.is_empty() {
            return Ok(err(400, "slug vazio"));
        }
        let mc = content::detect_mc_version(cfg, s);
        let vers = content::modrinth_versions(&slug, &content::detect_loader(cfg, s), mc.as_deref());
        return Ok(json(
            200,
            &obj! { "mcVersion" => mc, "versions" => vers.into_iter().map(|v| v.json).collect::<Vec<_>>() },
        ));
    }
    if p == "/api/content/installed" && m == "GET" {
        return Ok(json(200, &content::list_installed(cfg, s)));
    }
    if p == "/api/content/install" && m == "POST" {
        let b = read_body(req);
        let slug = destr(&b, "slug")?.cloned();
        let vid = b.get("versionId").filter(|v| truthy(Some(v))).map(|v| jsutil::to_string(Some(v)));
        if !truthy(slug.as_ref()) {
            return Ok(err(400, "slug vazio"));
        }
        let slug = jsutil::to_string(slug.as_ref());
        let loader = content::detect_loader(cfg, s);
        let mc = content::detect_mc_version(cfg, s);
        return Ok(match content::install_project(s, &slug, &loader, mc.as_deref(), vid.as_deref(), &mut Vec::new(), 0) {
            Ok(installed) => {
                audit::audit(cfg, &who, "mod-install", &format!("{} → {}", slug, s.name_str()));
                json(200, &obj! { "ok" => true, "installed" => installed })
            }
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/content/download-all" && m == "GET" {
        return Ok(match content::mods_zip(cfg, s) {
            Ok((file, size, n)) => {
                audit::audit(cfg, &who, "mods-baixar-zip", &format!("{} mods → {}", n, s.name_str()));
                let safe: String = s.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
                Response {
                    status: 200,
                    headers: vec![
                        ("Content-Type".into(), "application/zip".into()),
                        ("Content-Length".into(), size.to_string()),
                        ("Content-Disposition".into(), format!("attachment; filename=\"{}-mods.zip\"", safe)),
                    ],
                    body: Vec::new(),
                    stream: Some(file),
                }
            }
            Err((code, e)) => err(code, &e),
        });
    }
    if p == "/api/content/upload" && m == "POST" {
        let name = req.query_get("name").unwrap_or_default();
        return Ok(match content::upload_content(cfg, s, &name, &req.body) {
            Ok(v) => {
                audit::audit(cfg, &who, "mod-enviar", &format!("{} → {}", jsutil::to_string(v.get("file")), s.name_str()));
                json(200, &v)
            }
            Err((code, e)) => err(code, &e),
        });
    }
    if p == "/api/modpacks/manual-upload" && m == "POST" {
        let name = req.query_get("name").unwrap_or_default();
        return Ok(match modpacks::manual_mod_upload(&s.dir, &name, &req.body) {
            Ok((file, rest)) => {
                audit::audit(cfg, &who, "mod-manual", &format!("{} → {}", file, s.name_str()));
                json(200, &obj! { "ok" => true, "file" => file, "manualMods" => rest })
            }
            Err((code, e)) => err(code, &e),
        });
    }
    if p == "/api/content/toggle" && m == "POST" {
        let b = read_body(req);
        let file = destr(&b, "file")?;
        let enabled = truthy(b.get("enabled"));
        let file = jsutil::to_string_or_empty(file);
        if file.is_empty() || file.contains('/') || file.contains("..") {
            return Ok(err(400, "arquivo invalido"));
        }
        let folder = content::content_folder(s, &content::detect_loader(cfg, s));
        let base = file.strip_suffix(".disabled").unwrap_or(&file).to_string();
        let active = jsutil::path_join(&[&folder, &base]);
        let off = jsutil::path_join(&[&folder, &format!("{}.disabled", base)]);
        let r = if enabled {
            if std::path::Path::new(&off).exists() { std::fs::rename(&off, &active).map_err(|e| jsutil::fs_err(&e, "rename", &off)) } else { Ok(()) }
        } else if std::path::Path::new(&active).exists() {
            std::fs::rename(&active, &off).map_err(|e| jsutil::fs_err(&e, "rename", &active))
        } else {
            Ok(())
        };
        if let Err(e) = r {
            return Ok(err(500, &e));
        }
        let mut man = content::read_manifest(s);
        let keys: Vec<String> = man.iter().map(|(k, _)| k.clone()).collect();
        for k in keys {
            if let Some(Value::Obj(e)) = man.get_mut(&k) {
                if e.get("filename").and_then(|f| f.as_str()) == Some(base.as_str()) {
                    e.insert("disabled", Value::Bool(!enabled));
                }
            }
        }
        content::write_manifest(s, &man);
        audit::audit(cfg, &who, "mod-toggle", &format!("{} {} → {}", base, if enabled { "ativado" } else { "desativado" }, s.name_str()));
        return Ok(json(200, &obj! { "ok" => true }));
    }
    if p == "/api/content/updates" {
        return Ok(json(200, &obj! { "updates" => content::check_updates(cfg, s) }));
    }
    if p == "/api/content/installed" && m == "DELETE" {
        let file = req.query_get("file").unwrap_or_default();
        let slug = req.query_get("slug").unwrap_or_default();
        let folder = content::content_folder(s, &content::detect_loader(cfg, s));
        let mut man = content::read_manifest(s);
        audit::audit(cfg, &who, "mod-remove", &format!("{} → {}", if slug.is_empty() { &file } else { &slug }, s.name_str()));
        if !slug.is_empty() {
            if let Some(f) = man.get(&slug).and_then(|e| e.get("filename")).filter(|f| truthy(Some(f))) {
                let f = jsutil::to_string(Some(f));
                let _ = std::fs::remove_file(jsutil::path_join(&[&folder, &f]));
                let _ = std::fs::remove_file(jsutil::path_join(&[&folder, &format!("{}.disabled", f)]));
            }
            man.remove(&slug);
            content::write_manifest(s, &man);
            return Ok(json(200, &obj! { "ok" => true }));
        }
        if file.is_empty() || file.contains('/') || file.contains("..") {
            return Ok(err(400, "arquivo invalido"));
        }
        let fp = jsutil::path_join(&[&folder, &file]);
        if let Err(e) = std::fs::remove_file(&fp) {
            return Ok(err(500, &jsutil::fs_err(&e, "unlink", &fp)));
        }
        let base = file.strip_suffix(".disabled").unwrap_or(&file).to_string();
        let keys: Vec<String> = man.iter().map(|(k, _)| k.clone()).collect();
        for k in keys {
            if man.get(&k).and_then(|e| e.get("filename")).and_then(|f| f.as_str()) == Some(base.as_str()) {
                man.remove(&k);
            }
        }
        content::write_manifest(s, &man);
        return Ok(json(200, &obj! { "ok" => true }));
    }

    // ================= compatibilidade =================
    if p == "/api/compat/offline" && m == "POST" {
        let b = read_body(req);
        let enabled = truthy(destr(&b, "enabled")?);
        let pp = jsutil::path_join(&[&s.dir, "server.properties"]);
        ctx::write_props_kv(&s.dir, &[("online-mode", if enabled { "false" } else { "true" })])
            .map_err(|e| jsutil::fs_err(&e, "open", &pp))?;
        audit::audit(cfg, &who, "modo-offline", &format!("{} → {}", if enabled { "ATIVADO" } else { "desativado" }, s.name_str()));
        return Ok(json(200, &obj! { "ok" => true, "onlineMode" => !enabled }));
    }
    if p == "/api/compat/bedrock" && m == "POST" {
        return Ok(match content::install_bedrock(cfg, s) {
            Ok(r) => {
                audit::audit(cfg, &who, "bedrock", &format!("Geyser/Floodgate → {}", s.name_str()));
                json(200, &merge(obj! { "ok" => true }, r))
            }
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/compat/auth" && m == "POST" {
        return Ok(match content::install_player_auth(cfg, s) {
            Ok(r) => {
                let inst: Vec<String> = r
                    .get("installed")
                    .and_then(|a| a.as_arr())
                    .map(|a| a.iter().map(|x| jsutil::to_string(Some(x))).collect())
                    .unwrap_or_default();
                audit::audit(cfg, &who, "login-jogadores", &format!("{} → {}", inst.join(", "), s.name_str()));
                json(200, &merge(obj! { "ok" => true }, r))
            }
            Err(e) => err(502, &e),
        });
    }

    // ================= multi-servidor =================
    if p == "/api/servers" && m == "GET" {
        return Ok(json(200, &servers::list_servers_detailed(st, cfg, &ctx::current_id(cfg, None))));
    }
    if let Some((id, action)) = server_action_route(p) {
        if !ctx::list_instance_ids(cfg).iter().any(|i| i == id) {
            return Ok(err(404, "servidor não existe"));
        }
        let t = ctx::server_ctx(cfg, id);
        // B5 corrigido: a auditoria mantém ip/usuário (o Node registrava "-")
        let who = Who { ip: &rc.ip, user: &rc.user, server: &t.id };
        if action == "status" {
            let (active, players, uptime) = live_state(exec, cfg, &t);
            return Ok(json(
                200,
                &obj! { "id" => id, "name" => t.name.clone(), "active" => active, "uptime" => uptime, "players" => players },
            ));
        }
        let r = if action == "stop" { graceful_stop(exec, cfg, &t) } else { runner::svc_action(exec, cfg, action, &t.service)? };
        let verb = match action {
            "start" => "ligar",
            "stop" => "desligar",
            _ => "reiniciar",
        };
        audit::audit(cfg, &who, verb, &format!("{} ({})", t.name_str(), id));
        if action == "start" && r.code == 0 {
            intg::maybe_start_tunnels(exec, cfg, &t);
        }
        return Ok(run_json(r.code == 0, r.output()));
    }
    if p == "/api/servers/stop-all" && m == "POST" {
        let b = read_body(req);
        let force = truthy(b.get("force"));
        let mut stopped: Vec<Value> = Vec::new();
        let mut errors: Vec<Value> = Vec::new();
        for id in ctx::list_instance_ids(cfg) {
            let t = ctx::server_ctx(cfg, &id);
            let a = runner::svc_active_of(exec, cfg, &t.service);
            if !matches!(a.as_str(), "active" | "activating" | "deactivating" | "reloading") {
                continue;
            }
            // o jeito mais confiável de parar é o `stop` pelo RCON: salva o mundo e sai
            // com 0 (o Minecraft às vezes ignora o SIGTERM logo depois do "Done").
            // Sem RCON (subindo/travado) cai no systemctl stop.
            let r = if force { runner::svc_stop_nowait(exec, cfg, &t.service, true) } else { graceful_stop(exec, cfg, &t) };
            if r.code == 0 {
                stopped.push(Value::from(id.as_str()));
            } else {
                errors.push(obj! { "id" => id.as_str(), "error" => jsutil::trim(&r.output()) });
            }
        }
        let names: Vec<String> = stopped.iter().map(|v| jsutil::to_string(Some(v))).collect();
        audit::audit(
            cfg,
            &who,
            if force { "desligar-tudo-forcado" } else { "desligar-tudo" },
            &if names.is_empty() { "nada rodando".to_string() } else { names.join(", ") },
        );
        let ok = errors.is_empty();
        return Ok(json(if ok { 200 } else { 500 }, &obj! { "ok" => ok, "stopped" => stopped, "errors" => errors, "force" => force }));
    }
    if p == "/api/servers/select" && m == "POST" {
        let b = read_body(req);
        let id = destr(&b, "id")?.cloned();
        if !ctx::multi_enabled(cfg) {
            return Ok(err(400, "multi-servidor desativado"));
        }
        let Some(Value::Str(id)) = id.filter(|i| matches!(i, Value::Str(x) if ctx::list_instance_ids(cfg).contains(x))) else {
            return Ok(err(404, "servidor não existe"));
        };
        st.set_cfg(|c| {
            c.insert("activeServer", Value::from(id.as_str()));
            true
        });
        audit::audit(cfg, &who, "servidor-selecionar", &id);
        return Ok(json(200, &obj! { "ok" => true, "activeId" => id }));
    }
    if p == "/api/servers/create" && m == "POST" {
        let b = read_body(req);
        let name = prop(&b, "name")?;
        if !truthy(name) || jsutil::trim(&jsutil::to_string(name)).is_empty() {
            return Ok(err(400, "dê um nome ao servidor"));
        }
        let name = jsutil::to_string(name);
        let loader = b.get("loader").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let version = jsutil::to_string_or_empty(b.get("version"));
        return Ok(match servers::create_instance(st, cfg, &name, &loader, &version) {
            Ok(sv) => {
                audit::audit(
                    cfg,
                    &who,
                    "servidor-criar",
                    &format!(
                        "{} ({} {})",
                        jsutil::to_string(sv.get("name")),
                        jsutil::to_string(sv.get("loader")),
                        jsutil::to_string_or_empty(sv.get("mcVersion"))
                    ),
                );
                json(200, &obj! { "ok" => true, "server" => sv })
            }
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/servers/clone" && m == "POST" {
        let b = read_body(req);
        let id = prop(&b, "id")?.cloned().unwrap_or(Value::Undef);
        return Ok(match servers::clone_instance(cfg, &id, b.get("name")) {
            Ok(sv) => {
                audit::audit(cfg, &who, "servidor-clonar", &format!("{} → {}", jsutil::to_string(Some(&id)), jsutil::to_string(sv.get("name"))));
                json(200, &obj! { "ok" => true, "server" => sv })
            }
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/modpacks/sources" && m == "GET" {
        return Ok(json(
            200,
            &obj! {
                "modrinth" => true, "curseforge" => !modpacks::cf_key(cfg).is_empty(),
                "curseforgeFromEnv" => std::env::var("CRAFTBOX_CURSEFORGE_KEY").is_ok_and(|v| !v.is_empty()),
            },
        ));
    }
    if p == "/api/modpacks/curseforge-key" && m == "POST" {
        if !auth::is_admin(cfg, auth::session_user(cfg, cookie).as_deref()) {
            return Ok(err(403, "só um admin pode configurar a chave"));
        }
        let b = read_body(req);
        let key = jsutil::trim(&jsutil::to_string_or_empty(prop(&b, "key")?)).to_string();
        if !key.is_empty() {
            if let Err(e) = modpacks::cf_validate_key(cfg, &key) {
                let msg = if e.need_key {
                    "chave inválida — confira no console.curseforge.com".to_string()
                } else {
                    format!("não deu pra validar: {}", e.msg)
                };
                return Ok(err(400, &msg));
            }
        }
        st.set_cfg(|c| {
            c.insert("curseforgeApiKey", Value::from(key.as_str()));
            true
        });
        audit::audit(cfg, &who, "curseforge-chave", if key.is_empty() { "removida" } else { "configurada" });
        return Ok(json(200, &obj! { "ok" => true, "curseforge" => !key.is_empty() }));
    }
    if p == "/api/modpacks/search" {
        let src = req.query_get("source").filter(|x| !x.is_empty()).unwrap_or_else(|| "modrinth".into());
        let q = req.query_get("q").unwrap_or_default();
        let loader = req.query_get("loader").unwrap_or_default();
        let off = req.query_get("offset").filter(|x| !x.is_empty()).unwrap_or_else(|| "0".into());
        let r = if src == "curseforge" {
            modpacks::cf_modpack_search(cfg, &q, &loader, &off)
        } else {
            modpacks::modpack_search(&q, &loader, &off).map_err(modpacks::CfErr::from)
        };
        return Ok(match r {
            Ok(v) => json(200, &v),
            Err(e) => json(if e.need_key { 400 } else { 502 }, &obj! { "error" => e.msg, "needKey" => e.need_key }),
        });
    }
    if p == "/api/modpacks/versions" {
        let slug = req.query_get("slug").unwrap_or_default();
        if slug.is_empty() {
            return Ok(err(400, "slug vazio"));
        }
        let r = if req.query_get("source").as_deref() == Some("curseforge") {
            modpacks::cf_modpack_versions(cfg, &slug)
        } else {
            modpacks::modpack_versions(&slug).map_err(modpacks::CfErr::from)
        };
        return Ok(match r {
            Ok(v) => json(200, &obj! { "versions" => v }),
            Err(e) => json(if e.need_key { 400 } else { 502 }, &obj! { "error" => e.msg, "needKey" => e.need_key }),
        });
    }
    if p == "/api/servers/create-progress" && m == "GET" {
        return Ok(json(200, &servers::progress_get().map(Value::Obj).unwrap_or(obj! { "phase" => Value::Null })));
    }
    if p == "/api/servers/create-modpack" && m == "POST" {
        let b = read_body(req);
        let slug = prop(&b, "slug")?.cloned();
        if !truthy(slug.as_ref()) {
            return Ok(err(400, "modpack não informado"));
        }
        if servers::progress_active() {
            return Ok(err(409, "já tem uma instalação em andamento — espere terminar"));
        }
        let slug = jsutil::to_string(slug.as_ref());
        let name = jsutil::to_string_or_empty(b.get("name"));
        let vid = b.get("versionId").filter(|v| truthy(Some(v))).map(|v| jsutil::to_string(Some(v)));
        let r = if b.get("source").and_then(|x| x.as_str()) == Some("curseforge") {
            modpacks::create_from_curseforge(st, cfg, &name, &slug, vid.as_deref()).map_err(|e| e.msg)
        } else {
            modpacks::create_from_modpack(st, cfg, &name, &slug, vid.as_deref())
        };
        return Ok(match r {
            Ok(sv) => {
                let n = sv.get("strippedMods").and_then(|a| a.as_arr()).map(|a| a.len()).unwrap_or(0);
                let extra = if n > 0 { format!(" · {} mods client-only desativados", n) } else { String::new() };
                audit::audit(cfg, &who, "servidor-modpack", &format!("{} ({}){}", jsutil::to_string(sv.get("name")), slug, extra));
                json(200, &obj! { "ok" => true, "server" => sv })
            }
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/servers" && m == "DELETE" {
        let id = req.query_get("id").unwrap_or_default();
        return Ok(match servers::delete_instance(st, cfg, &id) {
            Ok(r) => {
                audit::audit(cfg, &who, "servidor-apagar", &id);
                json(200, &r)
            }
            Err(e) => err(500, &e),
        });
    }

    // ================= integrações (playit / tailscale / cloudflare) =================
    if p == "/api/integrations" && m == "GET" {
        return Ok(json(200, &intg::integrations_status(exec, cfg, s)));
    }
    if p.starts_with("/api/integrations/") && matches!(&p[18..], "install" | "start" | "stop") && m == "POST" {
        let b = read_body(req);
        let name = destr(&b, "name")?;
        let n = jsutil::to_string(name);
        let nm = if matches!(name, Some(Value::Str(_))) { n.as_str() } else { "" };
        let (r, act, fail) = match &p[18..] {
            "install" => (intg::integration_install(cfg, nm), "integração-instalar", 502),
            "start" => (intg::integration_start(exec, cfg, s, nm), "integração-ligar", 500),
            _ => (intg::integration_stop(exec, cfg, s, nm), "integração-desligar", 500),
        };
        return Ok(match r {
            Ok(v) => {
                audit::audit(cfg, &who, act, &n);
                json(200, &v)
            }
            Err(e) => err(fail, &e),
        });
    }
    if p == "/api/integrations/log" && m == "GET" {
        let name = req.query_get("name").unwrap_or_default();
        let Some(unit) = intg::intg_unit(cfg, s, &name) else { return Ok(err(400, "nome inválido")) };
        return Ok(json(200, &obj! { "log" => intg::u_journal(exec, cfg, &unit, 120) }));
    }
    // ================= rede / wi-fi (nmcli) =================
    if p == "/api/net" && m == "GET" {
        return Ok(json(200, &intg::net_status()));
    }
    if p == "/api/net/scan" && m == "GET" {
        return Ok(json(200, &obj! { "networks" => intg::wifi_scan() }));
    }
    if p == "/api/net/wifi" && m == "POST" {
        let b = read_body(req);
        let ssid = destr(&b, "ssid")?;
        return Ok(match intg::wifi_connect(ssid, b.get("password")) {
            Ok(v) => {
                audit::audit(cfg, &who, "wifi-conectar", &jsutil::to_string(ssid));
                json(200, &v)
            }
            Err(e) => err(502, &e),
        });
    }
    // ================= extras =================
    if p == "/api/extras" && m == "GET" {
        return Ok(json(200, &obj! { "show" => cfg.get("showExtras") != Some(&Value::Bool(false)) }));
    }
    if p == "/api/extras" && m == "POST" {
        let b = read_body(req);
        let show = truthy(prop(&b, "show")?);
        st.set_cfg(|c| {
            c.insert("showExtras", Value::Bool(show));
            true
        });
        audit::audit(cfg, &who, "config", &format!("detalhes extras: {}", if show { "visíveis" } else { "ocultos" }));
        return Ok(json(200, &obj! { "show" => show }));
    }
    if p == "/api/worldsize" && m == "GET" {
        return Ok(json(200, &servers::world_size(s)));
    }
    if p == "/api/diag/mojang" && m == "GET" {
        return Ok(json(200, &diag::diag_mojang(&s.dir)));
    }
    if p == "/api/speedtest" && (m == "POST" || m == "GET") {
        return Ok(match intg::speed_test() {
            Ok(v) => json(200, &v),
            Err(e) => err(502, &e),
        });
    }
    if p == "/api/integrations/cloudflare-token" && m == "POST" {
        let b = read_body(req);
        let token = destr(&b, "token")?;
        let t = jsutil::trim(&jsutil::to_string_or_empty(token)).to_string();
        if !truthy(token) || t.is_empty() {
            return Ok(err(400, "token vazio"));
        }
        let f = jsutil::path_join(&[&intg::intg_srv_dir(cfg, s), "cloudflared.token"]);
        write_secret(&f, &t)?;
        return Ok(json(200, &obj! { "ok" => true }));
    }
    if p == "/api/integrations/cloudflare-conf" && m == "POST" {
        let b = read_body(req);
        let on = prop(&b, "onStart")?.cloned();
        let mut meta = ctx::read_instance_meta(&s.dir);
        if let Some(Value::Bool(v)) = on {
            meta.insert("cfOnStart", Value::Bool(v));
            ctx::write_instance_meta(&s.dir, &meta);
        }
        if let Some(h) = b.get("hostname").filter(|h| !matches!(h, Value::Null)) {
            let f = jsutil::path_join(&[&intg::intg_srv_dir(cfg, s), "cloudflared.host"]);
            write_secret(&f, jsutil::trim(&jsutil::to_string(Some(h))))?;
        }
        return Ok(json(200, &obj! { "ok" => true }));
    }
    if p == "/api/integrations/playit-conf" && m == "POST" {
        let b = read_body(req);
        let on = truthy(prop(&b, "onStart")?);
        let mut meta = ctx::read_instance_meta(&s.dir);
        meta.insert("playitOnStart", Value::Bool(on));
        ctx::write_instance_meta(&s.dir, &meta);
        return Ok(json(200, &obj! { "ok" => true, "onStart" => on }));
    }
    if p == "/api/integrations/playit-secret" && m == "POST" {
        let b = read_body(req);
        let secret = jsutil::trim(&jsutil::to_string_or_empty(destr(&b, "secret")?)).to_string();
        if secret.len() < 16 || !secret.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Ok(err(400, "secret inválido (é uma sequência hexadecimal do playit.gg)"));
        }
        let f = jsutil::path_join(&[&intg::intg_srv_dir(cfg, s), "playit.toml"]);
        write_secret(&f, &format!("secret_key = \"{}\"\n", secret))?;
        return Ok(json(200, &obj! { "ok" => true }));
    }

    // ================= conta / acesso =================
    if p == "/api/change-password" && m == "POST" {
        return change_password(st, cfg, req, &who, cookie);
    }
    if p == "/api/users" && m == "GET" {
        let me = auth::session_user(cfg, cookie);
        let users: Vec<Value> = config::users(cfg)
            .iter()
            .map(|u| obj! { "user" => u.fld("user"), "role" => if truthy(u.get("role")) { u.fld("role") } else { Value::from("user") } })
            .collect();
        return Ok(json(
            200,
            &obj! { "multiUser" => auth::multi_user(cfg), "me" => me.clone(), "isAdmin" => auth::is_admin(cfg, me.as_deref()), "users" => users },
        ));
    }
    if p == "/api/users" && m == "POST" {
        return users_create(st, cfg, req, &who, cookie);
    }
    if p == "/api/users" && m == "DELETE" {
        if !auth::is_admin(cfg, auth::session_user(cfg, cookie).as_deref()) {
            return Ok(err(403, "só um admin pode gerenciar usuários"));
        }
        let name = req.query_get("user").unwrap_or_default();
        let users = config::users(cfg);
        let Some(idx) = find_user_idx(&users, &name) else { return Ok(err(404, "usuário não existe")) };
        let is_adm = |u: &Value| u.get("role").and_then(|r| r.as_str()) == Some("admin");
        if is_adm(&users[idx]) && users.iter().filter(|u| is_adm(u)).count() <= 1 {
            return Ok(err(400, "não dá pra remover o último admin"));
        }
        st.set_cfg(|c| {
            let mut us = config::users(c);
            if let Some(i) = find_user_idx(&us, &name) {
                us.remove(i);
            }
            c.insert("users", Value::Arr(us));
            true
        });
        audit::audit(cfg, &who, "usuario-remover", &name);
        return Ok(json(200, &obj! { "ok" => true }));
    }
    // ================= conector MCP (tokens) =================
    if p == "/api/mcp/tokens" {
        if !auth::is_admin(cfg, auth::session_user(cfg, cookie).as_deref()) {
            return Ok(err(403, "só um admin pode gerenciar o conector MCP"));
        }
        if m == "GET" {
            return Ok(json(200, &obj! { "tokens" => crate::mcp::tokens_public(cfg) }));
        }
        if m == "POST" {
            let b = read_body(req);
            let name = jsutil::trim(&jsutil::to_string_or_empty(b.get("name"))).to_string();
            let name = if name.is_empty() { "Claude".to_string() } else { name.chars().take(40).collect() };
            let owner = auth::session_user(cfg, cookie).unwrap_or_else(|| "admin".into());
            let (token, public) = crate::mcp::token_create(st, &name, &owner);
            audit::audit(cfg, &who, "mcp-token-criar", &name);
            return Ok(json(200, &obj! { "ok" => true, "token" => token, "entry" => public }));
        }
        if m == "DELETE" {
            let id = req.query_get("id").unwrap_or_default();
            if !crate::mcp::token_delete(st, &id) {
                return Ok(err(404, "token não existe"));
            }
            audit::audit(cfg, &who, "mcp-token-revogar", &id);
            return Ok(json(200, &obj! { "ok" => true }));
        }
    }
    // ================= auditoria / histórico =================
    // ================= atualização do sistema (craftbox-update) =================
    if p == "/api/system/update" && m == "GET" {
        // o updater só existe no appliance (ISO); no Docker/rootless não há o que atualizar por aqui
        if exec || config::systemctl_user(cfg) || !std::path::Path::new(UPDATER).exists() {
            return Ok(json(200, &obj! { "supported" => false, "reason" => "a atualização pelo painel só existe no craftbox instalado pela ISO" }));
        }
        let r = runner::run("sudo", &["-n", UPDATER, "--check", "--json"], Duration::from_secs(40));
        let parsed = json::parse(jsutil::trim(&r.stdout)).ok();
        return Ok(match parsed {
            Some(Value::Obj(o)) if r.code == 0 => json(200, &merge(obj! { "supported" => true, "running" => update_running() }, Value::Obj(o))),
            Some(Value::Obj(o)) => json(502, &Value::Obj(o)),
            _ => err(502, &format!("não consegui checar: {}", jsutil::trim(&r.output()))),
        });
    }
    if p == "/api/system/update" && m == "POST" {
        if !auth::is_admin(cfg, auth::session_user(cfg, cookie).as_deref()) {
            return Ok(err(403, "só um admin pode atualizar o sistema"));
        }
        if update_running() {
            return Ok(err(409, "já tem uma atualização rodando"));
        }
        // roda fora do cgroup do painel: no fim o updater reinicia o próprio painel
        let r = runner::run(
            "sudo",
            &["-n", "/usr/bin/systemd-run", "--unit=craftbox-update", "--collect", "--no-block", UPDATER, "--yes"],
            runner::RUN_TIMEOUT,
        );
        if r.code != 0 {
            return Ok(err(500, &format!("não consegui iniciar a atualização: {}", jsutil::trim(&r.output()))));
        }
        audit::audit(cfg, &who, "atualizar-sistema", "craftbox-update iniciado");
        return Ok(json(200, &obj! { "ok" => true }));
    }
    if p == "/api/system/update/log" && m == "GET" {
        let txt = std::fs::read(UPDATE_LOG).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        // só a última execução (cada uma começa com "=== craftbox-update")
        let last = txt.rfind("=== craftbox-update").map(|i| &txt[i..]).unwrap_or("");
        let lines: Vec<&str> = last.lines().collect();
        let tail = lines[lines.len().saturating_sub(300)..].join("\n");
        return Ok(json(200, &obj! { "running" => update_running(), "log" => tail }));
    }
    if p == "/api/audit" && m == "DELETE" {
        let _ = std::fs::write(audit::audit_path(cfg), "");
        audit::audit(cfg, &who, "historico-limpo", "histórico apagado pelo painel");
        return Ok(json(200, &obj! { "ok" => true }));
    }
    if p == "/api/audit" && m == "GET" {
        let n = query_lines(req, "300", 2000.0);
        let mut items: Vec<Value> = Vec::new();
        if let Ok(b) = std::fs::read(audit::audit_path(cfg)) {
            let txt = String::from_utf8_lossy(&b);
            let lines: Vec<&str> = jsutil::trim(&txt).split('\n').collect();
            for l in js_tail(&lines, n).iter().rev() {
                if let Ok(v) = json::parse(l) {
                    if truthy(Some(&v)) {
                        items.push(v);
                    }
                }
            }
        }
        return Ok(json(200, &obj! { "items" => items }));
    }

    Ok(err(404, "endpoint desconhecido"))
}

/// `{ ok: true, ...r }`
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Obj(mut x), Value::Obj(y)) => {
            for (k, v) in y.iter() {
                x.insert(k.clone(), v.clone());
            }
            Value::Obj(x)
        }
        (a, _) => a,
    }
}

/// `fs.writeFileSync(f, txt); chmod 600` (erro de escrita vira 500 com a mensagem do Node)
fn write_secret(f: &str, txt: &str) -> Result<(), String> {
    std::fs::write(f, txt).map_err(|e| jsutil::fs_err(&e, "open", f))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

fn find_user_idx(users: &[Value], name: &str) -> Option<usize> {
    let n = name.to_lowercase();
    users.iter().position(|u| jsutil::to_string_or_empty(u.get("user")).to_lowercase() == n)
}

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn change_password(st: &State, cfg: &Map, req: &Request, who: &Who, cookie: &str) -> Result<Response, String> {
    let b = read_body(req);
    let current = destr(&b, "current")?.cloned();
    let np = jsutil::to_string_or_empty(b.get("newPassword"));
    if js_len(&np) < 4 {
        return Ok(err(400, "a nova senha precisa de pelo menos 4 caracteres"));
    }
    if auth::multi_user(cfg) {
        let me = auth::session_user(cfg, cookie).unwrap_or_default();
        let users = config::users(cfg);
        let Some(idx) = find_user_idx(&users, &me) else { return Ok(err(401, "sessão inválida")) };
        let u = &users[idx];
        let uname = jsutil::to_string(u.get("user"));
        if !auth::check_password_js(current.as_ref(), u.get("salt"), u.get("hash")) {
            audit::audit(cfg, who, "senha-troca-falha", &uname);
            return Ok(err(401, "senha atual incorreta"));
        }
        let (salt, hash) = crypto::hash_password(&np);
        st.set_cfg(|c| {
            let mut us = config::users(c);
            if let Some(Value::Obj(o)) = us.get_mut(idx) {
                o.insert("salt", Value::from(salt.as_str()));
                o.insert("hash", Value::from(hash.as_str()));
            }
            c.insert("users", Value::Arr(us));
            true
        });
        audit::audit(cfg, who, "senha-alterada", &uname);
        return Ok(json(200, &obj! { "ok" => true }));
    }
    let salt = config::sub(cfg, "auth", "salt");
    let hash = config::sub(cfg, "auth", "hash");
    if !truthy(salt) || !truthy(hash) {
        return Ok(err(400, "senha ainda não configurada"));
    }
    if !auth::check_password_js(current.as_ref(), salt, hash) {
        audit::audit(cfg, who, "senha-troca-falha", "senha atual incorreta");
        return Ok(err(401, "senha atual incorreta"));
    }
    let (salt, hash) = crypto::hash_password(&np);
    st.set_cfg(|c| {
        c.insert("auth", obj! { "salt" => salt.as_str(), "hash" => hash.as_str() });
        true
    });
    audit::audit(cfg, who, "senha-alterada", "senha do painel trocada");
    Ok(json(200, &obj! { "ok" => true }))
}

fn users_create(st: &State, cfg: &Map, req: &Request, who: &Who, cookie: &str) -> Result<Response, String> {
    if !auth::is_admin(cfg, auth::session_user(cfg, cookie).as_deref()) {
        return Ok(err(403, "só um admin pode gerenciar usuários"));
    }
    let b = read_body(req);
    let user = destr(&b, "user")?;
    let name = jsutil::trim(&jsutil::to_string_or_empty(user)).to_string();
    // /^[\w.@+-]{2,32}$/
    let ok = (2..=32).contains(&js_len(&name))
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'@' | b'+' | b'-'));
    if !ok {
        return Ok(err(400, "usuário inválido (2-32 caracteres: letras, números, . _ @ + -)"));
    }
    let pw = jsutil::to_string_or_empty(b.get("password"));
    if js_len(&pw) < 4 {
        return Ok(err(400, "senha mínima de 4 caracteres"));
    }
    if find_user_idx(&config::users(cfg), &name).is_some() {
        return Ok(err(409, "usuário já existe"));
    }
    let role_admin = b.get("role").and_then(|r| r.as_str()) == Some("admin");
    let (salt, hash) = crypto::hash_password(&pw);
    let mut first = false;
    st.set_cfg(|c| {
        let mut us = match c.get("users") {
            Some(Value::Arr(a)) => a.clone(),
            _ => Vec::new(),
        };
        first = us.is_empty();
        let role = if first || role_admin { "admin" } else { "user" };
        us.push(obj! { "user" => name.as_str(), "salt" => salt.as_str(), "hash" => hash.as_str(), "role" => role });
        c.insert("users", Value::Arr(us));
        true
    });
    let mut resp = json(200, &obj! { "ok" => true, "firstUser" => first });
    // ao criar o 1º usuário, migra a sessão atual pra ele (pra não trancar o acesso)
    if first {
        resp = resp.header_first("Set-Cookie", &auth::session_cookie(&config::s(cfg, "sessionSecret"), &name));
    }
    audit::audit(cfg, who, "usuario-criar", &format!("{}{}", name, if first { " (admin, 1º usuário)" } else { "" }));
    Ok(resp)
}

fn login(cfg: &Map, req: &Request, rc: &ReqCtx) -> Response {
    let body = read_body(req);
    let secret = config::s(cfg, "sessionSecret");
    let who = rc.who();
    if auth::multi_user(cfg) {
        if body.is_null() {
            return err(500, "Cannot read properties of null (reading 'user')");
        }
        let u = auth::find_user(cfg, body.get("user"));
        let ok =
            u.as_ref().is_some_and(|u| auth::check_password_js(body.get("password"), u.get("salt"), u.get("hash")));
        if !ok {
            let who_str = if truthy(body.get("user")) { jsutil::to_string(body.get("user")) } else { "?".into() };
            audit::audit(cfg, &who, "login-falha", &format!("usuário {}", who_str));
            return err(401, "usuário ou senha incorretos");
        }
        let u = u.unwrap();
        let name = jsutil::to_string_or_empty(u.get("user"));
        audit::audit(cfg, &who, "login", &format!("entrou ({})", jsutil::to_string(u.get("user"))));
        return json(200, &obj! { "ok" => true, "user" => u.get("user").cloned() })
            .header_first("Set-Cookie", &auth::session_cookie(&secret, &name));
    }
    let salt = config::sub(cfg, "auth", "salt");
    let hash = config::sub(cfg, "auth", "hash");
    if !truthy(salt) || !truthy(hash) {
        return err(500, "Senha nao configurada. Rode: node server.js --hash SENHA");
    }
    if body.is_null() {
        return err(500, "Cannot read properties of null (reading 'password')");
    }
    if !auth::check_password_js(body.get("password"), salt, hash) {
        audit::audit(cfg, &who, "login-falha", "");
        return err(401, "Senha incorreta");
    }
    audit::audit(cfg, &who, "login", "entrou no painel");
    json(200, &obj! { "ok" => true }).header_first("Set-Cookie", &auth::session_cookie(&secret, "admin"))
}

/// `autostartServers()` (runner exec + multi): liga as instâncias com
/// `autostart` no meta (ou as listadas em CRAFTBOX_AUTOSTART) e os túneis delas.
pub fn autostart_servers(st: &State) {
    let cfg = st.cfg();
    if !st.exec_runner || !ctx::multi_enabled(&cfg) {
        return;
    }
    let want: Vec<String> = std::env::var("CRAFTBOX_AUTOSTART")
        .unwrap_or_default()
        .split(',')
        .map(|x| jsutil::trim(x).to_string())
        .filter(|x| !x.is_empty())
        .collect();
    for id in ctx::list_instance_ids(&cfg) {
        let meta = ctx::read_instance_meta(&jsutil::path_join(&[&config::s(&cfg, "serversDir"), &id]));
        let on = if want.is_empty() { truthy(meta.get("autostart")) } else { want.contains(&id) };
        if !on {
            continue;
        }
        let s = ctx::server_ctx(&cfg, &id);
        if runner::svc_active_of(true, &cfg, &s.service) == "active" {
            continue;
        }
        if let Ok(r) = runner::svc_action(true, &cfg, "start", &s.service) {
            if r.code == 0 {
                intg::maybe_start_tunnels(true, &cfg, &s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_action_paths() {
        assert_eq!(server_action_route("/api/servers/alpha/status"), Some(("alpha", "status")));
        assert_eq!(server_action_route("/api/servers/alpha/explode"), None);
        assert_eq!(server_action_route("/api/servers//start"), None);
        assert_eq!(server_action_route("/api/servers/a/start/x"), None);
    }

    #[test]
    fn js_slice_semantics() {
        let v = ["a", "b", "c", "d"];
        assert_eq!(js_tail(&v, Some(2.0)), ["c", "d"]);
        assert_eq!(js_tail(&v, None), v); // NaN → tudo (bug B7 reproduzido)
        assert_eq!(js_tail(&v, Some(0.0)), v); // slice(-0) = tudo
        assert_eq!(js_tail(&v, Some(-1.0)), ["b", "c", "d"]); // slice(1)
        assert_eq!(js_tail(&v, Some(10.0)), v);
    }

    #[test]
    fn static_mime_and_404() {
        let d = std::env::temp_dir().join(format!("cbrs-pub-{}", std::process::id()));
        std::fs::create_dir_all(d.join("logos")).unwrap();
        std::fs::write(d.join("app.js"), "x").unwrap();
        let pubdir = d.to_str().unwrap();
        let r = serve_static(pubdir, "app.js");
        assert_eq!(r.status, 200);
        assert_eq!(r.headers[0].1, "text/javascript; charset=utf-8");
        assert_eq!(serve_static(pubdir, "nada.css").status, 404);
        assert_eq!(serve_static(pubdir, "logos").status, 404); // diretório → EISDIR → 404
        assert_eq!(serve_static(pubdir, "").status, 404);
        let _ = std::fs::remove_dir_all(&d);
    }
}
