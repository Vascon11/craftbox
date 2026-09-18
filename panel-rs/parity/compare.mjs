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
];
const kind = (v) => (v === null ? 'null' : Array.isArray(v) ? 'array' : typeof v);

function deepCmp(a, b, path = '', out = []) {
  const rule = (VOLATILE.find(([re]) => re.test(path)) || [])[1];
  if (rule && rule.type === 'any') return out;
  if (kind(a) !== kind(b)) { out.push(`${path || '(raiz)'}: tipo ${kind(a)} × ${kind(b)} (${JSON.stringify(a)} × ${JSON.stringify(b)})`); return out; }
  if (kind(a) === 'object') {
    const ka = Object.keys(a), kb = Object.keys(b);
    if (ka.join(',') !== kb.join(',')) out.push(`${path || '(raiz)'}: chaves [${ka}] × [${kb}]`);
    for (const k of ka) if (k in b) deepCmp(a[k], b[k], path ? `${path}.${k}` : k, out);
  } else if (kind(a) === 'array') {
    if (a.length !== b.length) out.push(`${path}: tamanho ${a.length} × ${b.length}`);
    a.forEach((x, i) => i < b.length && deepCmp(x, b[i], `${path}.${i}`, out));
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
  const diffs = [];
  if (a.status !== b.status) diffs.push(`status ${a.status} × ${b.status}`);
  for (const h of FRAMING) if (a.headers[h] !== b.headers[h]) diffs.push(`header ${h}: ${a.headers[h]} × ${b.headers[h]}`);
  const na = a.names.map((x) => x.toLowerCase()).filter((x) => x !== 'date').join(',');
  const nb = b.names.map((x) => x.toLowerCase()).filter((x) => x !== 'date').join(',');
  if (na !== nb) diffs.push(`ordem/nomes de headers [${na}] × [${nb}]`);
  const ca = cookieShape(a.headers['set-cookie']).join('|'), cb = cookieShape(b.headers['set-cookie']).join('|');
  if (ca !== cb) diffs.push(`set-cookie ${ca} × ${cb}`);
  let summary;
  if ((a.headers['content-type'] || '').startsWith('application/json')) {
    let ja, jb;
    try { ja = JSON.parse(a.body); jb = JSON.parse(b.body); } catch (e) { diffs.push('JSON inválido: ' + e.message); }
    if (ja !== undefined && jb !== undefined) {
      diffs.push(...deepCmp(ja, jb));
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

function auditCmp(label) {
  const read = (p) => { try { return fs.readFileSync(p, 'utf8').trim().split('\n').filter(Boolean).map((l) => JSON.parse(l)); } catch { return []; } };
  const a = read(E.AUDIT_NODE), b = read(E.AUDIT_RUST);
  const norm = (e) => JSON.stringify([Object.keys(e), e.ip, e.user, e.server, e.action, e.detail, typeof e.ts]);
  const diffs = [];
  if (a.length !== b.length) diffs.push(`${a.length} × ${b.length} entradas`);
  a.forEach((e, i) => { if (b[i] && norm(e) !== norm(b[i])) diffs.push(`#${i}: ${norm(e)} × ${norm(b[i])}`); });
  report(diffs.length === 0 && a.length > 0, `${label}: log de auditoria (${a.length} entradas)`, diffs.join('; ') || a.map((e) => e.action).join(', '));
}

async function pendingNote(method, path, cookieN, cookieR) {
  const [a, b] = await Promise.all([req(N, method, path, { cookie: cookieN }), req(R, method, path, { cookie: cookieR })]);
  report('note', `${method} ${path}: Node ${a.status}, Rust ${b.status} (rota pendente — 501 é o esperado nesta fase)`);
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
  await pendingNote('GET', '/api/properties', ln.cookie, lr.cookie);
  await pendingNote('GET', '/api/servers', ln.cookie, lr.cookie);
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
  await pendingNote('GET', '/api/servers/alpha/status', an.cookie, ar.cookie);
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
  console.log(`  -- ${SC}: ${oks} ok, ${fails} falha(s), ${notes} nota(s)`);
  fs.appendFileSync(E.SUMMARY, `${SC} ${oks} ${fails} ${notes}\n`);
  process.exit(fails ? 1 : 0);
})();
