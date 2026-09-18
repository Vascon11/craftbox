//! Roteador: mesma ordem de decisão do `http.createServer` do server.js.
//!
//!  1. /api/login, /api/logout, /api/authcheck (sem sessão)
//!  2. estáticos (/, /public/*, /app.js, /style.css, /logo.png, /favicon.png, /logos/*)
//!  3. /api/* exige sessão → 401 {"error":"nao autenticado"}
//!  4. rotas migradas; rotas do Node ainda não migradas → 501; resto → 404 JSON
//!  5. fora de /api → 404 "not found" (texto puro)

use crate::audit::{self, Who};
use crate::auth;
use crate::config;
use crate::ctx::{self, Srv};
use crate::diag;
use crate::http::{Request, Response};
use crate::json::{self, Map, Value};
use crate::jsutil;
use crate::obj;
use crate::rcon;
use crate::runner;
use crate::stats;
use crate::State;

pub fn json(code: u16, v: &Value) -> Response {
    let body = json::stringify(v);
    Response {
        status: code,
        headers: vec![
            ("Content-Type".into(), "application/json; charset=utf-8".into()),
            ("Content-Length".into(), body.len().to_string()),
        ],
        body: body.into_bytes(),
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
        },
        Err(_) => Response::plain(404, "not found"),
    }
}

// ---------------------------------------------------------------------------
// Rotas do server.js ainda não migradas (respondem 501 depois da autenticação).
// A lista é o inventário executável do ROUTES.md; "*" = qualquer método.
// ---------------------------------------------------------------------------
pub const PENDING: &[(&str, &str)] = &[
    ("POST", "/api/power"),
    ("GET", "/api/properties"),
    ("PUT", "/api/properties"),
    ("POST", "/api/rcon"),
    ("*", "/api/logs"),
    ("GET", "/api/backups"),
    ("POST", "/api/backups"),
    ("GET", "/api/backups/download"),
    ("POST", "/api/backups/restore"),
    ("*", "/api/content/info"),
    ("*", "/api/content/search"),
    ("*", "/api/content/project"),
    ("*", "/api/content/versions"),
    ("GET", "/api/content/installed"),
    ("POST", "/api/content/install"),
    ("POST", "/api/content/toggle"),
    ("*", "/api/content/updates"),
    ("DELETE", "/api/content/installed"),
    ("POST", "/api/compat/offline"),
    ("POST", "/api/compat/bedrock"),
    ("POST", "/api/compat/auth"),
    ("GET", "/api/servers"),
    ("*", "/api/servers/:id/(start|stop|restart|status)"),
    ("POST", "/api/servers/select"),
    ("POST", "/api/servers/create"),
    ("POST", "/api/servers/clone"),
    ("*", "/api/modpacks/search"),
    ("*", "/api/modpacks/versions"),
    ("GET", "/api/servers/create-progress"),
    ("POST", "/api/servers/create-modpack"),
    ("DELETE", "/api/servers"),
    ("GET", "/api/integrations"),
    ("POST", "/api/integrations/install"),
    ("POST", "/api/integrations/start"),
    ("POST", "/api/integrations/stop"),
    ("GET", "/api/integrations/log"),
    ("GET", "/api/net"),
    ("GET", "/api/net/scan"),
    ("POST", "/api/net/wifi"),
    ("GET", "/api/extras"),
    ("POST", "/api/extras"),
    ("GET", "/api/worldsize"),
    ("POST", "/api/speedtest"),
    ("GET", "/api/speedtest"),
    ("POST", "/api/integrations/cloudflare-token"),
    ("POST", "/api/integrations/cloudflare-conf"),
    ("POST", "/api/integrations/playit-conf"),
    ("POST", "/api/integrations/playit-secret"),
    ("POST", "/api/change-password"),
    ("GET", "/api/users"),
    ("POST", "/api/users"),
    ("DELETE", "/api/users"),
    ("DELETE", "/api/audit"),
    ("GET", "/api/audit"),
];

fn server_action_route(p: &str) -> bool {
    // ^/api/servers/([^/]+)/(start|stop|restart|status)$
    let Some(rest) = p.strip_prefix("/api/servers/") else { return false };
    let mut it = rest.split('/');
    matches!((it.next(), it.next(), it.next()), (Some(id), Some("start" | "stop" | "restart" | "status"), None) if !id.is_empty())
}

pub fn is_pending(method: &str, p: &str) -> bool {
    server_action_route(p) || PENDING.iter().any(|(m, path)| *path == p && (*m == "*" || *m == method))
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------
pub fn handle(st: &State, req: &Request) -> Response {
    let cfg = st.cfg();
    let cookie = req.header("cookie").unwrap_or_default();
    let srv = ctx::server_ctx(&cfg, &ctx::current_id(&cfg, req.query_get("server").as_deref()));
    let rc = ReqCtx { srv, ip: req.ip.clone(), user: auth::session_user(&cfg, &cookie).unwrap_or_else(|| "-".into()) };
    match route(st, &cfg, req, &rc, &cookie) {
        Ok(r) => r,
        Err(msg) => err(500, &msg),
    }
}

fn route(st: &State, cfg: &Map, req: &Request, rc: &ReqCtx, cookie: &str) -> Result<Response, String> {
    let p = req.path.as_str();
    let m = req.method.as_str();

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
            || (jsutil::truthy(config::sub(cfg, "auth", "salt")) && jsutil::truthy(config::sub(cfg, "auth", "hash")));
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

    if p == "/api/status" {
        return Ok(status(st, cfg, rc));
    }
    if p == "/api/diag/mojang" && m == "GET" {
        return Ok(json(200, &diag::diag_mojang(&rc.srv.dir)));
    }

    if is_pending(m, p) {
        return Ok(err(501, "rota ainda não migrada para o backend Rust"));
    }
    Ok(err(404, "endpoint desconhecido"))
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
            let who_str =
                if jsutil::truthy(body.get("user")) { jsutil::to_string(body.get("user")) } else { "?".into() };
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
    if !jsutil::truthy(salt) || !jsutil::truthy(hash) {
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

/// `GET /api/status` (qualquer método, como no Node).
fn status(st: &State, cfg: &Map, rc: &ReqCtx) -> Response {
    let active = runner::svc_active_of(st.exec_runner, cfg, &rc.srv.service);
    let mut players = Value::Null;
    if active == "active" {
        if let Ok(list) = rcon::command(cfg, &rc.srv, "list") {
            if let Some((online, max)) = rcon::parse_players(&list) {
                players = obj! { "online" => online, "max" => max };
            }
        }
    }
    let uptime = runner::svc_uptime(st.exec_runner, cfg, &rc.srv.service);
    json(
        200,
        &obj! {
            "active" => active, "uptime" => uptime, "players" => players,
            "system" => stats::system_stats(&rc.srv.dir),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_table() {
        assert!(is_pending("POST", "/api/power"));
        assert!(!is_pending("GET", "/api/power"));
        assert!(is_pending("PATCH", "/api/logs"));
        assert!(is_pending("GET", "/api/servers/alpha/status"));
        assert!(!is_pending("GET", "/api/servers/alpha/explode"));
        assert!(!is_pending("GET", "/api/servers//start"));
        assert!(!is_pending("GET", "/api/status"));
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
