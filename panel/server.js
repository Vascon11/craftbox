#!/usr/bin/env node
'use strict';
/*
 * craftbox-panel — painel web para configurar e gerenciar o servidor Minecraft.
 * Node.js puro, SEM dependencias (so modulos nativos). Leve o bastante pra rodar
 * ao lado do servidor num notebook antigo.
 *
 * Uso:
 *   node server.js                 # sobe o painel
 *   node server.js --hash SENHA    # gera o hash da senha p/ o config.json
 *   node server.js --init          # cria um config.json inicial
 *
 * Config: config.json (ou env CRAFTBOX_PANEL_CONFIG).
 */
const http = require('http');
const https = require('https');
const fs = require('fs');
const os = require('os');
const path = require('path');
const net = require('net');
const crypto = require('crypto');
const { execFile } = require('child_process');
const { AsyncLocalStorage } = require('async_hooks');

const ROOT = __dirname;
const PUBLIC = path.join(ROOT, 'public');
const CONFIG_PATH = process.env.CRAFTBOX_PANEL_CONFIG || path.join(ROOT, 'config.json');

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------
const DEFAULT_CONFIG = {
  port: 8080,
  host: '0.0.0.0',
  mcDir: '/opt/minecraft',
  service: 'minecraft',
  rcon: { host: '127.0.0.1', port: 25575, password: '' }, // password vazio = ler do server.properties
  auth: { salt: '', hash: '' },                            // gere com: node server.js --hash SENHA
  sessionSecret: '',                                       // gerado automaticamente se vazio
  loader: '',                                              // 'paper'|'fabric' (auto-detecta se vazio)
  mcVersion: '',                                           // ex '1.21.1' (auto-detecta do log se vazio)
  serversDir: '',                                          // se definido, ativa MULTI-SERVIDOR (cada subpasta = 1 instancia)
  serviceTemplate: 'minecraft@',                           // unit systemd template; servico = serviceTemplate + <id>
  activeServer: '',                                        // instancia selecionada no painel (multi)
  systemctlUser: false,                                    // true = usa 'systemctl --user' (rootless, sem sudo)
  integrationsDir: ''                                      // onde ficam os binarios das integracoes (playit/cloudflared); vazio = auto
};

function loadConfig() {
  let cfg = { ...DEFAULT_CONFIG };
  try {
    const raw = JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8'));
    cfg = { ...cfg, ...raw, rcon: { ...cfg.rcon, ...(raw.rcon || {}) }, auth: { ...cfg.auth, ...(raw.auth || {}) } };
  } catch { /* usa defaults */ }
  if (!cfg.sessionSecret) {
    cfg.sessionSecret = crypto.randomBytes(32).toString('hex');
    try { fs.writeFileSync(CONFIG_PATH, JSON.stringify(cfg, null, 2)); } catch {}
  }
  return cfg;
}

function hashPassword(password, salt) {
  const s = salt || crypto.randomBytes(16).toString('hex');
  const h = crypto.scryptSync(password, s, 64).toString('hex');
  return { salt: s, hash: h };
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------
if (process.argv[2] === '--hash') {
  const pw = process.argv[3];
  if (!pw) { console.error('uso: node server.js --hash SENHA'); process.exit(1); }
  const { salt, hash } = hashPassword(pw);
  console.log('Cole isto no "auth" do config.json:');
  console.log(JSON.stringify({ salt, hash }, null, 2));
  process.exit(0);
}
if (process.argv[2] === '--init') {
  if (fs.existsSync(CONFIG_PATH)) { console.error('config.json ja existe.'); process.exit(1); }
  const cfg = { ...DEFAULT_CONFIG, sessionSecret: crypto.randomBytes(32).toString('hex') };
  fs.writeFileSync(CONFIG_PATH, JSON.stringify(cfg, null, 2));
  console.log('config.json criado em', CONFIG_PATH, '\nDefina a senha com: node server.js --hash SUA_SENHA');
  process.exit(0);
}

const CONFIG = loadConfig();
function saveConfig() { try { fs.writeFileSync(CONFIG_PATH, JSON.stringify(CONFIG, null, 2)); } catch {} }

// ---------------------------------------------------------------------------
// Multi-servidor: contexto por requisicao (AsyncLocalStorage, sem dependencia)
// Se CONFIG.serversDir estiver definido, cada subpasta e uma instancia.
// Senao, roda em modo legado (1 servidor: CONFIG.mcDir + CONFIG.service).
// ---------------------------------------------------------------------------
const als = new AsyncLocalStorage();
function multiEnabled() { return !!CONFIG.serversDir; }
function slugifyId(name) {
  return String(name || '').toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 32) || 'server';
}
function instanceMetaPath(dir) { return path.join(dir, '.craftbox-instance.json'); }
function readInstanceMeta(dir) { try { return JSON.parse(fs.readFileSync(instanceMetaPath(dir), 'utf8')); } catch { return {}; } }
function writeInstanceMeta(dir, meta) { try { fs.mkdirSync(dir, { recursive: true }); fs.writeFileSync(instanceMetaPath(dir), JSON.stringify(meta, null, 2)); } catch {} }
function listInstanceIds() {
  if (!multiEnabled()) return ['default'];
  try { return fs.readdirSync(CONFIG.serversDir, { withFileTypes: true }).filter(d => d.isDirectory()).map(d => d.name).sort(); }
  catch { return []; }
}
function serverCtx(id) {
  if (!multiEnabled()) return { id: 'default', dir: CONFIG.mcDir, service: CONFIG.service, name: 'Servidor', legacy: true };
  const dir = path.join(CONFIG.serversDir, id);
  const meta = readInstanceMeta(dir);
  return { id, dir, service: CONFIG.serviceTemplate + id, name: meta.name || id, loader: meta.loader || '', mcVersion: meta.mcVersion || '', port: meta.port, modpack: meta.modpack || null };
}
function currentId(url) {
  if (!multiEnabled()) return 'default';
  const q = url && url.searchParams.get('server');
  const ids = listInstanceIds();
  if (q && ids.includes(q)) return q;
  if (CONFIG.activeServer && ids.includes(CONFIG.activeServer)) return CONFIG.activeServer;
  return ids[0] || 'default';
}
function SRV() { return als.getStore() || serverCtx(currentId(null)); }

// ---------------------------------------------------------------------------
// Sessao (cookie assinado, sem dependencia)
// ---------------------------------------------------------------------------
function sign(value) {
  const mac = crypto.createHmac('sha256', CONFIG.sessionSecret).update(value).digest('base64url');
  return `${value}.${mac}`;
}
function verify(token) {
  if (!token || !token.includes('.')) return null;
  const i = token.lastIndexOf('.');
  const value = token.slice(0, i), mac = token.slice(i + 1);
  const expect = crypto.createHmac('sha256', CONFIG.sessionSecret).update(value).digest('base64url');
  if (mac.length !== expect.length || !crypto.timingSafeEqual(Buffer.from(mac), Buffer.from(expect))) return null;
  const exp = parseInt(value.split(':')[1], 10);
  if (!exp || Date.now() > exp) return null;
  return value;
}
function makeSession() { return sign(`s:${Date.now() + 12 * 3600 * 1000}`); } // 12h
function parseCookies(req) {
  const out = {};
  (req.headers.cookie || '').split(';').forEach(c => {
    const i = c.indexOf('='); if (i < 0) return;
    out[c.slice(0, i).trim()] = decodeURIComponent(c.slice(i + 1).trim());
  });
  return out;
}
function isAuthed(req) { return !!verify(parseCookies(req).cbsession); }

// ---------------------------------------------------------------------------
// systemctl
// ---------------------------------------------------------------------------
function run(cmd, args, opts = {}) {
  return new Promise((resolve) => {
    execFile(cmd, args, { timeout: 15000, ...opts }, (err, stdout, stderr) => {
      resolve({ code: err ? (err.code || 1) : 0, stdout: (stdout || '').toString(), stderr: (stderr || '').toString() });
    });
  });
}
// argumentos do systemctl (prefixa --user no modo rootless)
function scArgs(rest) { return CONFIG.systemctlUser ? ['--user', ...rest] : rest; }
async function svcActiveOf(service) {
  const r = await run('systemctl', scArgs(['is-active', service]));
  return r.stdout.trim() || 'unknown';
}
async function svcActive() { return svcActiveOf(SRV().service); }
async function svcAction(action, service) {
  if (!['start', 'stop', 'restart'].includes(action)) throw new Error('acao invalida');
  const svc = service || SRV().service;
  // rootless: systemctl --user <action> <svc>  |  sistema: sudoers permitindo systemctl <action> <svc>
  const r = CONFIG.systemctlUser
    ? await run('systemctl', ['--user', action, svc])
    : await run('sudo', ['-n', 'systemctl', action, svc]);
  return r;
}
async function svcUptime() {
  const r = await run('systemctl', scArgs(['show', SRV().service, '--property=ActiveEnterTimestamp', '--value']));
  const t = Date.parse(r.stdout.trim());
  return isNaN(t) ? null : Math.max(0, Math.floor((Date.now() - t) / 1000));
}

// ---------------------------------------------------------------------------
// server.properties
// ---------------------------------------------------------------------------
function propsPath() { return path.join(SRV().dir, 'server.properties'); }
function readProps() {
  const out = {};
  try {
    fs.readFileSync(propsPath(), 'utf8').split('\n').forEach(line => {
      const t = line.trim();
      if (!t || t.startsWith('#')) return;
      const i = t.indexOf('='); if (i < 0) return;
      out[t.slice(0, i)] = t.slice(i + 1);
    });
  } catch {}
  return out;
}
function writeProps(updates) {
  const p = propsPath();
  let lines = [];
  try { lines = fs.readFileSync(p, 'utf8').split('\n'); } catch { lines = []; }
  const seen = new Set();
  lines = lines.map(line => {
    const t = line.trim();
    if (!t || t.startsWith('#')) return line;
    const i = t.indexOf('='); if (i < 0) return line;
    const key = t.slice(0, i);
    if (Object.prototype.hasOwnProperty.call(updates, key)) {
      seen.add(key);
      return `${key}=${updates[key]}`;
    }
    return line;
  });
  for (const [k, v] of Object.entries(updates)) if (!seen.has(k)) lines.push(`${k}=${v}`);
  fs.writeFileSync(p, lines.join('\n'));
}

// ---------------------------------------------------------------------------
// RCON (protocolo Source, sobre TCP, sem dependencia)
// ---------------------------------------------------------------------------
function rconPassword() {
  const fromProps = readProps()['rcon.password'];
  if (fromProps) return fromProps;
  return CONFIG.rcon.password || '';
}
function rconPort() {
  const fromProps = parseInt(readProps()['rcon.port'], 10);
  return fromProps || CONFIG.rcon.port || 25575;
}
function rcon(command) {
  return new Promise((resolve, reject) => {
    const pw = rconPassword();
    if (!pw) return reject(new Error('RCON sem senha (habilite enable-rcon no server.properties)'));
    const sock = net.connect({ host: CONFIG.rcon.host, port: rconPort() });
    let buf = Buffer.alloc(0);
    let authed = false;
    const AUTH = 3, EXEC = 2;
    const pkt = (id, type, body) => {
      const b = Buffer.from(body + '\0\0', 'utf8');
      const len = 4 + 4 + b.length;
      const out = Buffer.alloc(4 + len);
      out.writeInt32LE(len, 0); out.writeInt32LE(id, 4); out.writeInt32LE(type, 8);
      b.copy(out, 12);
      return out;
    };
    sock.setTimeout(6000);
    sock.on('timeout', () => { sock.destroy(); reject(new Error('RCON timeout')); });
    sock.on('error', reject);
    sock.on('connect', () => sock.write(pkt(1, AUTH, pw)));
    sock.on('data', (d) => {
      buf = Buffer.concat([buf, d]);
      while (buf.length >= 4) {
        const len = buf.readInt32LE(0);
        if (buf.length < 4 + len) break;
        const id = buf.readInt32LE(4);
        const body = buf.slice(12, 4 + len - 2).toString('utf8');
        buf = buf.slice(4 + len);
        if (!authed) {
          if (id === -1) { sock.destroy(); return reject(new Error('senha RCON incorreta')); }
          authed = true;
          sock.write(pkt(2, EXEC, command));
        } else {
          sock.destroy();
          return resolve(body);
        }
      }
    });
  });
}

// ---------------------------------------------------------------------------
// Estatisticas do sistema
// ---------------------------------------------------------------------------
function readNum(p) { try { return parseInt(fs.readFileSync(p, 'utf8').trim(), 10); } catch { return null; } }
function cpuFreq() {
  const cur = readNum('/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq');
  const max = readNum('/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq');
  return { curMHz: cur ? Math.round(cur / 1000) : null, maxMHz: max ? Math.round(max / 1000) : null };
}
function cpuTemp() {
  const zones = ['/sys/class/thermal/thermal_zone0/temp', '/sys/class/thermal/thermal_zone1/temp'];
  for (const z of zones) { const v = readNum(z); if (v && v > 1000) return Math.round(v / 1000); }
  return null;
}
async function systemStats() {
  const mem = { total: Math.round(os.totalmem() / 1048576), free: Math.round(os.freemem() / 1048576) };
  mem.used = mem.total - mem.free;
  let disk = null;
  try {
    const s = fs.statfsSync(SRV().dir);
    const totalGB = (s.blocks * s.bsize) / 1073741824;
    const freeGB = (s.bfree * s.bsize) / 1073741824;
    disk = { totalGB: +totalGB.toFixed(1), usedGB: +(totalGB - freeGB).toFixed(1) };
  } catch {}
  return {
    load: os.loadavg().map(x => +x.toFixed(2)),
    cpus: os.cpus().length,
    cpu: cpuFreq(),
    tempC: cpuTemp(),
    mem, disk,
    uptimeHost: Math.floor(os.uptime())
  };
}

// ---------------------------------------------------------------------------
// Conteudo: loader, versao, Modrinth, GeyserMC, downloads
// ---------------------------------------------------------------------------
const UA = 'craftbox-panel/1.0 (github.com/Vascon11/craftbox)';
function detectLoader() {
  const s = SRV();
  if (s.loader) return s.loader;
  if (s.legacy && CONFIG.loader) return CONFIG.loader;
  try { const m = fs.readFileSync(path.join(s.dir, '.craftbox-loader'), 'utf8').trim(); if (m) return m; } catch {}
  const hasMods = fs.existsSync(path.join(s.dir, 'mods'));
  const hasPlugins = fs.existsSync(path.join(s.dir, 'plugins'));
  if (hasMods && !hasPlugins) return 'fabric';
  if (hasPlugins && !hasMods) return 'paper';
  return hasMods ? 'fabric' : 'paper';
}
function contentFolder(loader) {
  const dir = path.join(SRV().dir, loader === 'fabric' ? 'mods' : 'plugins');
  try { fs.mkdirSync(dir, { recursive: true }); } catch {}
  return dir;
}
function loaderCategories(loader) { return loader === 'fabric' ? ['fabric'] : ['paper', 'spigot', 'bukkit']; }
function detectMcVersion() {
  const s = SRV();
  if (s.mcVersion) return s.mcVersion;
  if (s.legacy && CONFIG.mcVersion) return CONFIG.mcVersion;
  try {
    const log = fs.readFileSync(path.join(s.dir, 'logs', 'latest.log'), 'utf8');
    const m = log.match(/minecraft server version\s+([\w.\-]+)/i) || log.match(/\(MC:\s*([\w.\-]+)\)/);
    if (m) return m[1];
  } catch {}
  return null;
}
function httpsJson(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': UA, Accept: 'application/json' } }, (r) => {
      if (r.statusCode >= 300 && r.statusCode < 400 && r.headers.location) { r.resume(); return resolve(httpsJson(new URL(r.headers.location, url).toString())); }
      if (r.statusCode !== 200) { r.resume(); return reject(new Error('HTTP ' + r.statusCode)); }
      let d = ''; r.on('data', c => d += c); r.on('end', () => { try { resolve(JSON.parse(d)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}
function download(url, dest) {
  return new Promise((resolve, reject) => {
    const f = fs.createWriteStream(dest);
    const go = (u) => {
      const req = https.get(u, { headers: { 'User-Agent': UA } }, (r) => {
        if (r.statusCode >= 300 && r.statusCode < 400 && r.headers.location) { r.resume(); return go(new URL(r.headers.location, u).toString()); }
        if (r.statusCode !== 200) { r.resume(); return reject(new Error('HTTP ' + r.statusCode)); }
        r.pipe(f); f.on('finish', () => f.close(() => resolve()));
      });
      req.setTimeout(60000, () => req.destroy(new Error('download timeout')));
      req.on('error', (e) => { try { fs.unlinkSync(dest); } catch {} reject(e); });
    };
    go(url);
  });
}
// loaders aceitos ao listar versoes de um projeto (mais amplo que a busca)
function versionLoaders(loader) {
  return loader === 'fabric' ? ['fabric', 'quilt'] : ['paper', 'purpur', 'spigot', 'bukkit', 'folia'];
}
// nomes de "categorias" que na verdade sao loaders/ambiente — escondemos na UI
const LOADER_TAGS = new Set(['fabric', 'quilt', 'forge', 'neoforge', 'paper', 'purpur', 'spigot', 'bukkit', 'folia', 'velocity', 'sponge', 'bungeecord', 'waterfall', 'datapack', 'client', 'server']);

const SORTS = ['relevance', 'downloads', 'follows', 'newest', 'updated'];
async function modrinthSearch(query, loader, version, opts = {}) {
  const cats = loaderCategories(loader).map(c => `"categories:${c}"`).join(',');
  const facets = [`[${cats}]`];
  if (version) facets.push(`["versions:${version}"]`);
  if (opts.category) facets.push(`["categories:${opts.category}"]`);
  const index = SORTS.includes(opts.sort) ? opts.sort : 'relevance';
  const url = `https://api.modrinth.com/v2/search?limit=40&index=${index}&query=${encodeURIComponent(query || '')}&facets=${encodeURIComponent('[' + facets.join(',') + ']')}`;
  const data = await httpsJson(url);
  return (data.hits || []).map(h => ({
    slug: h.slug, projectId: h.project_id, title: h.title, author: h.author,
    description: h.description, downloads: h.downloads, follows: h.follows,
    icon: h.icon_url, type: h.project_type,
    categories: (h.display_categories || h.categories || []).filter(c => !LOADER_TAGS.has(c)),
  }));
}
// lista todas as versoes compativeis com o loader; marca .compatible pra versao do MC
async function modrinthVersions(slug, loader, mcVersion) {
  const ls = versionLoaders(loader);
  const u = `https://api.modrinth.com/v2/project/${encodeURIComponent(slug)}/version?loaders=${encodeURIComponent(JSON.stringify(ls))}`;
  const vers = await httpsJson(u).catch(() => []);
  return (vers || []).map(v => {
    const file = v.files.find(f => f.primary) || v.files[0];
    return {
      id: v.id, name: v.name, versionNumber: v.version_number,
      gameVersions: v.game_versions || [], versionType: v.version_type,
      datePublished: v.date_published, downloads: v.downloads,
      filename: file ? path.basename(file.filename) : null,
      url: file ? file.url : null,
      compatible: mcVersion ? (v.game_versions || []).includes(mcVersion) : true,
      dependencies: v.dependencies || [],
    };
  });
}
async function modrinthProject(slugOrId) {
  const p = await httpsJson(`https://api.modrinth.com/v2/project/${encodeURIComponent(slugOrId)}`);
  return {
    slug: p.slug, projectId: p.id, title: p.title, description: p.description, body: p.body,
    icon: p.icon_url, categories: (p.categories || []).filter(c => !LOADER_TAGS.has(c)),
    downloads: p.downloads, follows: p.followers, gallery: (p.gallery || []).map(g => g.url),
    source: p.source_url, projectType: p.project_type,
    clientSide: p.client_side, serverSide: p.server_side,
  };
}
// escolhe a "melhor" versao: id explicito > compativel+release > compativel > qualquer release > qualquer
function pickVersion(versions, versionId, mcVersion) {
  if (versionId) { const v = versions.find(x => x.id === versionId); if (v) return v; }
  const compat = versions.filter(v => v.compatible && v.url);
  const pool = (compat.length ? compat : versions.filter(v => v.url));
  return pool.find(v => v.versionType === 'release') || pool[0] || null;
}
// wrapper legado (usado pelo Bedrock) — devolve {filename,url,version}
async function modrinthResolve(slug, loader, version) {
  const vers = await modrinthVersions(slug, loader, version);
  const v = pickVersion(vers, null, version);
  if (!v || !v.url) throw new Error('sem versao compativel pra esse loader');
  return { filename: v.filename, url: v.url, version: v.versionNumber };
}

// ---- manifesto: sabe QUAIS projetos estao instalados (nao so arquivos) ----
function manifestPath() { return path.join(SRV().dir, '.craftbox-content.json'); }
function readManifest() { try { return JSON.parse(fs.readFileSync(manifestPath(), 'utf8')); } catch { return {}; } }
function writeManifest(m) { try { fs.writeFileSync(manifestPath(), JSON.stringify(m, null, 2)); } catch {} }

// instala um projeto (versao especifica ou melhor) + dependencias obrigatorias
async function installProject(slug, loader, mcVersion, versionId, visited, depth) {
  visited = visited || new Set(); depth = depth || 0;
  if (visited.has(slug)) return [];
  visited.add(slug);
  const versions = await modrinthVersions(slug, loader, mcVersion);
  if (!versions.length) throw new Error(`sem versão compatível de "${slug}" pra ${loader}`);
  const v = pickVersion(versions, versionId, mcVersion);
  if (!v || !v.url) throw new Error(`sem arquivo instalável pra "${slug}"`);
  const folder = contentFolder(loader);
  const man = readManifest();
  // remove versao antiga do mesmo projeto (ativa ou desativada)
  if (man[slug] && man[slug].filename && man[slug].filename !== v.filename) {
    try { fs.unlinkSync(path.join(folder, man[slug].filename)); } catch {}
    try { fs.unlinkSync(path.join(folder, man[slug].filename + '.disabled')); } catch {}
  }
  await download(v.url, path.join(folder, v.filename));
  let proj = null; try { proj = await modrinthProject(slug); } catch {}
  man[slug] = {
    projectId: proj ? proj.projectId : (man[slug] && man[slug].projectId) || null,
    title: proj ? proj.title : ((man[slug] && man[slug].title) || slug),
    icon: proj ? proj.icon : (man[slug] && man[slug].icon) || null,
    filename: v.filename, versionId: v.id, versionNumber: v.versionNumber,
    type: loader === 'fabric' ? 'mod' : 'plugin', disabled: false, installedAt: Date.now(),
  };
  writeManifest(man);
  const installed = [{ slug, title: man[slug].title, filename: v.filename, version: v.versionNumber, dep: depth > 0 }];
  if (depth < 3) {
    for (const d of v.dependencies) {
      if (d.dependency_type !== 'required' || !d.project_id) continue;
      try {
        const dp = await modrinthProject(d.project_id).catch(() => null);
        const depSlug = dp ? dp.slug : d.project_id;
        if (visited.has(depSlug)) continue;
        installed.push(...await installProject(depSlug, loader, mcVersion, d.version_id || null, visited, depth + 1));
      } catch {}
    }
  }
  return installed;
}

// lista instalados: cruza o manifesto com os arquivos reais na pasta
function listInstalled() {
  const loader = detectLoader();
  const folder = contentFolder(loader);
  const man = readManifest();
  let files = [];
  try { files = fs.readdirSync(folder); } catch {}
  const byBase = {};
  files.filter(f => f.endsWith('.jar') || f.endsWith('.jar.disabled')).forEach(f => {
    const disabled = f.endsWith('.disabled');
    const base = disabled ? f.slice(0, -'.disabled'.length) : f;
    let sizeMB = 0; try { sizeMB = +(fs.statSync(path.join(folder, f)).size / 1048576).toFixed(2); } catch {}
    byBase[base] = { file: f, disabled, sizeMB };
  });
  const items = [], used = new Set();
  for (const slug of Object.keys(man)) {
    const m = man[slug]; const b = byBase[m.filename];
    if (!b) continue;
    used.add(m.filename);
    items.push({ slug, title: m.title, icon: m.icon, version: m.versionNumber, versionId: m.versionId,
      file: b.file, filename: m.filename, disabled: b.disabled, sizeMB: b.sizeMB, managed: true });
  }
  for (const base of Object.keys(byBase)) {
    if (used.has(base)) continue;
    const b = byBase[base];
    items.push({ slug: null, title: base.replace(/\.jar$/, ''), icon: null, version: null, versionId: null,
      file: b.file, filename: base, disabled: b.disabled, sizeMB: b.sizeMB, managed: false });
  }
  items.sort((a, b) => a.title.localeCompare(b.title));
  return { loader, folder, items };
}

// verifica atualizacoes dos projetos gerenciados
async function checkUpdates() {
  const loader = detectLoader(); const mcVersion = detectMcVersion();
  const man = readManifest(); const updates = [];
  for (const slug of Object.keys(man)) {
    try {
      const vers = await modrinthVersions(slug, loader, mcVersion);
      const latest = pickVersion(vers, null, mcVersion);
      if (latest && latest.id !== man[slug].versionId) {
        updates.push({ slug, title: man[slug].title, current: man[slug].versionNumber, latest: latest.versionNumber, versionId: latest.id });
      }
    } catch {}
  }
  return updates;
}
async function installBedrock() {
  const loader = detectLoader();
  const folder = contentFolder(loader);
  const gm = (proj, plat) => `https://download.geysermc.org/v2/projects/${proj}/versions/latest/builds/latest/downloads/${plat}`;
  const installed = [];
  if (loader === 'paper') {
    await download(gm('geyser', 'spigot'), path.join(folder, 'Geyser-Spigot.jar')); installed.push('Geyser-Spigot.jar');
    await download(gm('floodgate', 'spigot'), path.join(folder, 'floodgate-spigot.jar')); installed.push('floodgate-spigot.jar');
  } else {
    await download(gm('geyser', 'fabric'), path.join(folder, 'Geyser-Fabric.jar')); installed.push('Geyser-Fabric.jar');
    const ver = detectMcVersion();
    const fg = await modrinthResolve('floodgate', 'fabric', ver);
    await download(fg.url, path.join(folder, fg.filename)); installed.push(fg.filename);
    try { const fa = await modrinthResolve('fabric-api', 'fabric', ver); await download(fa.url, path.join(folder, fa.filename)); installed.push(fa.filename); } catch {}
  }
  return { loader, folder, installed };
}

// ---------------------------------------------------------------------------
// Multi-servidor: criar / clonar / apagar instancias
// ---------------------------------------------------------------------------
async function paperResolve(version) {
  let ver = version;
  if (!ver) {
    const proj = await httpsJson('https://fill.papermc.io/v3/projects/paper');
    const all = [];
    Object.values(proj.versions || {}).forEach(arr => (arr || []).forEach(v => all.push(v)));
    ver = all.find(v => !String(v).includes('-')) || all[0];
  }
  const info = await httpsJson(`https://fill.papermc.io/v3/projects/paper/versions/${encodeURIComponent(ver)}/builds/latest`);
  const dl = info.downloads && (info.downloads['server:default'] || info.downloads['server:mojmap']);
  if (!dl || !dl.url) throw new Error('sem build de Paper pra ' + ver);
  return { version: ver, url: dl.url, build: info.id };
}
async function fabricResolveServer(version) {
  let ver = version;
  if (!ver) {
    const games = await httpsJson('https://meta.fabricmc.net/v2/versions/game');
    ver = (games.find(g => g.stable) || games[0]).version;
  }
  const loaders = await httpsJson('https://meta.fabricmc.net/v2/versions/loader');
  const loader = (loaders.find(l => l.stable) || loaders[0]).version;
  const insts = await httpsJson('https://meta.fabricmc.net/v2/versions/installer');
  const inst = (insts.find(i => i.stable) || insts[0]).version;
  const url = `https://meta.fabricmc.net/v2/versions/loader/${encodeURIComponent(ver)}/${encodeURIComponent(loader)}/${encodeURIComponent(inst)}/server/jar`;
  return { version: ver, url };
}
function heapMB() {
  const totalMB = Math.round(os.totalmem() / 1048576);
  return Math.max(512, Math.min(3072, Math.floor(totalMB * 0.4)));
}
function usedPorts() {
  const set = new Set();
  listInstanceIds().forEach(id => { const m = readInstanceMeta(path.join(CONFIG.serversDir, id)); if (m.port) set.add(+m.port); });
  return set;
}
function freePort() { const used = usedPorts(); let p = 25565; while (used.has(p)) p++; return p; }
function updatePropsFile(dir, updates) {
  const p = path.join(dir, 'server.properties');
  let lines = []; try { lines = fs.readFileSync(p, 'utf8').split('\n'); } catch {}
  const seen = new Set();
  lines = lines.map(line => {
    const t = line.trim(); if (!t || t.startsWith('#')) return line;
    const i = t.indexOf('='); if (i < 0) return line;
    const k = t.slice(0, i);
    if (Object.prototype.hasOwnProperty.call(updates, k)) { seen.add(k); return `${k}=${updates[k]}`; }
    return line;
  });
  for (const [k, v] of Object.entries(updates)) if (!seen.has(k)) lines.push(`${k}=${v}`);
  fs.writeFileSync(p, lines.join('\n'));
}
function writeServerProps(dir, o) {
  const props = [
    `motd=${o.motd || 'A craftbox server'}`,
    `server-port=${o.port}`, `query.port=${o.port}`,
    `enable-rcon=true`, `rcon.port=${o.rconPort}`, `rcon.password=${o.rconPass}`,
    `online-mode=true`, `max-players=10`, `view-distance=8`, `simulation-distance=6`,
    `level-name=world`, `spawn-protection=0`, `enable-command-block=true`,
  ];
  fs.writeFileSync(path.join(dir, 'server.properties'), props.join('\n') + '\n');
}
function writeStartScript(dir) {
  const heap = heapMB();
  const jvm = `-Xms512M -Xmx${heap}M -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 ` +
    `-XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:+AlwaysPreTouch -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true`;
  const sh = `#!/usr/bin/env bash\ncd "$(dirname "$0")"\nexec /usr/bin/java ${jvm} -jar server.jar nogui\n`;
  const f = path.join(dir, 'start.sh'); fs.writeFileSync(f, sh); try { fs.chmodSync(f, 0o755); } catch {}
}
function writeBackupScript(dir) {
  const sh = `#!/usr/bin/env bash
cd "$(dirname "$0")"
mkdir -p backups
TS=$(date +%Y%m%d-%H%M%S)
WORLD=$(grep -E '^level-name=' server.properties 2>/dev/null | cut -d= -f2)
WORLD=\${WORLD:-world}
tar -czf "backups/\${WORLD}-\${TS}.tar.gz" "$WORLD" 2>/dev/null
ls -1t backups/*.tar.gz 2>/dev/null | tail -n +8 | xargs -r rm -f
`;
  const f = path.join(dir, 'backup.sh'); fs.writeFileSync(f, sh); try { fs.chmodSync(f, 0o755); } catch {}
}
function uniqueId(base) {
  const existing = new Set(listInstanceIds());
  let id = base, n = 2;
  while (existing.has(id)) id = `${base}-${n++}`;
  return id;
}
async function createInstance({ name, loader, version }) {
  if (!multiEnabled()) throw new Error('multi-servidor não está ativo (defina serversDir no config.json)');
  loader = loader === 'fabric' ? 'fabric' : 'paper';
  const id = uniqueId(slugifyId(name));
  const dir = path.join(CONFIG.serversDir, id);
  fs.mkdirSync(dir, { recursive: true });
  let resolvedVer = version || '';
  try {
    const r = loader === 'paper' ? await paperResolve(version) : await fabricResolveServer(version);
    resolvedVer = r.version;
    await download(r.url, path.join(dir, 'server.jar'));
  } catch (e) {
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
    throw new Error('falha ao baixar o servidor: ' + e.message);
  }
  const port = freePort(), rconPort = port + 10, rconPass = crypto.randomBytes(6).toString('hex');
  fs.writeFileSync(path.join(dir, 'eula.txt'), 'eula=true\n');
  fs.writeFileSync(path.join(dir, '.craftbox-loader'), loader + '\n');
  writeServerProps(dir, { port, rconPort, rconPass, motd: name });
  writeStartScript(dir); writeBackupScript(dir);
  fs.mkdirSync(path.join(dir, loader === 'fabric' ? 'mods' : 'plugins'), { recursive: true });
  writeInstanceMeta(dir, { name: name || id, loader, mcVersion: resolvedVer, port, createdAt: Date.now() });
  if (!CONFIG.activeServer) { CONFIG.activeServer = id; saveConfig(); }
  return { id, name: name || id, loader, mcVersion: resolvedVer, port };
}
async function cloneInstance(srcId, name) {
  if (!multiEnabled()) throw new Error('multi-servidor não está ativo');
  const src = path.join(CONFIG.serversDir, srcId);
  if (!fs.existsSync(src)) throw new Error('servidor de origem não existe');
  const id = uniqueId(slugifyId(name || (srcId + '-copia')));
  const dir = path.join(CONFIG.serversDir, id);
  fs.cpSync(src, dir, { recursive: true });
  try { fs.rmSync(path.join(dir, 'logs'), { recursive: true, force: true }); } catch {}
  const meta = readInstanceMeta(dir);
  const port = freePort(), rconPort = port + 10;
  meta.name = name || ((meta.name || srcId) + ' (cópia)');
  meta.port = port; meta.createdAt = Date.now();
  writeInstanceMeta(dir, meta);
  updatePropsFile(dir, { 'server-port': String(port), 'query.port': String(port), 'rcon.port': String(rconPort) });
  return { id, name: meta.name, loader: meta.loader, mcVersion: meta.mcVersion, port };
}
async function deleteInstance(id) {
  if (!multiEnabled()) throw new Error('multi-servidor não está ativo');
  const dir = path.join(CONFIG.serversDir, id);
  if (!fs.existsSync(dir)) throw new Error('servidor não existe');
  await svcAction('stop', CONFIG.serviceTemplate + id).catch(() => {});
  fs.rmSync(dir, { recursive: true, force: true });
  if (CONFIG.activeServer === id) { CONFIG.activeServer = listInstanceIds()[0] || ''; saveConfig(); }
  return { ok: true };
}
// ---------------------------------------------------------------------------
// Modpacks (.mrpack do Modrinth) — Fabric/Quilt/Forge/NeoForge
// ---------------------------------------------------------------------------
async function unzipTo(zip, dest) {
  const r = await run('unzip', ['-o', '-q', zip, '-d', dest], { timeout: 180000 });
  if (r.code !== 0) throw new Error('unzip falhou: ' + (r.stderr || r.stdout).slice(-200));
}
async function modpackSearch(query, loaderFilter) {
  const facets = ['["project_type:modpack"]'];
  if (loaderFilter) facets.push(`["categories:${loaderFilter}"]`);
  const url = `https://api.modrinth.com/v2/search?limit=40&index=relevance&query=${encodeURIComponent(query || '')}&facets=${encodeURIComponent('[' + facets.join(',') + ']')}`;
  const data = await httpsJson(url);
  return (data.hits || []).map(h => ({
    slug: h.slug, projectId: h.project_id, title: h.title, author: h.author,
    description: h.description, downloads: h.downloads, follows: h.follows, icon: h.icon_url,
    categories: (h.display_categories || h.categories || []).filter(c => !LOADER_TAGS.has(c)),
  }));
}
async function modpackVersions(slug) {
  const vers = await httpsJson(`https://api.modrinth.com/v2/project/${encodeURIComponent(slug)}/version`);
  return (vers || []).map(v => {
    const f = v.files.find(x => x.primary) || v.files[0];
    return { id: v.id, name: v.name, versionNumber: v.version_number, gameVersions: v.game_versions || [], loaders: v.loaders || [], datePublished: v.date_published, versionType: v.version_type, url: f ? f.url : null };
  });
}
function pickModpackVersion(versions, versionId) {
  if (versionId) { const v = versions.find(x => x.id === versionId); if (v) return v; }
  return versions.find(v => v.versionType === 'release') || versions[0] || null;
}
async function installFabricServer(dir, mc, loaderVer) {
  const insts = await httpsJson('https://meta.fabricmc.net/v2/versions/installer');
  const inst = (insts.find(i => i.stable) || insts[0]).version;
  let lv = loaderVer;
  if (!lv) { const ls = await httpsJson('https://meta.fabricmc.net/v2/versions/loader'); lv = (ls.find(l => l.stable) || ls[0]).version; }
  const url = `https://meta.fabricmc.net/v2/versions/loader/${encodeURIComponent(mc)}/${encodeURIComponent(lv)}/${encodeURIComponent(inst)}/server/jar`;
  await download(url, path.join(dir, 'server.jar'));
}
async function installQuiltServer(dir, mc, loaderVer) {
  const insts = await httpsJson('https://meta.quiltmc.org/v3/versions/installer');
  const iv = insts[0].version;
  const url = `https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-installer/${iv}/quilt-installer-${iv}.jar`;
  const jar = path.join(dir, 'quilt-installer.jar');
  await download(url, jar);
  const args = ['-jar', jar, 'install', 'server', mc];
  if (loaderVer) args.push('--loader=' + loaderVer);
  args.push('--download-server', '--install-dir=' + dir);
  const r = await run('/usr/bin/java', args, { cwd: dir, timeout: 600000 });
  if (r.code !== 0) throw new Error('instalador Quilt falhou: ' + (r.stderr || r.stdout).slice(-200));
  try { fs.unlinkSync(jar); } catch {}
  const launch = path.join(dir, 'quilt-server-launch.jar');
  if (fs.existsSync(launch)) { try { fs.copyFileSync(launch, path.join(dir, 'server.jar')); } catch {} }
}
function findArgsFile(dir) {
  const found = [];
  (function walk(d) {
    let e = []; try { e = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const x of e) { const fp = path.join(d, x.name); if (x.isDirectory()) walk(fp); else if (x.name === 'unix_args.txt') found.push(fp); }
  })(path.join(dir, 'libraries'));
  return found[0] ? path.relative(dir, found[0]) : null;
}
async function runInstaller(dir, url, label) {
  const jar = path.join(dir, 'installer.jar');
  await download(url, jar);
  const r = await run('/usr/bin/java', ['-jar', jar, '--installServer'], { cwd: dir, timeout: 900000 });
  try { fs.unlinkSync(jar); } catch {}
  try { fs.unlinkSync(path.join(dir, 'installer.jar.log')); } catch {}
  if (r.code !== 0) throw new Error(`instalador ${label} falhou: ` + (r.stderr || r.stdout).slice(-250));
}
async function installForgeServer(dir, mc, forgeVer) {
  const v = `${mc}-${forgeVer}`;
  await runInstaller(dir, `https://maven.minecraftforge.net/net/minecraftforge/forge/${v}/forge-${v}-installer.jar`, 'Forge');
}
async function installNeoForgeServer(dir, mc, neoVer) {
  await runInstaller(dir, `https://maven.neoforged.net/releases/net/neoforged/neoforge/${neoVer}/neoforge-${neoVer}-installer.jar`, 'NeoForge');
}
function writeForgeStart(dir) {
  const heap = heapMB();
  fs.writeFileSync(path.join(dir, 'user_jvm_args.txt'), `-Xms512M\n-Xmx${heap}M\n`);
  const argsRel = findArgsFile(dir);
  let execline;
  if (argsRel) execline = `exec /usr/bin/java @user_jvm_args.txt @${argsRel} nogui`;
  else if (fs.existsSync(path.join(dir, 'run.sh'))) execline = `exec bash run.sh nogui`;
  else execline = `exec /usr/bin/java -Xmx${heap}M -jar server.jar nogui`;
  const sh = `#!/usr/bin/env bash\ncd "$(dirname "$0")"\n${execline}\n`;
  const f = path.join(dir, 'start.sh'); fs.writeFileSync(f, sh); try { fs.chmodSync(f, 0o755); } catch {}
}
async function createFromModpack({ name, slug, versionId }) {
  if (!multiEnabled()) throw new Error('multi-servidor não está ativo');
  const versions = await modpackVersions(slug);
  const v = pickModpackVersion(versions, versionId);
  if (!v || !v.url) throw new Error('versão do modpack não encontrada');
  const id = uniqueId(slugifyId(name || slug));
  const dir = path.join(CONFIG.serversDir, id);
  const tmp = path.join(os.tmpdir(), 'craftbox-mrpack-' + id);
  fs.mkdirSync(dir, { recursive: true }); fs.mkdirSync(tmp, { recursive: true });
  let loader, mc, loaderVer;
  try {
    const packFile = path.join(tmp, 'pack.mrpack');
    await download(v.url, packFile);
    await unzipTo(packFile, tmp);
    const idx = JSON.parse(fs.readFileSync(path.join(tmp, 'modrinth.index.json'), 'utf8'));
    const deps = idx.dependencies || {};
    mc = deps.minecraft;
    if (deps['fabric-loader']) { loader = 'fabric'; loaderVer = deps['fabric-loader']; }
    else if (deps['quilt-loader']) { loader = 'quilt'; loaderVer = deps['quilt-loader']; }
    else if (deps['forge']) { loader = 'forge'; loaderVer = deps['forge']; }
    else if (deps['neoforge']) { loader = 'neoforge'; loaderVer = deps['neoforge']; }
    else throw new Error('loader do modpack não reconhecido');
    // arquivos do lado servidor
    for (const f of (idx.files || [])) {
      if (f.env && f.env.server === 'unsupported') continue;
      const rel = String(f.path || '').replace(/\\/g, '/');
      if (!rel || rel.includes('..') || rel.startsWith('/')) continue;
      const dl = f.downloads && f.downloads[0]; if (!dl) continue;
      const dest = path.join(dir, rel);
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      await download(dl, dest);
    }
    // overrides (server-overrides tem prioridade)
    for (const ov of ['overrides', 'server-overrides']) {
      const src = path.join(tmp, ov);
      if (fs.existsSync(src)) fs.cpSync(src, dir, { recursive: true });
    }
    // loader/servidor
    if (loader === 'fabric') await installFabricServer(dir, mc, loaderVer);
    else if (loader === 'quilt') await installQuiltServer(dir, mc, loaderVer);
    else if (loader === 'forge') await installForgeServer(dir, mc, loaderVer);
    else if (loader === 'neoforge') await installNeoForgeServer(dir, mc, loaderVer);
  } catch (e) {
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
    throw new Error('falha ao instalar o modpack: ' + e.message);
  }
  try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
  const port = freePort(), rconPort = port + 10, rconPass = crypto.randomBytes(6).toString('hex');
  if (!fs.existsSync(path.join(dir, 'eula.txt'))) fs.writeFileSync(path.join(dir, 'eula.txt'), 'eula=true\n');
  fs.writeFileSync(path.join(dir, '.craftbox-loader'), loader + '\n');
  if (!fs.existsSync(path.join(dir, 'server.properties'))) writeServerProps(dir, { port, rconPort, rconPass, motd: name });
  else updatePropsFile(dir, { 'server-port': String(port), 'query.port': String(port), 'enable-rcon': 'true', 'rcon.port': String(rconPort), 'rcon.password': rconPass });
  if (loader === 'forge' || loader === 'neoforge') writeForgeStart(dir); else writeStartScript(dir);
  writeBackupScript(dir);
  fs.mkdirSync(path.join(dir, 'mods'), { recursive: true });
  let proj = null; try { proj = await modrinthProject(slug); } catch {}
  const modpack = { slug, versionId: v.id, version: v.versionNumber, name: proj ? proj.title : slug, url: `https://modrinth.com/modpack/${slug}` };
  writeInstanceMeta(dir, { name: name || (proj ? proj.title : slug), loader, mcVersion: mc || '', loaderVersion: loaderVer, port, createdAt: Date.now(), modpack });
  if (!CONFIG.activeServer) { CONFIG.activeServer = id; saveConfig(); }
  return { id, name: name || (proj ? proj.title : slug), loader, mcVersion: mc, port, modpack };
}

function loaderOf(dir, metaLoader) {
  if (metaLoader) return metaLoader;
  try { const m = fs.readFileSync(path.join(dir, '.craftbox-loader'), 'utf8').trim(); if (m) return m; } catch {}
  if (fs.existsSync(path.join(dir, 'mods'))) return 'fabric';
  if (fs.existsSync(path.join(dir, 'plugins'))) return 'paper';
  return 'paper';
}
async function listServersDetailed() {
  const ids = listInstanceIds();
  const activeId = currentId(null);
  const servers = [];
  for (const id of ids) {
    const ctx = serverCtx(id);
    let active = 'unknown';
    try { active = await svcActiveOf(ctx.service); } catch {}
    servers.push({ id, name: ctx.name, loader: loaderOf(ctx.dir, ctx.loader), mcVersion: ctx.mcVersion, port: ctx.port, active, selected: id === activeId, modpack: ctx.modpack });
  }
  return { multi: multiEnabled(), activeId, servers };
}

// ---------------------------------------------------------------------------
// Integracoes (playit / tailscale / cloudflare) — expor o servidor / rede
// Rodam como servicos de usuario do systemd (rootless).
// ---------------------------------------------------------------------------
function integrationsDir() {
  const base = CONFIG.integrationsDir
    || (CONFIG.serversDir ? path.join(path.dirname(CONFIG.serversDir), 'craftbox-integrations') : path.join(os.homedir(), '.craftbox-integrations'));
  try { fs.mkdirSync(base, { recursive: true }); } catch {}
  return base;
}
function userUnitDir() { const d = path.join(os.homedir(), '.config', 'systemd', 'user'); try { fs.mkdirSync(d, { recursive: true }); } catch {} return d; }
function writeUserUnit(unit, desc, execStart) {
  const content = `[Unit]\nDescription=${desc}\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart=${execStart}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n`;
  fs.writeFileSync(path.join(userUnitDir(), unit + '.service'), content);
}
async function uSvc(action, unit) { return run('systemctl', ['--user', action, unit]); }
async function uActive(unit) { const r = await run('systemctl', ['--user', 'is-active', unit]); return (r.stdout || '').trim() || 'unknown'; }
async function uJournal(unit, lines) { const r = await run('journalctl', ['--user', '-u', unit, '-n', String(lines || 120), '--no-pager', '-o', 'cat']); return r.stdout || ''; }
const IARCH = os.arch() === 'arm64' ? 'aarch64' : 'amd64';
const PLAYIT_URL = `https://github.com/playit-cloud/playit-agent/releases/latest/download/playit-linux-${IARCH}`;
const CLOUDFLARED_URL = `https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-${os.arch() === 'arm64' ? 'arm64' : 'amd64'}`;

async function integrationsStatus() {
  const dir = integrationsDir();
  const playitBin = path.join(dir, 'playit');
  const cfBin = path.join(dir, 'cloudflared');
  // playit (daemon roda com um secret key criado no playit.gg)
  const playit = { installed: fs.existsSync(playitBin), hasSecret: fs.existsSync(path.join(dir, 'playit.toml')), running: false, address: null };
  if (playit.installed && await uActive('craftbox-playit') === 'active') {
    playit.running = true;
    const log = await uJournal('craftbox-playit', 150);
    const ad = log.match(/[a-z0-9-]+\.(?:craft\.)?playit\.gg(?::\d+)?|\d+\.tcp\.playit\.gg(?::\d+)?/i);
    if (ad) playit.address = ad[0];
  }
  // cloudflared
  const cloudflare = { installed: fs.existsSync(cfBin), running: false, hasToken: fs.existsSync(path.join(dir, 'cloudflared.token')) };
  if (cloudflare.installed && await uActive('craftbox-cloudflared') === 'active') cloudflare.running = true;
  // tailscale (binario do sistema)
  const tailscale = { installed: false, running: false, ip: null, state: '' };
  const tv = await run('tailscale', ['version']);
  tailscale.installed = tv.code === 0;
  if (tailscale.installed) {
    const st = await run('tailscale', ['status', '--json']);
    if (st.code === 0) { try { const j = JSON.parse(st.stdout); tailscale.state = j.BackendState || ''; tailscale.running = j.BackendState === 'Running'; if (j.Self && j.Self.TailscaleIPs && j.Self.TailscaleIPs.length) tailscale.ip = j.Self.TailscaleIPs.find(x => x.includes('.')) || j.Self.TailscaleIPs[0]; } catch {} }
  }
  return { dir, systemctlUser: CONFIG.systemctlUser, playit, cloudflare, tailscale };
}
async function integrationInstall(name) {
  const dir = integrationsDir();
  if (name === 'playit') {
    const bin = path.join(dir, 'playit');
    await download(PLAYIT_URL, bin); try { fs.chmodSync(bin, 0o755); } catch {}
    return { ok: true };
  }
  if (name === 'cloudflare') {
    const bin = path.join(dir, 'cloudflared');
    await download(CLOUDFLARED_URL, bin); try { fs.chmodSync(bin, 0o755); } catch {}
    return { ok: true };
  }
  if (name === 'tailscale') {
    const tv = await run('tailscale', ['version']);
    if (tv.code === 0) return { ok: true, note: 'Tailscale já está instalado no sistema.' };
    throw new Error('Tailscale precisa ser instalado no sistema (root): no Fedora, "sudo dnf install tailscale && sudo systemctl enable --now tailscaled".');
  }
  throw new Error('integração desconhecida');
}
async function integrationStart(name) {
  const dir = integrationsDir();
  if (name === 'playit') {
    const bin = path.join(dir, 'playit');
    const toml = path.join(dir, 'playit.toml');
    if (!fs.existsSync(bin)) throw new Error('playit não instalado');
    if (!fs.existsSync(toml)) throw new Error('cole o secret key do playit.gg primeiro');
    writeUserUnit('craftbox-playit', 'craftbox — playit.gg agent', `${bin} --secret-path ${toml} --socket-path ${path.join(dir, 'playit.sock')}`);
    await run('systemctl', ['--user', 'daemon-reload']);
    const r = await uSvc('start', 'craftbox-playit');
    if (r.code !== 0) throw new Error(r.stderr || 'falha ao iniciar playit');
    return { ok: true };
  }
  if (name === 'cloudflare') {
    const bin = path.join(dir, 'cloudflared');
    const tokFile = path.join(dir, 'cloudflared.token');
    if (!fs.existsSync(bin)) throw new Error('cloudflared não instalado');
    if (!fs.existsSync(tokFile)) throw new Error('configure o token do túnel Cloudflare primeiro');
    const token = fs.readFileSync(tokFile, 'utf8').trim();
    writeUserUnit('craftbox-cloudflared', 'craftbox — Cloudflare Tunnel', `${bin} tunnel --no-autoupdate run --token ${token}`);
    await run('systemctl', ['--user', 'daemon-reload']);
    const r = await uSvc('start', 'craftbox-cloudflared');
    if (r.code !== 0) throw new Error(r.stderr || 'falha ao iniciar cloudflared');
    return { ok: true };
  }
  if (name === 'tailscale') {
    const r = await run('tailscale', ['up', '--accept-routes'], { timeout: 20000 });
    const url = (r.stdout + r.stderr).match(/https:\/\/login\.tailscale\.com\/\S+/);
    if (url) return { ok: true, loginUrl: url[0] };
    if (r.code === 0) return { ok: true };
    throw new Error((r.stderr || r.stdout || 'falha').slice(-200) + ' (Tailscale normalmente precisa de root: "sudo tailscale up")');
  }
  throw new Error('integração desconhecida');
}
async function integrationStop(name) {
  if (name === 'tailscale') { const r = await run('tailscale', ['down']); return { ok: r.code === 0 }; }
  const unit = name === 'playit' ? 'craftbox-playit' : name === 'cloudflare' ? 'craftbox-cloudflared' : null;
  if (!unit) throw new Error('integração desconhecida');
  await uSvc('stop', unit);
  return { ok: true };
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------
function json(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8', 'Content-Length': Buffer.byteLength(body) });
  res.end(body);
}
function readBody(req) {
  return new Promise((resolve) => {
    let data = ''; req.on('data', c => { data += c; if (data.length > 1e6) req.destroy(); });
    req.on('end', () => { try { resolve(JSON.parse(data || '{}')); } catch { resolve({}); } });
  });
}
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.ico': 'image/x-icon' };
function serveStatic(res, file) {
  const full = path.join(PUBLIC, file);
  if (!full.startsWith(PUBLIC)) { res.writeHead(403); return res.end('forbidden'); }
  fs.readFile(full, (err, data) => {
    if (err) { res.writeHead(404); return res.end('not found'); }
    res.writeHead(200, { 'Content-Type': MIME[path.extname(full)] || 'application/octet-stream' });
    res.end(data);
  });
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
const server = http.createServer((req, res) => {
  const url = new URL(req.url, 'http://localhost');
  als.run(serverCtx(currentId(url)), async () => {
  const p = url.pathname;

  try {
    // --- login (nao exige sessao) ---
    if (p === '/api/login' && req.method === 'POST') {
      const body = await readBody(req);
      const { salt, hash } = CONFIG.auth;
      if (!salt || !hash) return json(res, 500, { error: 'Senha nao configurada. Rode: node server.js --hash SENHA' });
      const calc = crypto.scryptSync(String(body.password || ''), salt, 64).toString('hex');
      const ok = calc.length === hash.length && crypto.timingSafeEqual(Buffer.from(calc), Buffer.from(hash));
      if (!ok) return json(res, 401, { error: 'Senha incorreta' });
      res.setHeader('Set-Cookie', `cbsession=${makeSession()}; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`);
      return json(res, 200, { ok: true });
    }
    if (p === '/api/logout' && req.method === 'POST') {
      res.setHeader('Set-Cookie', 'cbsession=; HttpOnly; Path=/; Max-Age=0');
      return json(res, 200, { ok: true });
    }
    if (p === '/api/authcheck') return json(res, 200, { authed: isAuthed(req), configured: !!(CONFIG.auth.salt && CONFIG.auth.hash) });

    // --- estaticos + pagina ---
    if (p === '/' ) return serveStatic(res, 'index.html');
    if (p.startsWith('/public/')) return serveStatic(res, p.slice('/public/'.length));
    if (p === '/app.js' || p === '/style.css') return serveStatic(res, p.slice(1));

    // --- daqui pra baixo exige sessao ---
    if (p.startsWith('/api/')) {
      if (!isAuthed(req)) return json(res, 401, { error: 'nao autenticado' });

      if (p === '/api/status') {
        const active = await svcActive();
        let players = null, version = null;
        if (active === 'active') {
          try {
            const list = await rcon('list'); // "There are X of a max of Y players online: ..."
            const m = list.match(/(\d+)\s+of a max of\s+(\d+)/i) || list.match(/(\d+)\/(\d+)/);
            if (m) players = { online: +m[1], max: +m[2] };
          } catch {}
        }
        return json(res, 200, { active, uptime: await svcUptime(), players, system: await systemStats() });
      }
      if (p === '/api/power' && req.method === 'POST') {
        const { action } = await readBody(req);
        const r = await svcAction(action);
        return json(res, r.code === 0 ? 200 : 500, { ok: r.code === 0, output: r.stderr || r.stdout });
      }
      if (p === '/api/properties' && req.method === 'GET') return json(res, 200, { properties: readProps() });
      if (p === '/api/properties' && req.method === 'PUT') {
        const body = await readBody(req);
        if (!body || typeof body !== 'object') return json(res, 400, { error: 'payload invalido' });
        writeProps(body); // valores como strings
        return json(res, 200, { ok: true, properties: readProps() });
      }
      if (p === '/api/rcon' && req.method === 'POST') {
        const { command } = await readBody(req);
        if (!command) return json(res, 400, { error: 'comando vazio' });
        try { return json(res, 200, { response: await rcon(command) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/logs') {
        const n = Math.min(1000, parseInt(url.searchParams.get('lines') || '200', 10));
        let text = '';
        try {
          const data = fs.readFileSync(path.join(SRV().dir, 'logs', 'latest.log'), 'utf8');
          text = data.split('\n').slice(-n).join('\n');
        } catch {
          const r = await run('journalctl', ['-u', SRV().service, '-n', String(n), '--no-pager', '-o', 'cat']);
          text = r.stdout || '(sem logs)';
        }
        return json(res, 200, { log: text });
      }
      if (p === '/api/backups' && req.method === 'GET') {
        let items = [];
        try {
          const dir = path.join(SRV().dir, 'backups');
          items = fs.readdirSync(dir).filter(f => f.endsWith('.tar.gz')).map(f => {
            const st = fs.statSync(path.join(dir, f));
            return { name: f, sizeMB: +(st.size / 1048576).toFixed(1), mtime: st.mtimeMs };
          }).sort((a, b) => b.mtime - a.mtime);
        } catch {}
        return json(res, 200, { backups: items });
      }
      if (p === '/api/backups' && req.method === 'POST') {
        const r = await run('bash', [path.join(SRV().dir, 'backup.sh')], { timeout: 120000 });
        return json(res, r.code === 0 ? 200 : 500, { ok: r.code === 0, output: r.stderr || r.stdout || 'backup executado' });
      }
      // --- conteudo (mods/plugins via Modrinth) ---
      if (p === '/api/content/info') {
        const loader = detectLoader();
        return json(res, 200, { loader, kind: loader === 'fabric' ? 'mods' : 'plugins', mcVersion: detectMcVersion(), onlineMode: readProps()['online-mode'] !== 'false', sorts: SORTS });
      }
      if (p === '/api/content/search') {
        try {
          const q = url.searchParams.get('q') || '';
          const opts = { sort: url.searchParams.get('sort') || 'relevance', category: url.searchParams.get('category') || '' };
          return json(res, 200, { results: await modrinthSearch(q, detectLoader(), detectMcVersion(), opts) });
        } catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/project') {
        const slug = url.searchParams.get('slug') || '';
        if (!slug) return json(res, 400, { error: 'slug vazio' });
        try { return json(res, 200, { project: await modrinthProject(slug) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/versions') {
        const slug = url.searchParams.get('slug') || '';
        if (!slug) return json(res, 400, { error: 'slug vazio' });
        try { return json(res, 200, { mcVersion: detectMcVersion(), versions: await modrinthVersions(slug, detectLoader(), detectMcVersion()) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/installed' && req.method === 'GET') {
        return json(res, 200, listInstalled());
      }
      if (p === '/api/content/install' && req.method === 'POST') {
        const { slug, versionId } = await readBody(req);
        if (!slug) return json(res, 400, { error: 'slug vazio' });
        try {
          const installed = await installProject(slug, detectLoader(), detectMcVersion(), versionId || null);
          return json(res, 200, { ok: true, installed });
        } catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/toggle' && req.method === 'POST') {
        const { file, enabled } = await readBody(req);
        if (!file || file.includes('/') || file.includes('..')) return json(res, 400, { error: 'arquivo invalido' });
        try {
          const folder = contentFolder(detectLoader());
          const base = file.endsWith('.disabled') ? file.slice(0, -'.disabled'.length) : file;
          const active = path.join(folder, base), off = path.join(folder, base + '.disabled');
          if (enabled) { if (fs.existsSync(off)) fs.renameSync(off, active); }
          else { if (fs.existsSync(active)) fs.renameSync(active, off); }
          const man = readManifest();
          for (const s of Object.keys(man)) if (man[s].filename === base) { man[s].disabled = !enabled; }
          writeManifest(man);
          return json(res, 200, { ok: true });
        } catch (e) { return json(res, 500, { error: e.message }); }
      }
      if (p === '/api/content/updates') {
        try { return json(res, 200, { updates: await checkUpdates() }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/installed' && req.method === 'DELETE') {
        const file = url.searchParams.get('file') || '';
        const slug = url.searchParams.get('slug') || '';
        try {
          const folder = contentFolder(detectLoader());
          const man = readManifest();
          if (slug) {
            const m = man[slug];
            if (m && m.filename) { try { fs.unlinkSync(path.join(folder, m.filename)); } catch {} try { fs.unlinkSync(path.join(folder, m.filename + '.disabled')); } catch {} }
            delete man[slug]; writeManifest(man);
            return json(res, 200, { ok: true });
          }
          if (!file || file.includes('/') || file.includes('..')) return json(res, 400, { error: 'arquivo invalido' });
          fs.unlinkSync(path.join(folder, file));
          const base = file.endsWith('.disabled') ? file.slice(0, -'.disabled'.length) : file;
          for (const s of Object.keys(man)) if (man[s].filename === base) delete man[s];
          writeManifest(man);
          return json(res, 200, { ok: true });
        } catch (e) { return json(res, 500, { error: e.message }); }
      }
      // --- compatibilidade ---
      if (p === '/api/compat/offline' && req.method === 'POST') {
        const { enabled } = await readBody(req);
        writeProps({ 'online-mode': enabled ? 'false' : 'true' });
        return json(res, 200, { ok: true, onlineMode: !enabled });
      }
      if (p === '/api/compat/bedrock' && req.method === 'POST') {
        try { return json(res, 200, { ok: true, ...(await installBedrock()) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      // --- multi-servidor ---
      if (p === '/api/servers' && req.method === 'GET') return json(res, 200, await listServersDetailed());
      if (p === '/api/servers/select' && req.method === 'POST') {
        const { id } = await readBody(req);
        if (!multiEnabled()) return json(res, 400, { error: 'multi-servidor desativado' });
        if (!listInstanceIds().includes(id)) return json(res, 404, { error: 'servidor não existe' });
        CONFIG.activeServer = id; saveConfig();
        return json(res, 200, { ok: true, activeId: id });
      }
      if (p === '/api/servers/create' && req.method === 'POST') {
        const b = await readBody(req);
        if (!b.name || !String(b.name).trim()) return json(res, 400, { error: 'dê um nome ao servidor' });
        try { return json(res, 200, { ok: true, server: await createInstance({ name: b.name, loader: b.loader, version: b.version }) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/servers/clone' && req.method === 'POST') {
        const b = await readBody(req);
        try { return json(res, 200, { ok: true, server: await cloneInstance(b.id, b.name) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/modpacks/search') {
        try { return json(res, 200, { results: await modpackSearch(url.searchParams.get('q') || '', url.searchParams.get('loader') || '') }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/modpacks/versions') {
        const slug = url.searchParams.get('slug') || '';
        if (!slug) return json(res, 400, { error: 'slug vazio' });
        try { return json(res, 200, { versions: await modpackVersions(slug) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/servers/create-modpack' && req.method === 'POST') {
        const b = await readBody(req);
        if (!b.slug) return json(res, 400, { error: 'modpack não informado' });
        try { return json(res, 200, { ok: true, server: await createFromModpack({ name: b.name, slug: b.slug, versionId: b.versionId }) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/servers' && req.method === 'DELETE') {
        const id = url.searchParams.get('id') || '';
        try { return json(res, 200, await deleteInstance(id)); }
        catch (e) { return json(res, 500, { error: e.message }); }
      }
      // --- integracoes (playit / tailscale / cloudflare) ---
      if (p === '/api/integrations' && req.method === 'GET') {
        try { return json(res, 200, await integrationsStatus()); }
        catch (e) { return json(res, 500, { error: e.message }); }
      }
      if (p === '/api/integrations/install' && req.method === 'POST') {
        const { name } = await readBody(req);
        try { return json(res, 200, await integrationInstall(name)); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/integrations/start' && req.method === 'POST') {
        const { name } = await readBody(req);
        try { return json(res, 200, await integrationStart(name)); }
        catch (e) { return json(res, 500, { error: e.message }); }
      }
      if (p === '/api/integrations/stop' && req.method === 'POST') {
        const { name } = await readBody(req);
        try { return json(res, 200, await integrationStop(name)); }
        catch (e) { return json(res, 500, { error: e.message }); }
      }
      if (p === '/api/integrations/log' && req.method === 'GET') {
        const name = url.searchParams.get('name') || '';
        const unit = name === 'playit' ? 'craftbox-playit' : name === 'cloudflare' ? 'craftbox-cloudflared' : null;
        if (!unit) return json(res, 400, { error: 'nome inválido' });
        return json(res, 200, { log: await uJournal(unit, 120) });
      }
      if (p === '/api/integrations/cloudflare-token' && req.method === 'POST') {
        const { token } = await readBody(req);
        if (!token || !String(token).trim()) return json(res, 400, { error: 'token vazio' });
        const f = path.join(integrationsDir(), 'cloudflared.token');
        fs.writeFileSync(f, String(token).trim()); try { fs.chmodSync(f, 0o600); } catch {}
        return json(res, 200, { ok: true });
      }
      if (p === '/api/integrations/playit-secret' && req.method === 'POST') {
        const { secret } = await readBody(req);
        const s = String(secret || '').trim();
        if (!/^[0-9a-fA-F]{16,}$/.test(s)) return json(res, 400, { error: 'secret inválido (é uma sequência hexadecimal do playit.gg)' });
        const f = path.join(integrationsDir(), 'playit.toml');
        fs.writeFileSync(f, `secret_key = "${s}"\n`); try { fs.chmodSync(f, 0o600); } catch {}
        return json(res, 200, { ok: true });
      }

      return json(res, 404, { error: 'endpoint desconhecido' });
    }

    res.writeHead(404); res.end('not found');
  } catch (e) {
    json(res, 500, { error: e.message });
  }
  });
});

server.requestTimeout = 0; // instalacoes de modpack podem demorar
server.listen(CONFIG.port, CONFIG.host, () => {
  console.log(`craftbox-panel ouvindo em http://${CONFIG.host}:${CONFIG.port}`);
  if (!CONFIG.auth.salt || !CONFIG.auth.hash) {
    console.log('AVISO: senha nao configurada. Rode:  node server.js --hash SUA_SENHA  e cole no config.json');
  }
});
