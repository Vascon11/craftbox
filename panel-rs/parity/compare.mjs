// Comparador de paridade Node (panel/server.js) × Rust (panel-rs).
// Chamado pelo run.sh com as variáveis de ambiente do cenário; imprime uma
// linha por verificação e sai com código != 0 se alguma falhar.
import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs';
import net from 'node:net';

const E = process.env;
const SC = E.SCENARIO;
const N = E.NODE_BASE, R = E.RUST_BASE;
let fails = 0, oks = 0, notes = 0;

function req(base, method, path, { body, cookie, headers = {} } = {}) {
  return new Promise((resolve) => {
    const u = new URL(base);
    const h = { ...headers };
    if (cookie) h.cookie = cookie;
    if (body != null) { h['content-type'] = 'application/json'; h['content-length'] = Buffer.byteLength(body); }
    const r = http.request({ host: u.hostname, port: u.port, method, path, headers: h, agent: false }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, names: res.rawHeaders.filter((_, i) => i % 2 === 0), body: Buffer.concat(chunks) }));
    });
    r.on('error', (e) => resolve({ status: 0, headers: {}, names: [], body: Buffer.from(''), error: e.code || e.message }));
    r.setTimeout(20000, () => r.destroy(new Error('timeout')));
    if (body != null) r.write(body);
    r.end();
  });
}

const sha = (b) => crypto.createHash('sha256').update(b).digest('hex').slice(0, 12);
const cookieShape = (sc) => (sc || []).map((c) => c.replace(/^cbsession=s:\d+:[^;.]*\.[A-Za-z0-9_-]{43}/, 'cbsession=<token>'));
const tokenOf = (sc) => { const m = /^cbsession=([^;]*)/.exec((sc || [])[0] || ''); return m ? m[1] : ''; };

// valores voláteis: [regex do caminho, regra]
const VOLATILE = [
  [/^system\.load\.\d$/, { type: 'number' }],
  [/^system\.mem\.(free|used)$/, { tol: 256 }],
  [/^system\.uptimeHost$/, { tol: 3 }],
  [/^system\.cpu\.curMHz$/, { type: 'any' }],
  [/^system\.tempC$/, { tol: 3 }],
  [/^system\.disk\.(usedGB|freeGB)$/, { tol: 0.3 }],
  [/^uptime$/, { tol: 3 }],
  [/^targets\.\d\.ms$/, { type: 'number' }],
  [/^results\.\d+\.(downloads|follows)$/, { type: 'number' }],
  [/^total$/, { type: 'number' }],
];
const kind = (v) => (v === null ? 'null' : Array.isArray(v) ? 'array' : typeof v);

function deepCmp(a, b, path = '', out = [], ignore = null) {
  if (ignore && ignore.test(path)) return out;
  const rule = (VOLATILE.find(([re]) => re.test(path)) || [])[1];
  if (rule && rule.type === 'any') return out;
  if (kind(a) !== kind(b)) { out.push(`${path || '(raiz)'}: tipo ${kind(a)} × ${kind(b)} (${JSON.stringify(a)} × ${JSON.stringify(b)})`); return out; }
  if (kind(a) === 'object') {
    const ka = Object.keys(a), kb = Object.keys(b);
    if (ka.join(',') !== kb.join(',')) out.push(`${path || '(raiz)'}: chaves [${ka}] × [${kb}]`);
    for (const k of ka) if (k in b) deepCmp(a[k], b[k], path ? `${path}.${k}` : k, out, ignore);
  } else if (kind(a) === 'array') {
    if (a.length !== b.length) out.push(`${path}: tamanho ${a.length} × ${b.length}`);
    a.forEach((x, i) => i < b.length && deepCmp(x, b[i], `${path}.${i}`, out, ignore));
  } else if (rule && rule.type) {
    /* só o tipo importa */
  } else if (rule && rule.tol != null) {
    if (Math.abs(a - b) > rule.tol) out.push(`${path}: ${a} × ${b} (tolerância ${rule.tol})`);
  } else if (a !== b) out.push(`${path}: ${JSON.stringify(a)} × ${JSON.stringify(b)}`);
  return out;
}

function report(ok, label, detail = '') {
  if (ok === 'note') { notes++; console.log(`  NOTA  ${label}${detail ? ' — ' + detail : ''}`); return; }
  if (ok) oks++; else fails++;
  console.log(`  ${ok ? 'OK  ' : 'FALHA'} ${label}${detail ? ' — ' + detail : ''}`);
}

const FRAMING = ['content-type', 'transfer-encoding', 'connection', 'keep-alive'];

// compara uma requisição feita igual nos dois lados
async function same(label, method, path, opts = {}) {
  const on = opts.cookieN !== undefined ? { ...opts, cookie: opts.cookieN } : opts;
  const or = opts.cookieR !== undefined ? { ...opts, cookie: opts.cookieR } : opts;
  const [a, b] = await Promise.all([req(N, method, path, on), req(R, method, path, or)]);
  if (E.NORM_NODE && (a.headers['content-type'] || '').startsWith('application/json')) a.body = Buffer.from(a.body.toString().split(E.NORM_NODE).join('<D>'));
  if (E.NORM_RUST && (b.headers['content-type'] || '').startsWith('application/json')) b.body = Buffer.from(b.body.toString().split(E.NORM_RUST).join('<D>'));
  const diffs = [];
  if (a.status !== b.status) diffs.push(`status ${a.status} × ${b.status}`);
  for (const h of FRAMING) if (a.headers[h] !== b.headers[h]) diffs.push(`header ${h}: ${a.headers[h]} × ${b.headers[h]}`);
  const na = a.names.map((x) => x.toLowerCase()).filter((x) => x !== 'date').join(',');
  const nb = b.names.map((x) => x.toLowerCase()).filter((x) => x !== 'date').join(',');
  if (na !== nb) diffs.push(`ordem/nomes de headers [${na}] × [${nb}]`);
  const ca = cookieShape(a.headers['set-cookie']).join('|'), cb = cookieShape(b.headers['set-cookie']).join('|');
  if (ca !== cb) diffs.push(`set-cookie ${ca} × ${cb}`);
  let summary;
  if (method === 'HEAD') {
    if (a.body.length || b.body.length) diffs.push(`HEAD com corpo: ${a.body.length} × ${b.body.length} bytes`);
    summary = `sem corpo (${b.headers['content-type'] || 'sem content-type'})`;
  } else if ((a.headers['content-type'] || '').startsWith('application/json')) {
    let ja, jb;
    try { ja = JSON.parse(a.body); jb = JSON.parse(b.body); } catch (e) { diffs.push('JSON inválido: ' + e.message); }
    if (ja !== undefined && jb !== undefined) {
      diffs.push(...deepCmp(ja, jb, '', [], opts.ignore || null));
      if (!opts.volatile && a.headers['content-length'] !== b.headers['content-length']) diffs.push(`content-length ${a.headers['content-length']} × ${b.headers['content-length']}`);
      summary = JSON.stringify(jb).slice(0, opts.show || 110);
    }
  } else {
    if (!a.body.equals(b.body)) diffs.push(`corpo sha ${sha(a.body)} × ${sha(b.body)} (${a.body.length} × ${b.body.length} bytes)`);
    summary = `${b.body.length} bytes sha ${sha(b.body)}`;
  }
  report(diffs.length === 0, `${method} ${path}${opts.tag ? ' [' + opts.tag + ']' : ''} → ${b.status}`, diffs.length ? diffs.join('; ') : summary);
  return { a, b };
}

// requisição crua (sem cliente HTTP) — lê até o servidor fechar ou 1,5 s
function raw(base, payload) {
  return new Promise((resolve) => {
    const u = new URL(base);
    const sock = net.connect(Number(u.port), u.hostname, () => sock.write(payload));
    const chunks = [];
    const done = () => { sock.destroy(); resolve(Buffer.concat(chunks).toString('latin1')); };
    sock.on('data', (c) => chunks.push(c));
    sock.on('end', done); sock.on('error', done);
    setTimeout(done, 1500);
  });
}
const stripDate = (t) => t.replace(/^Date: .*\r\n/mi, '');

async function sameRaw(label, payload) {
  const [a, b] = await Promise.all([raw(N, payload), raw(R, payload)]);
  const ok = stripDate(a) === stripDate(b);
  report(ok, `bruto: ${label}`, ok ? JSON.stringify(stripDate(b).split('\r\n')[0]) : `${JSON.stringify(stripDate(a))} × ${JSON.stringify(stripDate(b))}`);
}

async function login(base, body) {
  const r = await req(base, 'POST', '/api/login', { body: JSON.stringify(body) });
  return { r, cookie: tokenOf(r.headers['set-cookie']) ? `cbsession=${tokenOf(r.headers['set-cookie'])}` : '' };
}

// B5: nas rotas /api/servers/:id/* o Node perde ip/usuário na auditoria ("-"); o Rust mantém (corrigido)
const B5_ACTIONS = new Set(['ligar', 'desligar', 'reiniciar']);
function auditCmp(label) {
  const read = (p) => { try { return fs.readFileSync(p, 'utf8').trim().split('\n').filter(Boolean).map((l) => JSON.parse(l)); } catch { return []; } };
  const a = read(E.AUDIT_NODE), b = read(E.AUDIT_RUST);
  let b5 = 0;
  const norm = (e) => {
    const b5e = B5_ACTIONS.has(e.action);
    if (b5e) b5++;
    return JSON.stringify([Object.keys(e), b5e ? '*' : e.ip, b5e ? '*' : e.user, e.server, e.action, e.detail, typeof e.ts]);
  };
  const diffs = [];
  if (a.length !== b.length) diffs.push(`${a.length} × ${b.length} entradas`);
  a.forEach((e, i) => { if (b[i] && norm(e) !== norm(b[i])) diffs.push(`#${i}: ${norm(e)} × ${norm(b[i])}`); });
  report(diffs.length === 0 && a.length > 0, `${label}: log de auditoria (${a.length} entradas)`, diffs.join('; ') || a.map((e) => e.action).join(', '));
  if (b5) {
    const bi = b.filter((e) => B5_ACTIONS.has(e.action)), ai = a.filter((e) => B5_ACTIONS.has(e.action));
    report('note', `B5 corrigido no Rust: ${ai.length} entrada(s) de ligar/desligar — Node ip/user "${ai[0] && ai[0].ip}"/"${ai[0] && ai[0].user}", Rust "${bi[0] && bi[0].ip}"/"${bi[0] && bi[0].user}"`);
  }
}

// compara um arquivo gravado pelos dois lados (mesmo caminho relativo), com máscaras
function fileCmp(label, rel, mask = (t) => t) {
  const rd = (base) => { try { return mask(fs.readFileSync(base + '/' + rel, 'utf8').split(base).join('<D>')); } catch (e) { return '<' + e.code + '>'; } };
  const a = rd(E.NORM_NODE), b = rd(E.NORM_RUST);
  report(a === b, `arquivo ${rel}${label ? ' [' + label + ']' : ''}`, a === b ? `${a.length} bytes` : `\n    NODE: ${JSON.stringify(a).slice(0, 700)}\n    RUST: ${JSON.stringify(b).slice(0, 700)}`);
}
const maskSecrets = (t) => t.replace(/^  "port": \d+,$/m, '  "port": <CRAFTBOX_PORT>,').replace(/"(salt|hash)": "[0-9a-f]+"/g, '"$1": "<hex>"').replace(/"createdAt": [0-9.]+/g, '"createdAt": <ts>').replace(/rcon\.password=[0-9a-f]+/g, 'rcon.password=<hex>').replace(/"installedAt": [0-9.]+/g, '"installedAt": <ts>');
const exists = (base, rel) => fs.existsSync(base + '/' + rel);
function existsCmp(rel) {
  const a = exists(E.NORM_NODE, rel), b = exists(E.NORM_RUST, rel);
  report(a === b, `existe ${rel}: ${a} × ${b}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------
// Cenário D: todas as rotas das fatias 2–7, com uma cópia idêntica dos dados
// pra cada lado (as rotas gravam arquivos; depois comparamos os arquivos).
// ---------------------------------------------------------------------------
async function scenarioD() {
  console.log('\n== Cenário D: fatias 2–7 (energia, backups, conteúdo, servidores, modpacks, integrações, contas) ==');
  const ln = await login(N, { password: E.PASSWORD }), lr = await login(R, { password: E.PASSWORD });
  const c = { cookieN: ln.cookie, cookieR: lr.cookie };
  const J = (o) => JSON.stringify(o);
  const S = 'servers/alpha';

  // --- energia e console ---
  await same('', 'GET', '/api/properties', c);
  await same('', 'PUT', '/api/properties', { ...c, body: J({ 'max-players': 20, motd: 'paridade=ok', 'nova-chave': true, 2: 'x' }) });
  fileCmp('', S + '/server.properties');
  await same('', 'PUT', '/api/properties', { ...c, body: 'null', tag: 'corpo null' });
  await same('', 'PUT', '/api/properties', { ...c, body: '"texto"', tag: 'corpo string' });
  await same('', 'PUT', '/api/properties', { ...c, body: '["a","b"]', tag: 'corpo array' });
  await same('', 'PUT', '/api/properties?server=beta', { ...c, body: J({ 'view-distance': 6 }), tag: 'outra instância' });
  fileCmp('', 'servers/beta/server.properties');
  for (const q of ['', '?lines=3', '?lines=abc', '?lines=-2', '?lines=0', '?lines=5000', '?server=beta'])
    await same('', 'GET', '/api/logs' + q, c);
  await same('', 'POST', '/api/rcon', { ...c, body: J({}), tag: 'comando vazio' });
  await same('', 'POST', '/api/rcon', { ...c, body: 'null', tag: 'corpo null' });
  await same('', 'POST', '/api/rcon', { ...c, body: J({ command: 'list' }), tag: 'sem RCON rodando' });
  await same('', 'POST', '/api/power', { ...c, body: J({ action: 'explodir' }), tag: 'ação inválida' });
  await same('', 'POST', '/api/power', { ...c, body: J({}), tag: 'sem ação' });
  await same('', 'POST', '/api/power', { ...c, body: 'null', tag: 'corpo null' });
  await same('', 'POST', '/api/power', { ...c, body: J({ action: 'start' }) });
  await sleep(700);
  await same('', 'GET', '/api/status', { ...c, volatile: true, show: 60, tag: 'depois de ligar' });
  await same('', 'GET', '/api/servers/alpha/status', { ...c, volatile: true });
  await same('', 'POST', '/api/servers/alpha/restart', c);
  await sleep(500);
  await same('', 'GET', '/api/servers/alpha/status', { ...c, volatile: true, tag: 'depois de reiniciar' });
  await same('', 'POST', '/api/backups/restore', { ...c, body: J({ name: 'world-a.tar.gz' }), tag: 'servidor ligado → 409' });
  await same('', 'POST', '/api/servers/alpha/stop', c);
  await same('', 'GET', '/api/servers/alpha/status', { ...c, volatile: true, tag: 'depois de desligar' });
  await same('', 'POST', '/api/servers/beta/start', { ...c, tag: 'sem start.sh' });
  await same('', 'GET', '/api/servers/nada/status', c);
  await same('', 'GET', '/api/logs?lines=4', { ...c, tag: 'log do runner (sem latest.log)', cookieN: ln.cookie, cookieR: lr.cookie });

  // --- backups ---
  await same('', 'GET', '/api/backups', c);
  await same('', 'GET', '/api/backups/download?name=world-a.tar.gz', c);
  await same('', 'HEAD', '/api/backups/download?name=world-a.tar.gz', c);
  for (const n of ['', 'nada.tar.gz', '../../x.tar.gz', 'world-a.zip'])
    await same('', 'GET', '/api/backups/download?name=' + encodeURIComponent(n), { ...c, tag: n || 'vazio' });
  await same('', 'POST', '/api/backups/restore', { ...c, body: J({ name: '../world-a.tar.gz' }), tag: 'basename' });
  await same('', 'POST', '/api/backups/restore', { ...c, body: J({ name: 'nada.tar.gz' }) });
  await same('', 'POST', '/api/backups/restore', { ...c, body: J({ name: 'world-a.tar.gz' }) });
  fileCmp('restaurado', S + '/world/level.dat');
  await same('', 'POST', '/api/backups/restore', { ...c, body: J({ name: 'quebrado.tar.gz' }), tag: 'tar inválido' });
  await same('', 'POST', '/api/backups', c);
  const bl = await Promise.all([req(N, 'GET', '/api/backups', { cookie: ln.cookie }), req(R, 'GET', '/api/backups', { cookie: lr.cookie })]);
  const cnt = (r) => JSON.parse(r.body).backups.length;
  report(cnt(bl[0]) === cnt(bl[1]) && cnt(bl[1]) === 4, `POST /api/backups criou 1 backup nos dois (${cnt(bl[0])} × ${cnt(bl[1])})`);

  // --- conteúdo ---
  await same('', 'GET', '/api/content/info', c);
  await same('', 'GET', '/api/content/info?server=beta', c);
  await same('', 'GET', '/api/content/installed', c);
  await same('', 'POST', '/api/content/toggle', { ...c, body: J({ file: 'a.jar', enabled: false }) });
  fileCmp('', S + '/.craftbox-content.json', maskSecrets);
  existsCmp(S + '/plugins/a.jar.disabled');
  await same('', 'POST', '/api/content/toggle', { ...c, body: J({ file: 'b.jar.disabled', enabled: true }) });
  await same('', 'POST', '/api/content/toggle', { ...c, body: J({ file: '../x.jar', enabled: true }), tag: 'caminho inválido' });
  await same('', 'GET', '/api/content/installed', { ...c, tag: 'depois do toggle' });
  await same('', 'DELETE', '/api/content/installed?file=c.jar', c);
  await same('', 'DELETE', '/api/content/installed?file=nao-existe.jar', { ...c, tag: 'ENOENT' });
  await same('', 'DELETE', '/api/content/installed?file=..%2Fx', { ...c, tag: 'inválido' });
  await same('', 'DELETE', '/api/content/installed?slug=fake-slug-xyz', c);
  fileCmp('depois de remover', S + '/.craftbox-content.json', maskSecrets);
  await same('', 'GET', '/api/content/installed', { ...c, tag: 'final' });
  await same('', 'GET', '/api/content/project', { ...c, tag: 'sem slug' });
  await same('', 'GET', '/api/content/project?slug=fabric-api', { ...c, show: 80 });
  await same('', 'GET', '/api/content/versions?slug=essentialsx', { ...c, show: 80 });
  await same('', 'GET', '/api/content/search?q=worldedit&sort=downloads', { ...c, show: 80 });
  await same('', 'GET', '/api/content/updates', c);
  await same('', 'POST', '/api/compat/offline', { ...c, body: J({ enabled: true }) });
  fileCmp('offline', S + '/server.properties');
  await same('', 'POST', '/api/compat/offline', { ...c, body: J({ enabled: false }) });
  await same('', 'POST', '/api/content/install', { ...c, body: J({}), tag: 'sem slug' });
  await same('', 'POST', '/api/content/install', { ...c, body: J({ slug: 'nao-existe-craftbox-xyz' }), tag: 'projeto inexistente' });

  // --- servidores ---
  await same('', 'GET', '/api/servers', c);
  await same('', 'POST', '/api/servers/select', { ...c, body: J({ id: 'beta' }) });
  fileCmp('activeServer', 'config.json', maskSecrets);
  await same('', 'POST', '/api/servers/select', { ...c, body: J({ id: 'nada' }) });
  await same('', 'POST', '/api/servers/select', { ...c, body: J({ id: 'alpha' }) });
  await same('', 'POST', '/api/servers/create', { ...c, body: J({ name: '   ' }), tag: 'nome vazio' });
  await same('', 'POST', '/api/servers/create', { ...c, body: 'null', tag: 'corpo null' });
  await same('', 'POST', '/api/servers/clone', { ...c, body: J({ id: 'alpha', name: 'Cópia Ação' }) });
  for (const f of ['.craftbox-instance.json', 'server.properties']) fileCmp('clone', 'servers/copia-acao/' + f, maskSecrets);
  existsCmp('servers/copia-acao/logs');
  await same('', 'POST', '/api/servers/clone', { ...c, body: J({ id: 'alpha' }), tag: 'sem nome' });
  await same('', 'POST', '/api/servers/clone', { ...c, body: J({ id: '..' }), tag: 'origem inválida' });
  await same('', 'DELETE', '/api/servers?id=alpha-copia', c);
  await same('', 'DELETE', '/api/servers', { ...c, tag: 'sem id (B1)' });
  await same('', 'DELETE', '/api/servers?id=..', { ...c, tag: 'id .. (B1)' });
  existsCmp('servers/alpha');
  await same('', 'POST', '/api/servers/create', { ...c, body: J({ name: 'Fábrica Teste', loader: 'fabric', version: '1.21.1' }) });
  for (const f of ['.craftbox-instance.json', 'server.properties', 'start.sh', 'backup.sh', 'eula.txt', '.craftbox-loader']) fileCmp('fabric', 'servers/fabrica-teste/' + f, maskSecrets);
  const jar = (b) => { try { return fs.statSync(b + '/servers/fabrica-teste/server.jar').size; } catch { return -1; } };
  report(jar(E.NORM_NODE) > 0 && jar(E.NORM_NODE) === jar(E.NORM_RUST), `server.jar do Fabric baixado nos dois (${jar(E.NORM_NODE)} × ${jar(E.NORM_RUST)} bytes)`);
  await same('', 'GET', '/api/servers', { ...c, tag: 'depois de criar' });
  await same('', 'GET', '/api/worldsize', c);
  await same('', 'GET', '/api/worldsize?server=beta', c);

  // --- modpacks ---
  await same('', 'GET', '/api/modpacks/sources', c);
  await same('', 'GET', '/api/servers/create-progress', c);
  await same('', 'GET', '/api/modpacks/search?source=curseforge&q=atm', { ...c, tag: 'CurseForge sem chave' });
  await same('', 'GET', '/api/modpacks/versions?source=curseforge&slug=1', { ...c, tag: 'CurseForge sem chave' });
  await same('', 'POST', '/api/modpacks/curseforge-key', { ...c, body: J({ key: 'chave-falsa-123' }), tag: 'chave inválida' });
  await same('', 'POST', '/api/servers/create-modpack', { ...c, body: J({}), tag: 'sem slug' });
  await same('', 'GET', '/api/modpacks/search?q=cobblemon&loader=fabric', { ...c, show: 80 });
  await same('', 'GET', '/api/modpacks/versions', { ...c, tag: 'sem slug' });
  await same('', 'GET', '/api/modpacks/versions?slug=nao-existe-craftbox-xyz', c);

  // --- integrações e rede ---
  await same('', 'GET', '/api/integrations', c);
  await same('', 'POST', '/api/integrations/cloudflare-token', { ...c, body: J({ token: '  ' }), tag: 'vazio' });
  await same('', 'POST', '/api/integrations/cloudflare-token', { ...c, body: J({ token: ' tok123 ' }) });
  fileCmp('', 'integrations/alpha/cloudflared.token');
  await same('', 'POST', '/api/integrations/cloudflare-conf', { ...c, body: J({ onStart: true, hostname: ' mc.exemplo.com ' }) });
  fileCmp('', S + '/.craftbox-instance.json', maskSecrets);
  await same('', 'POST', '/api/integrations/playit-conf', { ...c, body: J({ onStart: 1 }) });
  await same('', 'POST', '/api/integrations/playit-secret', { ...c, body: J({ secret: 'xyz' }), tag: 'inválido' });
  await same('', 'POST', '/api/integrations/playit-secret', { ...c, body: J({ secret: ' 0123456789abcdef0123 ' }) });
  fileCmp('', 'integrations/alpha/playit.toml');
  await same('', 'GET', '/api/integrations', { ...c, tag: 'depois de configurar' });
  await same('', 'POST', '/api/integrations/start', { ...c, body: J({ name: 'playit' }), tag: 'sem binário' });
  await same('', 'POST', '/api/integrations/start', { ...c, body: J({ name: 'ftp' }), tag: 'desconhecida' });
  await same('', 'POST', '/api/integrations/stop', { ...c, body: J({ name: 'cloudflare' }) });
  await same('', 'POST', '/api/integrations/install', { ...c, body: J({ name: 'nada' }) });
  await same('', 'GET', '/api/integrations/log?name=x', c);
  await same('', 'GET', '/api/integrations/log?name=playit', c);
  await same('', 'GET', '/api/net', c);
  await same('', 'POST', '/api/net/wifi', { ...c, body: J({}), tag: 'sem rede' });

  // --- extras ---
  await same('', 'GET', '/api/extras', c);
  await same('', 'POST', '/api/extras', { ...c, body: J({ show: 0 }) });
  await same('', 'GET', '/api/extras', { ...c, tag: 'oculto' });

  // --- contas e histórico ---
  // ts difere entre os processos; ip/user diferem só nas entradas B5 (conferidas no auditCmp)
  const auditIgnore = /^items\.\d+\.(ts|ip|user)$/;
  await same('', 'GET', '/api/audit?lines=5', { ...c, ignore: auditIgnore, volatile: true });
  await same('', 'GET', '/api/audit?lines=abc', { ...c, ignore: auditIgnore, volatile: true, tag: 'NaN → tudo' });
  await same('', 'POST', '/api/change-password', { ...c, body: J({ current: 'x', newPassword: 'abc' }), tag: 'curta' });
  await same('', 'POST', '/api/change-password', { ...c, body: J({ current: 'errada', newPassword: 'nova-senha' }), tag: 'atual errada' });
  await same('', 'GET', '/api/users', c);
  await same('', 'POST', '/api/users', { ...c, body: J({ user: 'x', password: '1234' }), tag: 'nome curto' });
  await same('', 'POST', '/api/users', { ...c, body: J({ user: 'maria', password: '12' }), tag: 'senha curta' });
  await same('', 'POST', '/api/users', { ...c, body: J({ user: 'Maria', password: 'segredo1' }), tag: '1º usuário (vira admin + cookie)' });
  const mn = await login(N, { user: 'maria', password: 'segredo1' }), mr = await login(R, { user: 'maria', password: 'segredo1' });
  const m = { cookieN: mn.cookie, cookieR: mr.cookie };
  await same('', 'POST', '/api/users', { ...m, body: J({ user: 'MARIA', password: 'segredo1' }), tag: 'já existe' });
  await same('', 'POST', '/api/users', { ...m, body: J({ user: 'pedro.s', password: 'segredo2', role: 'user' }) });
  fileCmp('usuários', 'config.json', maskSecrets);
  await same('', 'GET', '/api/users', { ...m, tag: 'multiusuário' });
  const pn = await login(N, { user: 'pedro.s', password: 'segredo2' }), pr = await login(R, { user: 'pedro.s', password: 'segredo2' });
  await same('', 'POST', '/api/users', { cookieN: pn.cookie, cookieR: pr.cookie, body: J({ user: 'z', password: '1234' }), tag: 'não admin → 403' });
  await same('', 'POST', '/api/modpacks/curseforge-key', { cookieN: pn.cookie, cookieR: pr.cookie, body: J({ key: '' }), tag: 'não admin → 403' });
  await same('', 'POST', '/api/change-password', { cookieN: pn.cookie, cookieR: pr.cookie, body: J({ current: 'segredo2', newPassword: 'outra-senha' }), tag: 'multiusuário' });
  const pn2 = await login(N, { user: 'pedro.s', password: 'outra-senha' }), pr2 = await login(R, { user: 'pedro.s', password: 'outra-senha' });
  report(!!pn2.cookie && !!pr2.cookie, 'senha nova vale nos dois depois da troca');
  await same('', 'DELETE', '/api/users?user=maria', { ...m, tag: 'último admin' });
  await same('', 'DELETE', '/api/users?user=ninguem', { ...m, tag: 'não existe' });
  await same('', 'DELETE', '/api/users?user=PEDRO.S', m);
  fileCmp('depois de remover', 'config.json', maskSecrets);
  await same('', 'DELETE', '/api/audit', m);
  await same('', 'GET', '/api/audit', { ...m, ignore: auditIgnore, volatile: true, tag: 'depois de limpar' });
  auditCmp('D');
}

async function scenarioA() {
  console.log('\n== Cenário A: senha única (legado), runner systemd ==');
  for (const p of ['/', '/app.js', '/style.css', '/logo.png', '/favicon.png', '/logos/playit.svg', '/public/index.html', '/public//app.js', '/logos/nada.svg', '/nada', '/public/', '/public/../app.js', '/logos/%2e%2e/app.js'])
    await same('estático', 'GET', p);
  await same('', 'HEAD', '/');
  await sameRaw('HTTP/1.0 sem Host', 'GET /api/authcheck HTTP/1.0\r\n\r\n');
  await sameRaw('HTTP/1.0 estático (sem chunked, fecha)', 'GET /logos/playit.svg HTTP/1.0\r\n\r\n');
  await sameRaw('request line inválida', 'BLAH\r\n\r\n');
  await sameRaw('Connection: close', 'GET /nada HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n');
  await same('', 'POST', '/', { tag: 'estático aceita qualquer método' });
  await same('', 'GET', '/api/authcheck', { tag: 'sem sessão' });
  await same('', 'GET', '/api/status', { tag: 'sem sessão' });
  await same('', 'GET', '/api/login', { tag: 'GET não é login' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ password: 'errada' }), tag: 'senha errada' });
  await same('', 'POST', '/api/login', { body: 'isto não é json', tag: 'corpo inválido' });
  await same('', 'POST', '/api/login', { body: 'null', tag: 'corpo null' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ password: E.PASSWORD }), tag: 'senha certa' });
  const ln = await login(N, { password: E.PASSWORD }), lr = await login(R, { password: E.PASSWORD });
  report(!!ln.cookie && !!lr.cookie, 'login devolve cookie cbsession nos dois');
  await same('', 'GET', '/api/authcheck', { cookieN: ln.cookie, cookieR: lr.cookie, tag: 'com sessão' });
  // cookies trocados: o do Node vale no Rust e vice-versa (mesmo sessionSecret)
  await same('', 'GET', '/api/authcheck', { cookieN: lr.cookie, cookieR: ln.cookie, tag: 'cookie cruzado' });
  const cross = await Promise.all([req(N, 'GET', '/api/status', { cookie: lr.cookie }), req(R, 'GET', '/api/status', { cookie: ln.cookie })]);
  report(cross[0].status === 200 && cross[1].status === 200, 'cookie emitido por um backend autentica no outro (/api/status 200 nos dois)');
  // token adulterado e expirado
  const bad = ln.cookie.slice(0, -2) + (ln.cookie.endsWith('AA') ? 'BB' : 'AA');
  await same('', 'GET', '/api/status', { cookie: bad, tag: 'MAC adulterado' });
  const expVal = 's:1000:admin', expMac = crypto.createHmac('sha256', E.SECRET).update(expVal).digest('base64url');
  await same('', 'GET', '/api/status', { cookie: `cbsession=${expVal}.${expMac}`, tag: 'sessão expirada' });
  await same('', 'GET', '/api/status', { cookieN: ln.cookie, cookieR: lr.cookie, volatile: true, show: 400 });
  await same('', 'POST', '/api/status', { cookieN: ln.cookie, cookieR: lr.cookie, volatile: true, tag: 'qualquer método' });
  await same('', 'GET', '/api/diag/mojang', { cookieN: ln.cookie, cookieR: lr.cookie, volatile: true, show: 600 });
  await same('', 'POST', '/api/diag/mojang', { cookieN: ln.cookie, cookieR: lr.cookie, tag: 'só GET' });
  await same('', 'GET', '/api/nao-existe', { cookieN: ln.cookie, cookieR: lr.cookie });
  await same('', 'GET', '/api/properties', { cookieN: ln.cookie, cookieR: lr.cookie, tag: 'legado' });
  await same('', 'GET', '/api/servers', { cookieN: ln.cookie, cookieR: lr.cookie, tag: 'legado (multi desligado)' });
  await same('', 'POST', '/api/logout', { cookieN: ln.cookie, cookieR: lr.cookie });
  auditCmp('A');
}

async function scenarioB() {
  console.log('\n== Cenário B: multiusuário + multi-servidor, runner exec (PID files) ==');
  await same('', 'GET', '/api/authcheck', { tag: 'sem sessão, multiusuário' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ user: 'ninguem', password: 'x' }), tag: 'usuário inexistente' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ user: 'joao', password: 'errada' }), tag: 'senha errada' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ password: E.PASSWORD }), tag: 'sem usuário' });
  await same('', 'POST', '/api/login', { body: 'null', tag: 'corpo null' });
  await same('', 'POST', '/api/login', { body: JSON.stringify({ user: 'joao', password: E.PASSWORD }), tag: 'usuário comum (case-insensitive)' });
  const jn = await login(N, { user: 'joao', password: E.PASSWORD }), jr = await login(R, { user: 'joao', password: E.PASSWORD });
  const an = await login(N, { user: 'admin', password: E.PASSWORD }), ar = await login(R, { user: 'admin', password: E.PASSWORD });
  await same('', 'GET', '/api/authcheck', { cookieN: jn.cookie, cookieR: jr.cookie, tag: 'Joao (não admin)' });
  await same('', 'GET', '/api/authcheck', { cookieN: an.cookie, cookieR: ar.cookie, tag: 'admin' });
  await same('', 'GET', '/api/authcheck', { cookieN: ar.cookie, cookieR: an.cookie, tag: 'cookie cruzado' });
  for (const q of ['', '?server=alpha', '?server=gamma', '?server=beta', '?server=nao-existe'])
    await same('', 'GET', '/api/status' + q, { cookieN: an.cookie, cookieR: ar.cookie, volatile: true, show: 160 });
  for (const q of ['?server=alpha', '?server=beta', '?server=gamma'])
    await same('', 'GET', '/api/diag/mojang' + q, { cookieN: an.cookie, cookieR: ar.cookie, volatile: true, show: 60 });
  await same('', 'GET', '/api/servers/alpha/status', { cookieN: an.cookie, cookieR: ar.cookie, volatile: true });
  await same('', 'GET', '/api/servers', { cookieN: an.cookie, cookieR: ar.cookie });
  auditCmp('B');
}

// Bug do Node documentado no ROUTES.md: cookie com '%' malformado lança URIError
// fora do try e derruba o processo. Roda por último (o Node morre aqui).
async function malformedCookie() {
  console.log('\n== Divergência documentada: cookie malformado ==');
  const b = await req(R, 'GET', '/api/status', { cookie: 'cbsession=%E0%A4%A' });
  report(b.status === 401, `Rust responde ${b.status} {"error":"nao autenticado"} e segue no ar`);
  const a = await req(N, 'GET', '/api/status', { cookie: 'cbsession=%E0%A4%A' });
  await new Promise((r) => setTimeout(r, 300));
  let alive = true; try { process.kill(Number(E.NODE_PID), 0); } catch { alive = false; }
  report('note', `Node: ${a.status ? 'status ' + a.status : 'conexão caiu (' + a.error + ')'}; processo ${alive ? 'continua vivo' : 'MORREU (URIError não tratado)'}`);
}

(async () => {
  if (SC === 'A') await scenarioA();
  if (SC === 'B') await scenarioB();
  if (SC === 'COOKIE') await malformedCookie();
  if (SC === 'D') await scenarioD();
  console.log(`  -- ${SC}: ${oks} ok, ${fails} falha(s), ${notes} nota(s)`);
  fs.appendFileSync(E.SUMMARY, `${SC} ${oks} ${fails} ${notes}\n`);
  process.exit(fails ? 1 : 0);
})();
