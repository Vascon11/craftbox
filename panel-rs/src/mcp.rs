//! Conector MCP (Model Context Protocol) do painel: `POST /mcp` no transporte
//! "Streamable HTTP", sem sessão e só com respostas JSON (sem SSE), pra usar o
//! craftbox a partir do Claude Code / Claude Desktop.
//!
//! Autenticação: `Authorization: Bearer cbx_...`. Os tokens são criados por um
//! admin no painel (`/api/mcp/tokens`) e ficam no config só como SHA-256.
//!
//! Cada ferramenta vira uma requisição interna às rotas `/api/*` que já
//! existem, com uma sessão do usuário que criou o token: mesmas validações,
//! mesmas mensagens de erro e mesma trilha na auditoria (ip "… via MCP").

use crate::auth;
use crate::config;
use crate::crypto;
use crate::http::{Request, Response};
use crate::json::{self, Map, Value};
use crate::jsutil::{self, now_ms, truthy};
use crate::obj;
use crate::routes;
use crate::State;

const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const TOKEN_PREFIX: &str = "cbx_";

const INSTRUCTIONS: &str = "Painel do craftbox, um appliance de servidores Minecraft. \
Cada servidor tem um id (veja list_servers); o parâmetro `server` das ferramentas é esse id e, \
se omitido, vale o servidor selecionado no painel. A máquina costuma ter pouca RAM: rode um \
servidor por vez (desligue um antes de ligar outro). Ligar um modpack grande leva de 1 a 3 minutos; \
acompanhe com server_status/get_logs. Comandos de console (run_command) são comandos do Minecraft \
sem a barra inicial.";

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

fn sha256_hex(s: &str) -> String {
    crypto::hex(ring::digest::digest(&ring::digest::SHA256, s.as_bytes()).as_ref())
}

fn tokens(cfg: &Map) -> Vec<Value> {
    cfg.get("mcpTokens").and_then(|v| v.as_arr()).cloned().unwrap_or_default()
}

/// Token → entrada do config (a comparação do hash é em tempo constante).
fn find_token(cfg: &Map, bearer: &str) -> Option<Map> {
    if !bearer.starts_with(TOKEN_PREFIX) {
        return None;
    }
    let h = sha256_hex(bearer);
    tokens(cfg).into_iter().find_map(|t| match t {
        Value::Obj(m) if crypto::ct_eq(jsutil::to_string_or_empty(m.get("hash")).as_bytes(), h.as_bytes()) => Some(m),
        _ => None,
    })
}

/// Lista pública (sem o hash).
pub fn tokens_public(cfg: &Map) -> Value {
    Value::Arr(
        tokens(cfg)
            .iter()
            .map(|t| {
                obj! {
                    "id" => t.fld("id"), "name" => t.fld("name"), "user" => t.fld("user"),
                    "createdAt" => t.fld("createdAt"), "lastUsedAt" => t.get("lastUsedAt").cloned().unwrap_or(Value::Null),
                }
            })
            .collect(),
    )
}

/// Cria um token pro `user`; devolve (token em claro — só aparece aqui, entrada pública).
pub fn token_create(st: &State, name: &str, user: &str) -> (String, Value) {
    let token = format!("{}{}", TOKEN_PREFIX, crypto::random_hex(24));
    let id = crypto::random_hex(4);
    let entry = obj! {
        "id" => id.clone(), "name" => name, "user" => user, "hash" => sha256_hex(&token), "createdAt" => now_ms(),
    };
    st.set_cfg(|c| {
        let mut ts = tokens(c);
        ts.push(entry);
        c.insert("mcpTokens", Value::Arr(ts));
        true
    });
    let public = obj! { "id" => id, "name" => name, "user" => user, "createdAt" => now_ms(), "lastUsedAt" => Value::Null };
    (token, public)
}

/// Revoga pelo id; false se não existia.
pub fn token_delete(st: &State, id: &str) -> bool {
    let mut found = false;
    st.set_cfg(|c| {
        let ts = tokens(c);
        let n = ts.len();
        let rest: Vec<Value> = ts.into_iter().filter(|t| t.get("id").and_then(|x| x.as_str()) != Some(id)).collect();
        found = rest.len() != n;
        if found {
            c.insert("mcpTokens", Value::Arr(rest));
        }
        found
    });
    found
}

/// Marca uso (no máximo 1 gravação do config por minuto por token).
fn touch_token(st: &State, id: &str) {
    let now = now_ms();
    st.set_cfg(|c| {
        let mut ts = tokens(c);
        let mut changed = false;
        for t in ts.iter_mut() {
            if let Value::Obj(m) = t {
                if m.get("id").and_then(|x| x.as_str()) == Some(id) {
                    let last = m.get("lastUsedAt").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    if now - last > 60_000.0 {
                        m.insert("lastUsedAt", Value::Num(now));
                        changed = true;
                    }
                }
            }
        }
        if changed {
            c.insert("mcpTokens", Value::Arr(ts));
        }
        changed
    });
}

// ---------------------------------------------------------------------------
// Ferramentas
// ---------------------------------------------------------------------------

struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// propriedades do inputSchema: (nome, json do schema, obrigatório)
    props: &'static [(&'static str, &'static str, bool)],
    read_only: bool,
    destructive: bool,
}

const SERVER: (&str, &str, bool) =
    ("server", r#"{"type":"string","description":"id do servidor (veja list_servers); omitido = o selecionado no painel"}"#, false);
const SERVER_REQ: (&str, &str, bool) = ("server", r#"{"type":"string","description":"id do servidor (veja list_servers)"}"#, true);

const TOOLS: &[Tool] = &[
    Tool {
        name: "list_servers",
        title: "Listar servidores",
        description: "Lista os servidores Minecraft da máquina: id, nome, loader (paper/fabric/forge/…), versão, porta, se está ligado, modpack e mods pendentes de download manual.",
        props: &[],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "server_status",
        title: "Status do servidor",
        description: "Estado de um servidor (active/inactive/failed), tempo ligado, jogadores online e uso da máquina (CPU, RAM, disco, temperatura).",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "power_server",
        title: "Ligar/desligar servidor",
        description: "Liga, desliga ou reinicia um servidor. Desligar salva o mundo antes de sair. Com pouca RAM, desligue o servidor que estiver ligado antes de ligar outro.",
        props: &[SERVER_REQ, ("action", r#"{"type":"string","enum":["start","stop","restart"]}"#, true)],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "run_command",
        title: "Comando no console",
        description: "Executa um comando no console do servidor via RCON e devolve a resposta (ex.: \"list\", \"say oi\", \"op Fulano\", \"time set day\"). Sem a barra inicial. O servidor precisa estar ligado.",
        props: &[SERVER, ("command", r#"{"type":"string","description":"comando do Minecraft, sem '/'"}"#, true)],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "get_logs",
        title: "Ler o log",
        description: "Últimas linhas do logs/latest.log do servidor (padrão 200, máximo 1000). Útil pra ver se subiu (\"Done (\"), erros e quem entrou.",
        props: &[SERVER, ("lines", r#"{"type":"integer","minimum":1,"maximum":1000}"#, false)],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "get_properties",
        title: "Ler server.properties",
        description: "Lê o server.properties do servidor (motd, max-players, difficulty, view-distance, online-mode, whitelist…).",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "set_properties",
        title: "Alterar server.properties",
        description: "Altera chaves do server.properties (as outras ficam como estão). Vale depois de reiniciar o servidor.",
        props: &[
            SERVER,
            ("properties", r#"{"type":"object","description":"chave → valor, ex. {\"max-players\":\"10\",\"difficulty\":\"hard\"}","additionalProperties":{"type":["string","number","boolean"]}}"#, true),
        ],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "list_mods",
        title: "Mods/plugins instalados",
        description: "Lista os mods (Fabric/Forge/NeoForge) ou plugins (Paper) instalados no servidor, com estado ativado/desativado.",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "search_mods",
        title: "Buscar mods no Modrinth",
        description: "Busca mods/plugins no Modrinth compatíveis com o loader e a versão do servidor. Use o slug do resultado em install_mod.",
        props: &[
            SERVER,
            ("query", r#"{"type":"string"}"#, true),
            ("sort", r#"{"type":"string","enum":["relevance","downloads","follows","newest","updated"]}"#, false),
            ("offset", r#"{"type":"integer","minimum":0}"#, false),
        ],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "install_mod",
        title: "Instalar mod/plugin",
        description: "Instala um mod/plugin do Modrinth (com as dependências obrigatórias) na versão compatível com o servidor. Vale depois de reiniciar.",
        props: &[SERVER, ("slug", r#"{"type":"string","description":"slug ou id do projeto no Modrinth"}"#, true), ("versionId", r#"{"type":"string","description":"versão específica (opcional)"}"#, false)],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "toggle_mod",
        title: "Ativar/desativar mod",
        description: "Ativa ou desativa um mod/plugin pelo nome do arquivo (list_mods mostra os nomes). Desativar renomeia pra .disabled; vale depois de reiniciar.",
        props: &[SERVER, ("file", r#"{"type":"string"}"#, true), ("enabled", r#"{"type":"boolean"}"#, true)],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "remove_mod",
        title: "Remover mod/plugin",
        description: "Apaga um mod/plugin do servidor, pelo slug (instalado pelo painel) ou pelo nome do arquivo.",
        props: &[SERVER, ("slug", r#"{"type":"string"}"#, false), ("file", r#"{"type":"string"}"#, false)],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "check_mod_updates",
        title: "Atualizações de mods",
        description: "Verifica no Modrinth se há versões mais novas dos mods/plugins instalados pelo painel.",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "list_backups",
        title: "Listar backups",
        description: "Lista os backups do mundo do servidor (nome, tamanho em MB, data).",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "create_backup",
        title: "Fazer backup",
        description: "Faz um backup .tar.gz do mundo agora (mantém os 7 mais recentes).",
        props: &[SERVER],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "create_server",
        title: "Criar servidor",
        description: "Cria um servidor novo. loader: paper (plugins), fabric/quilt/forge/neoforge (mods) ou vanilla. version vazia = a mais recente. Forge/NeoForge levam alguns minutos; acompanhe com get_create_progress.",
        props: &[
            ("name", r#"{"type":"string"}"#, true),
            ("loader", r#"{"type":"string","enum":["paper","fabric","quilt","forge","neoforge","vanilla"]}"#, true),
            ("version", r#"{"type":"string","description":"versão do Minecraft, ex. 1.21.1 (opcional)"}"#, false),
        ],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "get_create_progress",
        title: "Progresso da criação",
        description: "Progresso da criação de servidor/modpack em andamento (fase, arquivos baixados). phase null = nada em andamento.",
        props: &[],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "get_network_info",
        title: "Endereços de acesso",
        description: "Como os jogadores chegam no servidor: IP do Tailscale, endereço do playit.gg e do túnel Cloudflare, se configurados.",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "world_size",
        title: "Tamanho do mundo",
        description: "Espaço ocupado pelo mundo, backups e pela pasta do servidor.",
        props: &[SERVER],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "get_audit_log",
        title: "Histórico do painel",
        description: "Histórico de ações feitas no painel e pelo MCP (quem ligou/desligou, instalou mods, mudou config…), mais recentes primeiro.",
        props: &[("lines", r#"{"type":"integer","minimum":1,"maximum":2000}"#, false)],
        read_only: true,
        destructive: false,
    },
];

fn tool_json(t: &Tool) -> Value {
    let mut props = Map::new();
    let mut req: Vec<Value> = Vec::new();
    for (k, schema, required) in t.props {
        props.insert(*k, json::parse(schema).expect("schema de ferramenta inválido"));
        if *required {
            req.push(Value::from(*k));
        }
    }
    let mut schema = obj! { "type" => "object", "properties" => props };
    if let Value::Obj(m) = &mut schema {
        if !req.is_empty() {
            m.insert("required", Value::Arr(req));
        }
    }
    obj! {
        "name" => t.name, "title" => t.title, "description" => t.description, "inputSchema" => schema,
        "annotations" => obj! {
            "title" => t.title, "readOnlyHint" => t.read_only, "destructiveHint" => t.destructive,
            "openWorldHint" => false,
        },
    }
}

/// Uma chamada interna: método, caminho (sem query), query já montada e corpo JSON.
struct Call {
    method: &'static str,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
}

fn s_arg(a: &Map, k: &str) -> Option<String> {
    a.get(k).filter(|v| !matches!(v, Value::Null | Value::Undef)).map(|v| jsutil::to_string(Some(v)))
}

fn plan(name: &str, a: &Map) -> Result<Call, String> {
    let mut q: Vec<(String, String)> = Vec::new();
    if let Some(s) = s_arg(a, "server").filter(|s| !s.is_empty()) {
        q.push(("server".into(), s));
    }
    let need = |k: &str| s_arg(a, k).filter(|s| !s.is_empty()).ok_or_else(|| format!("parâmetro obrigatório: {}", k));
    let get = |path: &str, q: Vec<(String, String)>| Call { method: "GET", path: path.into(), query: q, body: None };
    let post = |path: &str, q: Vec<(String, String)>, b: Value| Call { method: "POST", path: path.into(), query: q, body: Some(b) };
    Ok(match name {
        "list_servers" => get("/api/servers", q),
        "server_status" => get("/api/status", q),
        "power_server" => {
            let id = need("server")?;
            let action = need("action")?;
            if !matches!(action.as_str(), "start" | "stop" | "restart") {
                return Err("action deve ser start, stop ou restart".into());
            }
            Call { method: "POST", path: format!("/api/servers/{}/{}", jsutil::encode_uri_component(&id), action), query: vec![], body: None }
        }
        "run_command" => {
            let c = need("command")?;
            post("/api/rcon", q, obj! { "command" => c.trim_start_matches('/') })
        }
        "get_logs" => {
            if let Some(n) = s_arg(a, "lines") {
                q.push(("lines".into(), n));
            }
            get("/api/logs", q)
        }
        "get_properties" => get("/api/properties", q),
        "set_properties" => {
            let Some(Value::Obj(p)) = a.get("properties") else { return Err("properties deve ser um objeto chave → valor".into()) };
            Call { method: "PUT", path: "/api/properties".into(), query: q, body: Some(Value::Obj(p.clone())) }
        }
        "list_mods" => get("/api/content/installed", q),
        "search_mods" => {
            q.push(("q".into(), need("query")?));
            for k in ["sort", "offset"] {
                if let Some(v) = s_arg(a, k) {
                    q.push((k.into(), v));
                }
            }
            get("/api/content/search", q)
        }
        "install_mod" => {
            let mut b = obj! { "slug" => need("slug")? };
            if let (Value::Obj(m), Some(v)) = (&mut b, s_arg(a, "versionId")) {
                m.insert("versionId", Value::from(v));
            }
            post("/api/content/install", q, b)
        }
        "toggle_mod" => {
            let enabled = a.get("enabled").cloned().ok_or("parâmetro obrigatório: enabled")?;
            post("/api/content/toggle", q, obj! { "file" => need("file")?, "enabled" => truthy(Some(&enabled)) })
        }
        "remove_mod" => {
            let (slug, file) = (s_arg(a, "slug").unwrap_or_default(), s_arg(a, "file").unwrap_or_default());
            if slug.is_empty() && file.is_empty() {
                return Err("informe slug ou file".into());
            }
            if !slug.is_empty() {
                q.push(("slug".into(), slug));
            } else {
                q.push(("file".into(), file));
            }
            Call { method: "DELETE", path: "/api/content/installed".into(), query: q, body: None }
        }
        "check_mod_updates" => get("/api/content/updates", q),
        "list_backups" => get("/api/backups", q),
        "create_backup" => post("/api/backups", q, obj! {}),
        "create_server" => {
            let mut b = obj! { "name" => need("name")?, "loader" => need("loader")? };
            if let (Value::Obj(m), Some(v)) = (&mut b, s_arg(a, "version")) {
                m.insert("version", Value::from(v));
            }
            post("/api/servers/create", vec![], b)
        }
        "get_create_progress" => get("/api/servers/create-progress", vec![]),
        "get_network_info" => get("/api/integrations", q),
        "world_size" => get("/api/worldsize", q),
        "get_audit_log" => {
            let mut q = vec![];
            if let Some(n) = s_arg(a, "lines") {
                q.push(("lines".into(), n));
            }
            get("/api/audit", q)
        }
        _ => return Err(format!("ferramenta desconhecida: {}", name)),
    })
}

/// Executa a chamada nas rotas do painel como o dono do token.
fn dispatch(st: &State, cfg: &Map, user: &str, ip: &str, c: Call) -> (u16, Value) {
    let query = c
        .query
        .iter()
        .map(|(k, v)| format!("{}={}", jsutil::encode_uri_component(k), jsutil::encode_uri_component(v)))
        .collect::<Vec<_>>()
        .join("&");
    let cookie = format!("cbsession={}", auth::make_session(&config::s(cfg, "sessionSecret"), user));
    let body = c.body.map(|b| json::stringify(&b).into_bytes()).unwrap_or_default();
    let req = Request {
        method: c.method.into(),
        target: if query.is_empty() { c.path.clone() } else { format!("{}?{}", c.path, query) },
        path: c.path,
        query,
        headers: vec![("cookie".into(), cookie), ("content-type".into(), "application/json".into())],
        body,
        ip: format!("{} via MCP", ip),
    };
    let resp = routes::handle(st, &req);
    let v = json::parse(&String::from_utf8_lossy(&resp.body)).unwrap_or_else(|_| Value::from(String::from_utf8_lossy(&resp.body).into_owned()));
    (resp.status, v)
}

fn call_tool(st: &State, cfg: &Map, user: &str, ip: &str, params: &Value) -> Value {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = match params.get("arguments") {
        Some(Value::Obj(m)) => m.clone(),
        _ => Map::new(),
    };
    let (is_err, payload) = match plan(name, &args) {
        Err(e) => (true, obj! { "error" => e }),
        Ok(c) => {
            let (status, v) = dispatch(st, cfg, user, ip, c);
            // as rotas de ação respondem {ok:false, output} com 500 quando o systemctl falha
            let failed = status >= 400 || v.get("ok") == Some(&Value::Bool(false));
            (failed, if status >= 400 { obj! { "status" => status as f64, "error" => v.get("error").cloned().unwrap_or(v) } } else { v })
        }
    };
    let text = json::stringify_pretty(&payload);
    let mut r = obj! { "content" => vec![obj! { "type" => "text", "text" => text }], "isError" => is_err };
    if let (Value::Obj(m), Value::Obj(_)) = (&mut r, &payload) {
        m.insert("structuredContent", payload.clone());
    }
    r
}

// ---------------------------------------------------------------------------
// JSON-RPC
// ---------------------------------------------------------------------------

fn rpc_result(id: &Value, result: Value) -> Value {
    obj! { "jsonrpc" => "2.0", "id" => id.clone(), "result" => result }
}

fn rpc_error(id: &Value, code: f64, msg: &str) -> Value {
    obj! { "jsonrpc" => "2.0", "id" => id.clone(), "error" => obj! { "code" => code, "message" => msg } }
}

/// Processa uma mensagem; None para notificações (não têm resposta).
fn handle_message(st: &State, cfg: &Map, user: &str, ip: &str, msg: &Value) -> Option<Value> {
    let Value::Obj(_) = msg else { return Some(rpc_error(&Value::Null, -32600.0, "requisição inválida")) };
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned()?; // sem id = notificação (initialized, cancelled…)
    let params = msg.get("params").cloned().unwrap_or(Value::Obj(Map::new()));
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let pv = if PROTOCOLS.contains(&asked) { asked } else { DEFAULT_PROTOCOL };
            rpc_result(
                &id,
                obj! {
                    "protocolVersion" => pv,
                    "capabilities" => obj! { "tools" => obj! { "listChanged" => false } },
                    "serverInfo" => obj! { "name" => "craftbox", "title" => "craftbox", "version" => env!("CARGO_PKG_VERSION") },
                    "instructions" => INSTRUCTIONS,
                },
            )
        }
        "ping" => rpc_result(&id, obj! {}),
        "tools/list" => rpc_result(&id, obj! { "tools" => TOOLS.iter().map(tool_json).collect::<Vec<_>>() }),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !TOOLS.iter().any(|t| t.name == name) {
                rpc_error(&id, -32602.0, &format!("ferramenta desconhecida: {}", name))
            } else {
                rpc_result(&id, call_tool(st, cfg, user, ip, &params))
            }
        }
        "resources/list" => rpc_result(&id, obj! { "resources" => Vec::<Value>::new() }),
        "prompts/list" => rpc_result(&id, obj! { "prompts" => Vec::<Value>::new() }),
        _ => rpc_error(&id, -32601.0, &format!("método não suportado: {}", method)),
    })
}

fn unauthorized(msg: &str) -> Response {
    routes::json(401, &obj! { "error" => msg }).header_first("WWW-Authenticate", "Bearer realm=\"craftbox\"")
}

/// `/mcp`: POST com JSON-RPC; GET/DELETE → 405 (sem stream SSE nem sessão).
pub fn handle(st: &State, req: &Request) -> Response {
    if req.method != "POST" {
        return routes::json(405, &obj! { "error" => "use POST (transporte Streamable HTTP, só JSON)" }).header_first("Allow", "POST");
    }
    let cfg = st.cfg();
    let bearer = req.header("authorization").unwrap_or_default();
    let tok = bearer.strip_prefix("Bearer ").or_else(|| bearer.strip_prefix("bearer ")).unwrap_or("").trim();
    let Some(entry) = find_token(&cfg, tok) else {
        return unauthorized(if tok.is_empty() { "token ausente (Authorization: Bearer cbx_…)" } else { "token inválido ou revogado" });
    };
    let user = jsutil::to_string_or_empty(entry.get("user"));
    // o dono do token precisa continuar existindo (multiusuário) — senão o token morre junto
    if auth::multi_user(&cfg) && auth::find_user(&cfg, Some(&Value::from(user.as_str()))).is_none() {
        return unauthorized("o usuário dono deste token não existe mais");
    }
    touch_token(st, &jsutil::to_string_or_empty(entry.get("id")));
    let who = if user.is_empty() { "admin".to_string() } else { user };
    let msg = match json::parse(&String::from_utf8_lossy(&req.body)) {
        Ok(v) => v,
        Err(_) => return routes::json(400, &rpc_error(&Value::Null, -32700.0, "JSON inválido")),
    };
    let out = match &msg {
        Value::Arr(batch) => {
            let rs: Vec<Value> = batch.iter().filter_map(|m| handle_message(st, &cfg, &who, &req.ip, m)).collect();
            if rs.is_empty() {
                None
            } else {
                Some(Value::Arr(rs))
            }
        }
        m => handle_message(st, &cfg, &who, &req.ip, m),
    };
    match out {
        Some(v) => routes::json(200, &v),
        None => Response { status: 202, headers: vec![("Content-Length".into(), "0".into())], body: Vec::new(), stream: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        for t in TOOLS {
            let j = tool_json(t);
            assert_eq!(j.get("inputSchema").and_then(|s| s.get("type")).and_then(|v| v.as_str()), Some("object"), "{}", t.name);
        }
        let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOOLS.len(), "nomes de ferramenta repetidos");
    }

    #[test]
    fn plan_maps_arguments() {
        let mut a = Map::new();
        a.insert("server", Value::from("cw"));
        a.insert("command", Value::from("/say oi"));
        let c = plan("run_command", &a).unwrap();
        assert_eq!((c.method, c.path.as_str()), ("POST", "/api/rcon"));
        assert_eq!(c.query, vec![("server".to_string(), "cw".to_string())]);
        assert_eq!(c.body.unwrap().get("command").and_then(|v| v.as_str()), Some("say oi"));

        let mut a = Map::new();
        a.insert("server", Value::from("a b"));
        a.insert("action", Value::from("stop"));
        assert_eq!(plan("power_server", &a).unwrap().path, "/api/servers/a%20b/stop");
        a.insert("action", Value::from("rm -rf"));
        assert!(plan("power_server", &a).is_err());
        assert!(plan("remove_mod", &Map::new()).is_err());
        assert!(plan("nada", &Map::new()).is_err());
    }

    #[test]
    fn token_hash_lookup() {
        let mut cfg = Map::new();
        let tok = "cbx_abc";
        cfg.insert("mcpTokens", Value::Arr(vec![obj! { "id" => "1", "user" => "admin", "hash" => sha256_hex(tok) }]));
        assert!(find_token(&cfg, tok).is_some());
        assert!(find_token(&cfg, "cbx_abd").is_none());
        assert!(find_token(&cfg, "abc").is_none());
    }
}
