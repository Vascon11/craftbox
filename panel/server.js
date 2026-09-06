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
  mcVersion: ''                                            // ex '1.21.1' (auto-detecta do log se vazio)
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
async function svcActive() {
  const r = await run('systemctl', ['is-active', CONFIG.service]);
  return r.stdout.trim() || 'unknown';
}
async function svcAction(action) {
  if (!['start', 'stop', 'restart'].includes(action)) throw new Error('acao invalida');
  // precisa de sudoers permitindo: systemctl <action> <service>
  const r = await run('sudo', ['-n', 'systemctl', action, CONFIG.service]);
  return r;
}
async function svcUptime() {
  const r = await run('systemctl', ['show', CONFIG.service, '--property=ActiveEnterTimestamp', '--value']);
  const t = Date.parse(r.stdout.trim());
  return isNaN(t) ? null : Math.max(0, Math.floor((Date.now() - t) / 1000));
}

// ---------------------------------------------------------------------------
// server.properties
// ---------------------------------------------------------------------------
function propsPath() { return path.join(CONFIG.mcDir, 'server.properties'); }
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
  if (CONFIG.rcon.password) return CONFIG.rcon.password;
  return readProps()['rcon.password'] || '';
}
function rcon(command) {
  return new Promise((resolve, reject) => {
    const pw = rconPassword();
    if (!pw) return reject(new Error('RCON sem senha (habilite enable-rcon no server.properties)'));
    const sock = net.connect({ host: CONFIG.rcon.host, port: CONFIG.rcon.port });
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
    const s = fs.statfsSync(CONFIG.mcDir);
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
  if (CONFIG.loader) return CONFIG.loader;
  try { const m = fs.readFileSync(path.join(CONFIG.mcDir, '.craftbox-loader'), 'utf8').trim(); if (m) return m; } catch {}
  const hasMods = fs.existsSync(path.join(CONFIG.mcDir, 'mods'));
  const hasPlugins = fs.existsSync(path.join(CONFIG.mcDir, 'plugins'));
  if (hasMods && !hasPlugins) return 'fabric';
  if (hasPlugins && !hasMods) return 'paper';
  return hasMods ? 'fabric' : 'paper';
}
function contentFolder(loader) {
  const dir = path.join(CONFIG.mcDir, loader === 'fabric' ? 'mods' : 'plugins');
  try { fs.mkdirSync(dir, { recursive: true }); } catch {}
  return dir;
}
function loaderCategories(loader) { return loader === 'fabric' ? ['fabric'] : ['paper', 'spigot', 'bukkit']; }
function detectMcVersion() {
  if (CONFIG.mcVersion) return CONFIG.mcVersion;
  try {
    const log = fs.readFileSync(path.join(CONFIG.mcDir, 'logs', 'latest.log'), 'utf8');
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
async function modrinthSearch(query, loader, version) {
  const cats = loaderCategories(loader).map(c => `"categories:${c}"`).join(',');
  const facets = [`[${cats}]`];
  if (version) facets.push(`["versions:${version}"]`);
  const url = `https://api.modrinth.com/v2/search?limit=25&query=${encodeURIComponent(query || '')}&facets=${encodeURIComponent('[' + facets.join(',') + ']')}`;
  const data = await httpsJson(url);
  return (data.hits || []).map(h => ({ slug: h.slug, title: h.title, description: h.description, downloads: h.downloads, icon: h.icon_url, type: h.project_type }));
}
async function modrinthResolve(slug, loader, version) {
  const tryLoaders = loader === 'fabric' ? [['fabric']] : [['paper'], ['spigot'], ['bukkit']];
  for (const ls of tryLoaders) {
    let u = `https://api.modrinth.com/v2/project/${slug}/version?loaders=${encodeURIComponent(JSON.stringify(ls))}`;
    if (version) u += `&game_versions=${encodeURIComponent(JSON.stringify([version]))}`;
    let vers = await httpsJson(u).catch(() => []);
    if ((!vers || !vers.length)) { // sem match exato de versao: qualquer versao desse loader
      vers = await httpsJson(`https://api.modrinth.com/v2/project/${slug}/version?loaders=${encodeURIComponent(JSON.stringify(ls))}`).catch(() => []);
    }
    if (vers && vers.length) {
      const v = vers[0];
      const file = v.files.find(f => f.primary) || v.files[0];
      return { filename: path.basename(file.filename), url: file.url, version: v.version_number };
    }
  }
  throw new Error('sem versao compativel pra esse loader');
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
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
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
          const data = fs.readFileSync(path.join(CONFIG.mcDir, 'logs', 'latest.log'), 'utf8');
          text = data.split('\n').slice(-n).join('\n');
        } catch {
          const r = await run('journalctl', ['-u', CONFIG.service, '-n', String(n), '--no-pager', '-o', 'cat']);
          text = r.stdout || '(sem logs)';
        }
        return json(res, 200, { log: text });
      }
      if (p === '/api/backups' && req.method === 'GET') {
        let items = [];
        try {
          const dir = path.join(CONFIG.mcDir, 'backups');
          items = fs.readdirSync(dir).filter(f => f.endsWith('.tar.gz')).map(f => {
            const st = fs.statSync(path.join(dir, f));
            return { name: f, sizeMB: +(st.size / 1048576).toFixed(1), mtime: st.mtimeMs };
          }).sort((a, b) => b.mtime - a.mtime);
        } catch {}
        return json(res, 200, { backups: items });
      }
      if (p === '/api/backups' && req.method === 'POST') {
        const r = await run('bash', [path.join(CONFIG.mcDir, 'backup.sh')], { timeout: 120000 });
        return json(res, r.code === 0 ? 200 : 500, { ok: r.code === 0, output: r.stderr || r.stdout || 'backup executado' });
      }
      // --- conteudo (mods/plugins via Modrinth) ---
      if (p === '/api/content/info') {
        const loader = detectLoader();
        return json(res, 200, { loader, kind: loader === 'fabric' ? 'mods' : 'plugins', mcVersion: detectMcVersion(), onlineMode: readProps()['online-mode'] !== 'false' });
      }
      if (p === '/api/content/search') {
        try { return json(res, 200, { results: await modrinthSearch(url.searchParams.get('q') || '', detectLoader(), detectMcVersion()) }); }
        catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/installed' && req.method === 'GET') {
        const folder = contentFolder(detectLoader());
        let files = [];
        try { files = fs.readdirSync(folder).filter(f => f.endsWith('.jar')).map(f => ({ name: f, sizeMB: +(fs.statSync(path.join(folder, f)).size / 1048576).toFixed(2) })); } catch {}
        return json(res, 200, { folder, files });
      }
      if (p === '/api/content/install' && req.method === 'POST') {
        const { slug } = await readBody(req);
        if (!slug) return json(res, 400, { error: 'slug vazio' });
        try {
          const loader = detectLoader(); const folder = contentFolder(loader);
          const r = await modrinthResolve(slug, loader, detectMcVersion());
          await download(r.url, path.join(folder, r.filename));
          return json(res, 200, { ok: true, file: r.filename, version: r.version });
        } catch (e) { return json(res, 502, { error: e.message }); }
      }
      if (p === '/api/content/installed' && req.method === 'DELETE') {
        const file = url.searchParams.get('file') || '';
        if (!file || file.includes('/') || file.includes('..')) return json(res, 400, { error: 'arquivo invalido' });
        try { fs.unlinkSync(path.join(contentFolder(detectLoader()), file)); return json(res, 200, { ok: true }); }
        catch (e) { return json(res, 500, { error: e.message }); }
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

      return json(res, 404, { error: 'endpoint desconhecido' });
    }

    res.writeHead(404); res.end('not found');
  } catch (e) {
    json(res, 500, { error: e.message });
  }
});

server.listen(CONFIG.port, CONFIG.host, () => {
  console.log(`craftbox-panel ouvindo em http://${CONFIG.host}:${CONFIG.port}`);
  if (!CONFIG.auth.salt || !CONFIG.auth.hash) {
    console.log('AVISO: senha nao configurada. Rode:  node server.js --hash SUA_SENHA  e cole no config.json');
  }
});
