# Contrato do `panel/server.js` — inventário para a migração em Rust

Este documento descreve **tudo o que o frontend (e quem mais fala com o painel) pode observar** do backend Node atual, para que o `panel-rs` reproduza o mesmo contrato. Referência: `panel/server.js` (~2000 linhas; tem um byte NUL intencional em `nmParse`, use `grep -a`).

Status de cada rota: **FEITA** (migrada e coberta pelo teste de paridade) ou **pendente** (no Rust responde `501 {"error":"rota ainda não migrada para o backend Rust"}` depois da autenticação). A lista de pendentes também existe como código em `src/routes.rs` (`PENDING`).

---

## 1. Configuração

### 1.1 Arquivo

- Caminho: `CRAFTBOX_PANEL_CONFIG`, senão `<dir do server.js>/config.json`. No Rust: `CRAFTBOX_PANEL_CONFIG`, senão `<dir do binário>/config.json`.
- Carga (`loadConfig`): `{ ...DEFAULTS, ...arquivo, rcon: {...DEFAULTS.rcon, ...arquivo.rcon}, auth: {...DEFAULTS.auth, ...arquivo.auth} }`. O merge é **raso**, e as **chaves desconhecidas são preservadas** e regravadas. Se o arquivo faltar ou for inválido, valem os defaults. Se o arquivo for o JSON `null`, também valem os defaults.
- Gravação (`saveConfig`): `JSON.stringify(CONFIG, null, 2)`, **sem newline final**, com os erros ignorados. Grava o objeto em memória inteiro, **incluindo os overrides de env** (ver 1.3).
- Se `sessionSecret` estiver vazio na subida, é gerado (32 bytes aleatórios em hex, ou seja, 64 caracteres) e o arquivo é regravado na hora.

### 1.2 Chaves (ordem = ordem de gravação)

| chave | default | uso |
|---|---|---|
| `port` | `8080` | porta do HTTP |
| `host` | `"0.0.0.0"` | bind |
| `mcDir` | `"/opt/minecraft"` | modo legado (1 servidor): pasta do servidor |
| `service` | `"minecraft"` | modo legado: unidade systemd |
| `rcon` | `{host:"127.0.0.1", port:25575, password:""}` | fallback do RCON (senha vazia = ler do server.properties) |
| `auth` | `{salt:"", hash:""}` | senha única (modo legado de login) |
| `users` | `[]` | contas `[{user, salt, hash, role:"admin"\|"user"}]`; não vazio = multiusuário |
| `sessionSecret` | `""` | chave do HMAC da sessão (gerada se vazia) |
| `loader` | `""` | modo legado: `paper`\|`fabric` (auto-detecta se vazio) |
| `mcVersion` | `""` | modo legado: versão do MC (auto-detecta do log se vazio) |
| `serversDir` | `""` | se definido, **ativa o multi-servidor** (cada subpasta = 1 instância) |
| `serviceTemplate` | `"minecraft@"` | unidade = `serviceTemplate + id` |
| `activeServer` | `""` | instância selecionada (gravada pelo painel) |
| `systemctlUser` | `false` | `systemctl --user` (rootless, sem sudo) |
| `integrationsDir` | `""` | binários playit/cloudflared; vazio = automático |
| `runDir` | `""` | runner exec: PID files e logs; vazio = automático |
| `showExtras` | *(ausente)* | criada por `POST /api/extras`; `!== false` = mostrar |
| `auditLog` | *(ausente)* | caminho do log de auditoria; ausente = automático |

Caminhos derivados:
- `auditPath` = `auditLog`, senão `dirname(serversDir)/craftbox-audit.log` (com multi), senão `~/craftbox-audit.log`.
- `runDir` = `runDir`, senão `dirname(serversDir)/craftbox-run`, senão `~/.craftbox-run`. **É criado (`mkdir -p`) em toda leitura.**
- `integrationsDir` = `integrationsDir`, senão `dirname(serversDir)/craftbox-integrations`, senão `~/.craftbox-integrations`. Também é criado em toda leitura; com multi, há uma subpasta por instância.

### 1.3 Variáveis de ambiente

| env | efeito |
|---|---|
| `CRAFTBOX_PANEL_CONFIG` | caminho do config.json |
| `CRAFTBOX_PORT` | `parseInt(v,10)`; se `> 0`, sobrescreve `port` (e é persistido se o config for regravado) |
| `CRAFTBOX_SERVERS_DIR` | sobrescreve `serversDir` (idem) |
| `CRAFTBOX_INTEGRATIONS_DIR` | sobrescreve `integrationsDir` |
| `CRAFTBOX_RUN_DIR` | sobrescreve `runDir` |
| `CRAFTBOX_RUNNER` | `exec` força o runner de processos (ver 3) |
| `CRAFTBOX_AUTOSTART` | lista `id1,id2` de instâncias que sobem com o painel (só no runner exec) |
| `CRAFTBOX_JAVA_MAJOR` | **setada pelo painel** nos filhos do runner exec (8/17/21 conforme `mcVersion`) |
| `CRAFTBOX_PASSWORD` | não é lida pelo server.js; quem lê é o init do container (`docker/`) |
| `HOME` | base dos caminhos automáticos (`os.homedir()`) e de `~/.config/systemd/user` |
| `CRAFTBOX_PUBLIC_DIR` | **só no Rust**: pasta do frontend (padrão `<dir do binário>/public`) |

Env vazia é ignorada (`if (process.env.X)`).

### 1.4 Arquivos por instância (multi-servidor)

`<serversDir>/<id>/`: `.craftbox-instance.json` (meta: `name, loader, mcVersion, port, createdAt, modpack, strippedMods, loaderVersion, rconPort, rconPassword, autostart, playitOnStart, cfOnStart`), `server.properties`, `.craftbox-loader`, `.craftbox-content.json` (manifesto de mods), `start.sh`, `backup.sh`, `eula.txt`, `backups/*.tar.gz`, `logs/latest.log`, `mods/` ou `plugins/`.

Contexto por requisição (`SRV()`): a instância vem de `?server=<id>` se existir; senão de `activeServer` se existir; senão da primeira (ordem alfabética); senão `default`. No modo legado é sempre `{id:'default', dir: mcDir, service, name:'Servidor'}`. O frontend acrescenta `?server=` em toda chamada `/api/*`, exceto `login`, `logout`, `authcheck` e `/api/servers*`.

---

## 2. Autenticação, sessão e cookie

- **Hash de senha**: `crypto.scryptSync(senha, salt, 64)` com os defaults do Node (N=16384, r=8, p=1). O `salt` são 16 bytes em hex, usados **como texto** (bytes UTF-8 da string hex). A comparação é em tempo constante. Com salt ou hash vazios, a senha é sempre recusada.
- **Token**: `valor.mac`, com `valor = "s:<Date.now()+12h em ms>:<encodeURIComponent(usuário)>"` e `mac = base64url(HMAC-SHA256(sessionSecret, valor))`. No modo de senha única o usuário é `admin`.
- **Cookie de login**: `Set-Cookie: cbsession=<token>; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`.
- **Cookie de logout**: `Set-Cookie: cbsession=; HttpOnly; Path=/; Max-Age=0`.
- **Leitura**: `req.headers.cookie.split(';')`, depois `indexOf('=')`, `trim` e `decodeURIComponent(valor)`. Nomes repetidos: vale o último. O token é válido se o MAC confere e `parseInt(valor.split(':')[1]) > Date.now()`.
- **Sem sessão** em qualquer `/api/*` que não seja login/logout/authcheck: `401 {"error":"nao autenticado"}`.
- **Modos**: `users` vazio = senha única (`auth.salt/hash`), e todo mundo é admin. `users` com itens = multiusuário: login por `{user, password}`, `user` sem diferenciar maiúsculas, e `isAdmin` = `role === 'admin'`.
- Como o segredo e o formato são os mesmos, **um cookie emitido pelo Node vale no Rust e vice-versa** (verificado no teste de paridade). Isso permite trocar o backend sem deslogar ninguém.

## 3. Gerenciador de serviços: systemd × runner exec

Decidido **uma vez na subida**: `CRAFTBOX_RUNNER=exec` **ou** ausência de `/run/systemd/system` liga o runner exec.

| operação | systemd | runner exec |
|---|---|---|
| estado | `systemctl [--user] is-active <svc>` → stdout (vazio = `unknown`) | `kill(pid,0)` do PID file → `active`/`inactive` |
| uptime | `systemctl [--user] show <svc> --property=ActiveEnterTimestamp --value` → `Date.parse` | linha 2 do PID file (epoch ms) |
| start/stop/restart | `systemctl --user <a> <svc>` (rootless) ou `sudo -n systemctl <a> <svc>` | spawn `/bin/bash -lc "bash start.sh"` destacado, PID file `<runDir>/<unit>.pid` (`pid\nstartedAt\n`), log `<runDir>/<unit>.log`; stop = SIGTERM, até 12,5 s, depois SIGKILL |
| logs de túnel | `journalctl --user -u <unit>` | cauda de `<runDir>/<unit>.log` |
| daemon-reload | `systemctl --user daemon-reload` | no-op |

`run()` = `execFile` com timeout de 15 s (padrão), `maxBuffer` de 256 MiB, sem rejeitar. Timeout = SIGTERM.

**Detalhe do uptime no systemd**: o `Date.parse` do V8 entende `"Thu 2026-09-18 17:02:03 -03"` e `UTC`/`GMT`/`EST`…, mas **não** abreviações como `BRT` ou `CEST`: nesses fusos o uptime sai `null`. O Rust reproduz isso (`runner::parse_systemd_timestamp`, com testes).

## 4. Ciclo de vida

- **Subida**: `listen(port, host)`; `server.requestTimeout = 0`. Imprime `craftbox-panel ouvindo em http://host:port` e o aviso de senha não configurada. No runner exec, roda `autostartServers()`: para cada instância com `meta.autostart` (ou listada em `CRAFTBOX_AUTOSTART`), sobe o servidor e os túneis marcados com "ligar junto".
- **Shutdown gracioso** (SIGTERM/SIGINT): no runner exec, `unitStop` em toda unidade com `.pid` no `runDir`, **em série**; depois `process.exit(0)`. No systemd só sai.
  - Rust: **FEITO** (thread com `sigwait`; mesma sequência SIGTERM → 25×500 ms → SIGKILL). O autostart está **pendente** (depende do `unitStart`, da fatia 1); o Rust registra um aviso no log.
- **CLI**: `--hash SENHA` imprime `{salt, hash}`; `--init` cria o config.json (erro se já existir). Rust: **FEITO** (e também `--version`).

## 5. HTTP (enquadramento observado)

- JSON: `Content-Type: application/json; charset=utf-8` + `Content-Length`; `Date`, `Connection: keep-alive`, `Keep-Alive: timeout=5`. `Set-Cookie`, quando existe, vem antes dos outros.
- Estáticos e 404 em texto (`writeHead` + `end`): **sem** Content-Length, com `Transfer-Encoding: chunked`. Em HEAD, sem corpo nem framing.
- Requisição malformada: `400 Bad Request` + `Connection: close`. Corpo lido por `readBody`: `JSON.parse(corpo || '{}')` ou `{}` se inválido; acima de ~1e6 caracteres a conexão é destruída.
- Pathname vem de `new URL(req.url, 'http://localhost')`: os segmentos `.`/`..` (inclusive `%2e`) são resolvidos e **nada é decodificado**.
- Exceção dentro do handler vira `500 {"error": e.message}`.

---

## 6. Rotas

Legenda: **Auth** = exige sessão; `*` = qualquer método (o Node não confere). "Front" = usada pelo `app.js` atual.

### 6.1 Públicas (sem sessão)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| POST | `/api/login` | senha única: `{password}`; multi: `{user, password}` | `200 {ok:true}` (multi: `{ok:true, user}`) + Set-Cookie · `401 {error:"Senha incorreta"}` / `{error:"usuário ou senha incorretos"}` · `500 {error:"Senha nao configurada. Rode: node server.js --hash SENHA"}` · corpo `null` → `500 {error:"Cannot read properties of null (reading 'password'\|'user')"}` | scrypt (~16 MiB de RAM por tentativa); audit `login` / `login-falha` | **FEITA** |
| POST | `/api/logout` | — | `200 {ok:true}` + cookie de logout | — | **FEITA** |
| * | `/api/authcheck` | cookie | `200 {authed, configured, multiUser, user\|null, isAdmin}` | — | **FEITA** |
| * | `/` | — | `index.html` | leitura de arquivo | **FEITA** |
| * | `/public/<caminho>` | — | arquivo de `public/` com MIME; `404 "not found"` (texto); `403 "forbidden"` (inalcançável, porque o pathname já vem normalizado) | leitura | **FEITA** |
| * | `/app.js`, `/style.css`, `/logo.png`, `/favicon.png`, `/logos/*` | — | idem | leitura | **FEITA** |
| * | qualquer outra fora de `/api/` | — | `404 "not found"` (texto) | — | **FEITA** |

MIME (sensível a maiúsculas, via `path.extname`): `.html` `text/html; charset=utf-8`, `.js` `text/javascript; charset=utf-8`, `.css` `text/css; charset=utf-8`, `.svg` `image/svg+xml`, `.ico` `image/x-icon`, `.png` `image/png`, `.jpg`/`.jpeg` `image/jpeg`, `.webp` `image/webp`; o resto é `application/octet-stream`.

### 6.2 Painel e servidor (Auth)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| * | `/api/status` | `?server=` | `200 {active, uptime\|null, players:{online,max}\|null, system:{load:[3], cpus, cpu:{curMHz,maxMHz}, tempC, mem:{total,free,used}, disk:{totalGB,usedGB,freeGB,low,lowGB}\|null, uptimeHost}}` | is-active + show (ou PID file); se `active`, RCON `list` (TCP, 6 s); `statfs(dir)` com **bavail**; `/proc`/`/sys` | **FEITA** |
| GET | `/api/diag/mojang` | `?server=` | `200 {ok, onlineMode:true\|false\|null, targets:[{name, url, ok, status, ms, error?, code?}]}` | 2 GETs HTTPS em paralelo (sessionserver `hasJoined`, esperado 204; `api.minecraftservices.com/`); qualquer resposta HTTP = ok; redirect não é seguido; 6 s por alvo; falha = `{ok:false, status:null, ms, error:"CODE: msg", code}` | **FEITA** |
| POST | `/api/power` | `{action: start\|stop\|restart}` | `200/500 {ok, output}`; ação inválida → `500 {error:"acao invalida"}` | systemctl/sudo ou spawn/kill; audit `power`; no start, sobe os túneis com "ligar junto" | pendente (não usada pelo front, que chama `/api/servers/:id/*`) |
| GET | `/api/properties` | — | `{properties:{...}}` | lê server.properties | pendente |
| PUT | `/api/properties` | `{chave: valor}` | `{ok, properties}`; `400 {error:"payload invalido"}` | reescreve server.properties preservando comentários e acrescentando chaves novas; audit `config` | pendente |
| POST | `/api/rcon` | `{command}` | `{response}`; `400 {error:"comando vazio"}`; `502 {error}` | TCP RCON | pendente |
| * | `/api/logs` | `?lines=` (padrão 200, máx 1000) | `{log}` | `logs/latest.log`; fallback: log do runner ou `journalctl -u <svc>` (**de sistema**, mesmo em rootless) | pendente |
| GET | `/api/backups` | — | `{backups:[{name,sizeMB,mtime}], totalMB}` (mais novo primeiro; `totalMB` é a soma crua, sem arredondar) | lista `backups/*.tar.gz` | pendente |
| POST | `/api/backups` | — | `200/500 {ok, output}` | `bash backup.sh` (120 s): tar do mundo e mantém os 7 últimos; audit `backup` | pendente |
| GET | `/api/backups/download` | `?name=` | stream `application/gzip` + `Content-Disposition: attachment`; `404 {error:"não encontrado"}` | leitura em stream | pendente |
| POST | `/api/backups/restore` | `{name}` | `{ok:true}`; `404`; `409 {error:"pare o servidor antes de restaurar…"}`; `502 {ok:false, output}` | `tar -xzf` sobre a pasta da instância (180 s); audit `backup-restaurar` | pendente |

### 6.3 Conteúdo (mods/plugins via Modrinth) (Auth)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| * | `/api/content/info` | — | `{loader, kind:"mods"\|"plugins", mcVersion, onlineMode, sorts}` | lê meta, `.craftbox-loader`, `logs/latest.log` | pendente |
| * | `/api/content/search` | `?q&sort&category&offset` | `{results:[{slug,projectId,title,author,description,downloads,follows,icon,type,categories}], total, offset, limit:24}`; `502` | HTTPS api.modrinth.com | pendente |
| * | `/api/content/project` | `?slug` | `{project:{...}}`; `400 {error:"slug vazio"}`; `502` | HTTPS | pendente |
| * | `/api/content/versions` | `?slug` | `{mcVersion, versions:[...]}`; `400`; `502` | HTTPS | pendente |
| GET | `/api/content/installed` | — | `{loader, folder, items:[{slug,title,icon,version,versionId,file,filename,disabled,sizeMB,managed}]}` | `mkdir` de mods/plugins; lê o manifesto | pendente |
| POST | `/api/content/install` | `{slug, versionId?}` | `{ok, installed:[{slug,title,filename,version,dep}]}`; `400`; `502` | baixa `.jar` e dependências obrigatórias (até profundidade 3); troca a versão antiga; manifesto; audit `mod-install` | pendente |
| POST | `/api/content/toggle` | `{file, enabled}` | `{ok}`; `400 {error:"arquivo invalido"}`; `500` | renomeia `.jar` ↔ `.jar.disabled`; manifesto; audit `mod-toggle` | pendente |
| * | `/api/content/updates` | — | `{updates:[{slug,title,current,latest,versionId}]}`; `502` | HTTPS, 1 chamada por projeto | pendente |
| DELETE | `/api/content/installed` | `?slug` ou `?file` | `{ok}`; `400`; `500` | apaga o arquivo e a entrada do manifesto; audit `mod-remove` (registrado **antes** de apagar) | pendente |
| POST | `/api/compat/offline` | `{enabled}` | `{ok, onlineMode}` | `online-mode` no server.properties; audit `modo-offline` | pendente |
| POST | `/api/compat/bedrock` | — | `{ok, loader, folder, installed}`; `502` | baixa Geyser/Floodgate (download.geysermc.org, Modrinth); audit `bedrock` | pendente |
| POST | `/api/compat/auth` | — | `{ok, loader, offline:true, installed}`; `502` | baixa AuthMe (API do GitHub) ou EasyAuth; `online-mode=false`; audit `login-jogadores` | pendente |

### 6.4 Multi-servidor e modpacks (Auth)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| GET | `/api/servers` | — | `{multi, activeId, servers:[{id,name,loader,mcVersion,port,active,selected,modpack}]}` | 1 `is-active` por instância | pendente |
| * | `/api/servers/:id/(start\|stop\|restart\|status)` | — | status: `{id,name,active,uptime,players}`; ação: `200/500 {ok, output}`; `404 {error:"servidor não existe"}` | power da instância; audit `ligar`/`desligar`/`reiniciar`; túneis no start | pendente |
| POST | `/api/servers/select` | `{id}` | `{ok, activeId}`; `400 {error:"multi-servidor desativado"}`; `404` | grava `activeServer` no config; audit | pendente |
| POST | `/api/servers/create` | `{name, loader: paper\|fabric\|pumpkin, version?}` | `{ok, server:{id,name,loader,mcVersion,port}}`; `400 {error:"dê um nome ao servidor"}`; `502` | baixa o servidor (fill.papermc.io v3, meta.fabricmc.net, release do Pumpkin no GitHub); grava eula, props, `start.sh`, `backup.sh` e meta; porta livre a partir de 25565 (RCON = porta+10); pode gravar `activeServer`; audit | pendente |
| POST | `/api/servers/clone` | `{id, name?}` | `{ok, server}`; `502` | `cp -r` da instância, novas portas; audit | pendente |
| DELETE | `/api/servers` | `?id=` | `{ok:true}`; `500 {error}` | para a unidade e **`rm -rf`** da pasta; pode gravar `activeServer`; audit | pendente (**ver bug B1**) |
| * | `/api/modpacks/search` | `?q&loader&offset` | `{results, total, offset, limit}`; `502` | HTTPS Modrinth | pendente |
| * | `/api/modpacks/versions` | `?slug` | `{versions:[{id,name,versionNumber,gameVersions,loaders,datePublished,versionType,url}]}`; `400`; `502` | HTTPS | pendente |
| GET | `/api/servers/create-progress` | — | `{name, phase, done, total, startedAt, at}` ou `{phase:null}` | estado global em memória (1 instalação por vez) | pendente |
| POST | `/api/servers/create-modpack` | `{name?, slug, versionId?}` | `{ok, server:{id,name,loader,mcVersion,port,modpack,strippedMods}}`; `400 {error:"modpack não informado"}`; `502` | **leva minutos**: baixa `.mrpack` para `os.tmpdir()`, `unzip` + `chmod`, downloads em pool de 6, overrides, desativa mods só de cliente (`unzip -p` do `fabric.mod.json`), instala o loader (Fabric por download; Quilt/Forge/NeoForge rodando o instalador `java`, até 15 min); audit | pendente |

### 6.5 Integrações e rede (Auth)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| GET | `/api/integrations` | — | `{dir, systemctlUser, server:{id,name,port}, playit:{installed,hasSecret,running,address,onStart}, cloudflare:{installed,running,hasToken,hostname,onStart}, tailscale:{installed,running,ip,state}}` | `tailscale version/status --json`, `is-active` e journal das unidades | pendente |
| POST | `/api/integrations/install` | `{name: playit\|cloudflare\|tailscale}` | `{ok, note?}`; `502` | baixa o binário (releases do GitHub) com `chmod 755`; audit | pendente |
| POST | `/api/integrations/start` | `{name}` | `{ok}` ou `{ok, loginUrl}` (tailscale); `500` | grava a unidade `~/.config/systemd/user/*.service` (o token do Cloudflare vai **no ExecStart**), `daemon-reload`, `start`; tailscale: `tailscale up`, com fallback para `sudo -n`; audit | pendente |
| POST | `/api/integrations/stop` | `{name}` | `{ok}`; `500` | `stop` da unidade / `tailscale down`; audit | pendente |
| GET | `/api/integrations/log` | `?name` | `{log}`; `400 {error:"nome inválido"}` | journal/log do runner | pendente (não usada pelo front) |
| POST | `/api/integrations/cloudflare-token` | `{token}` | `{ok}`; `400 {error:"token vazio"}` | arquivo 0600 | pendente |
| POST | `/api/integrations/cloudflare-conf` | `{onStart?, hostname?}` | `{ok}` | meta `cfOnStart`, arquivo do host | pendente (não usada pelo front) |
| POST | `/api/integrations/playit-conf` | `{onStart}` | `{ok, onStart}` | meta `playitOnStart` | pendente |
| POST | `/api/integrations/playit-secret` | `{secret}` (hex ≥ 16) | `{ok}`; `400` | `playit.toml` 0600 | pendente |
| GET | `/api/net` | — | `{hasWifi, ip, ssid, connectivity}` | `nmcli` ×3 + `ip` | pendente |
| GET | `/api/net/scan` | — | `{networks:[{ssid,signal,security}]}`; `502` | `nmcli ... --rescan yes` (30 s) | pendente |
| POST | `/api/net/wifi` | `{ssid, password?}` | `{ok}`; `502` | `sudo -n nmcli dev wifi connect` (45 s); audit | pendente |

### 6.6 Extras, conta e auditoria (Auth)

| método | rota | entrada | saída | efeitos | status |
|---|---|---|---|---|---|
| GET | `/api/extras` | — | `{show}` | — | pendente (não usada pelo front) |
| POST | `/api/extras` | `{show}` | `{show}` | grava `showExtras` no config; audit | pendente (não usada pelo front) |
| GET | `/api/worldsize` | — | `{bytes, dirs:[...]}` | percorre as pastas de mundo | pendente |
| GET/POST | `/api/speedtest` | — | `{mbps, bytes, secs}`; `502` | download real de 25 MB (speed.cloudflare.com, 30 s) | pendente |
| POST | `/api/change-password` | `{current, newPassword}` | `{ok}`; `400` (menos de 4 caracteres / sem senha); `401` | novo salt+hash; grava o config; audit | pendente |
| GET | `/api/users` | — | `{multiUser, me, isAdmin, users:[{user,role}]}` | — | pendente |
| POST | `/api/users` | `{user, password, role}` | `{ok, firstUser}`; `403`, `400`, `409` | grava o config; o 1º usuário vira admin e **recebe Set-Cookie** (a sessão migra pra ele); audit | pendente |
| DELETE | `/api/users` | `?user=` | `{ok}`; `403`, `404`, `400 {error:"não dá pra remover o último admin"}` | grava o config; audit | pendente |
| GET | `/api/audit` | `?lines=` (padrão 300, máx 2000) | `{items:[...]}` (mais novo primeiro) | lê o log | pendente |
| DELETE | `/api/audit` | — | `{ok}` | trunca o log e registra `historico-limpo` | pendente |
| * | qualquer outra `/api/*` | — | `404 {error:"endpoint desconhecido"}` | — | **FEITA** |

### 6.7 Auditoria (efeito colateral transversal)

Uma linha JSON por ação: `{"ts":<ms>,"ip":"<remoto sem ::ffff:>","user":"<sessão ou ->","server":"<id>","action":"...","detail":"..."}`. É acrescentada ao arquivo; acima de 1 MiB, ficam as últimas 500 linhas. No Rust está **FEITA** (módulo `audit`), e o teste compara as entradas de login nos dois backends.

---

## 7. Bugs e esquisitices do server.js encontrados no inventário

O Rust **reproduz** o que é contrato e **não reproduz** o que derruba o processo. O que for correção de segurança deve entrar nos dois backends (ou no Rust e ser documentado como mudança de contrato).

| # | gravidade | onde | o quê | no Rust |
|---|---|---|---|---|
| B1 | **crítica** | `DELETE /api/servers?id=` | O `id` não é validado contra a lista de instâncias: **sem `id`** (`''`), `path.join(serversDir,'')` = `serversDir` e o `rm -rf` apaga **todos os servidores**; com `id=..`, apaga a pasta-mãe (`/srv/minecraft`, `/data`…). Qualquer usuário logado consegue (não exige admin). **Confirmado** num diretório temporário: `DELETE /api/servers` sem `id` respondeu `{"ok":true}` e a pasta `servers/` sumiu. O mesmo padrão vale para `clone` (`id=..` copia a pasta-mãe). | validar `id ∈ listInstanceIds()` ao migrar (fatia 3); vale corrigir já no Node |
| B2 | alta (DoS) | leitura de cookie | `decodeURIComponent` de um cookie malformado (`cbsession=%E0`, ou **qualquer** cookie com `%` inválido, até de outro app no mesmo host) lança `URIError` **fora do try**, e o processo Node morre. Demonstrado no teste de paridade. | o cookie malformado é ignorado (→ 401) |
| B3 | média | sessão em multiusuário | O valor do cookie é decodificado **antes** de verificar o MAC, mas o MAC foi calculado sobre o valor codificado. Usuários com `@`, `+` ou acentos (o regex de criação aceita `@` e `+`) fazem login, recebem o cookie e **nunca ficam autenticados**. | reproduzido (senão os cookies deixariam de ser intercambiáveis); corrigir nos dois, verificando o MAC sobre o valor cru |
| B4 | baixa | RCON | Se o servidor fechar a conexão sem responder, a Promise nunca resolve e `/api/status` fica pendurado. | vira erro, `players: null` |
| B5 | baixa | `/api/servers/:id/*` | A auditoria dessas ações sai com `ip:"-"` e `user:"-"` (o contexto novo do `als.run` não copia ip/user). | decidir na fatia 3 (provavelmente corrigir) |
| B6 | baixa | uptime (systemd) | Fusos cuja abreviação o V8 não conhece (`CEST`, `BRT`…) dão `uptime: null`. | reproduzido; dá para corrigir usando `ActiveEnterTimestampMonotonic` |
| B7 | info | `/api/logs?lines=abc` | `Math.min(1000, NaN)` = NaN, e `slice(-NaN)` devolve o **arquivo inteiro**. | decidir na fatia 1 |
| B8 | info | `DELETE /api/audit` | Qualquer usuário logado (não só admin) apaga o histórico. | decidir na fatia 6 |
| B9 | info | `new URL(req.url)` | Um `req.url` que o parser WHATWG rejeite também lança fora do try. | o Rust normaliza sem lançar |

---

## 8. Fatias da migração (ordem sugerida)

A ordem prioriza (1) o que o dashboard usa a cada 3 s, (2) risco baixo antes de alto e (3) reaproveitar a infraestrutura de cada fatia na seguinte.

| fase | fatia | rotas | infraestrutura nova | por quê nesta ordem |
|---|---|---|---|---|
| **1 (feita)** | fundação | login, logout, authcheck, estáticos, `/api/status`, `/api/diag/mojang`, 404/401, shutdown gracioso, CLI | HTTP, JSON com semântica JS, config, sessão, auditoria, RCON, `systemctl` só leitura, HTTPS de saída | base de tudo |
| 2 | **energia e console** | `/api/servers/:id/*`, `/api/power`, `/api/servers` (GET), `/api/servers/select`, `/api/logs`, `/api/rcon`, `/api/properties` (GET/PUT), `/api/compat/offline`, autostart | `unitStart`/`spawnUnit`, `svcAction` com sudo/`--user`, `writeProps`, gravação do config | é o que o painel mais usa depois do status; fecha o ciclo do runner exec (container) |
| 3 | servidores | `/api/servers/create`, `clone`, `DELETE /api/servers` (com a **correção B1**), `/api/worldsize`, backups (listar, criar, baixar em stream, restaurar) | downloads com redirect e timeout, gravação de `start.sh`/props/meta, stream de arquivo na resposta | reusa o HTTPS e o `run()` da fase 1 |
| 4 | conteúdo | `/api/content/*`, `/api/compat/bedrock`, `/api/compat/auth` | cliente Modrinth, manifesto, resolução de dependências | só HTTPS + arquivos, baixo risco |
| 5 | modpacks | `/api/modpacks/*`, `create-modpack`, `create-progress` | `.mrpack` (zip; avaliar o crate `zip` × manter `unzip` externo), pool de downloads, instaladores `java`, progresso compartilhado | a mais longa e arriscada; fica depois que o resto estiver estável |
| 6 | contas e histórico | `/api/change-password`, `/api/users` (GET/POST/DELETE), `/api/audit` (GET/DELETE), `/api/extras` | escrita concorrente do config (`RwLock` + gravação atômica) | pequena, mas mexe em credenciais |
| 7 | integrações e rede | `/api/integrations/*`, `/api/net*`, `/api/speedtest` | unidades de usuário do systemd, `tailscale`, `nmcli`, sudo | depende de muito ambiente externo; testar no appliance real |
| 8 | corte | trocar o `craftbox-panel.service` (e o Dockerfile) para o binário; compilar na instalação com `RUSTFLAGS="-C target-cpu=native"` e usar o pré-compilado como fallback | script de build na ISO/primeiro boot | só quando o teste de paridade cobrir todas as rotas |

Durante a transição dá para rodar os dois backends com o **mesmo config.json**: o cookie vale nos dois. A ressalva é que cada processo mantém o config em memória e grava o objeto inteiro, então **não** convém os dois gravarem ao mesmo tempo. Um proxy "strangler" (Rust na frente, repassando as rotas pendentes ao Node) só é seguro depois da fase 6, ou com o Rust relendo o config quando o mtime mudar.
