'use strict';
const $ = (s) => document.querySelector(s);
let currentServer = '';
const api = async (path, opts = {}) => {
  let url = path;
  const skip = path === '/api/login' || path === '/api/logout' || path === '/api/authcheck' || path.startsWith('/api/servers');
  if (currentServer && path.startsWith('/api/') && !skip) {
    url += (path.includes('?') ? '&' : '?') + 'server=' + encodeURIComponent(currentServer);
  }
  const r = await fetch(url, { headers: { 'Content-Type': 'application/json' }, ...opts });
  let data = {}; try { data = await r.json(); } catch {}
  return { ok: r.ok, status: r.status, data };
};
const fmtDur = (s) => { if (s == null) return '—'; const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60); return h ? `${h}h ${m}min` : `${m}min`; };

// ---- auth / boot ----
async function boot() {
  const { data } = await api('/api/authcheck');
  if (data.authed) showApp();
  else { $('#login').hidden = false; if (!data.configured) { $('#loginErr').hidden = false; $('#loginErr').textContent = 'Senha ainda não configurada no servidor (node server.js --hash).'; } }
}
$('#loginForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const { ok, data } = await api('/api/login', { method: 'POST', body: JSON.stringify({ password: $('#pw').value }) });
  if (ok) { $('#login').hidden = true; showApp(); }
  else { $('#loginErr').hidden = false; $('#loginErr').textContent = data.error || 'Falha no login'; }
});
$('#logout').addEventListener('click', async () => { await api('/api/logout', { method: 'POST' }); location.reload(); });

async function showApp() { $('#app').hidden = false; await loadServers(); reloadAll(); }
function reloadAll() { loadProps(); startStatus(); startLogs(); loadBackups(); loadContentInfo(); loadInstalled(); loadIntegrations(); }

// ---- multi-servidor ----
async function loadServers() {
  const { ok, data } = await api('/api/servers');
  const wrap = $('#serverBar');
  if (!ok || !data.multi) { if (wrap) wrap.hidden = true; currentServer = ''; return; }
  wrap.hidden = false;
  currentServer = data.activeId || (data.servers[0] && data.servers[0].id) || '';
  const sel = $('#serverSelect'); sel.innerHTML = '';
  data.servers.forEach(s => {
    const label = `${s.name} · ${s.loader}${s.mcVersion ? ' ' + s.mcVersion : ''}${s.active === 'active' ? ' 🟢' : ''}`;
    const o = new Option(label, s.id); sel.add(o);
  });
  sel.value = currentServer;
}
async function switchServer(id) {
  await api('/api/servers/select', { method: 'POST', body: JSON.stringify({ id }) });
  currentServer = id;
  reloadAll();
}
async function renderServerManager() {
  const { data } = await api('/api/servers');
  const box = $('#srvList'); box.innerHTML = '';
  (data.servers || []).forEach(s => {
    const el = document.createElement('div'); el.className = 'inst-row';
    const mp = s.modpack;
    el.innerHTML = `
      <div class="inst-meta">
        <div class="inst-title">${esc(s.name)} ${s.active === 'active' ? '<span class="tag">no ar</span>' : ''} ${s.selected ? '<span class="tag">selecionado</span>' : ''}</div>
        <div class="muted small">${esc(s.loader)}${s.mcVersion ? ' ' + esc(s.mcVersion) : ''} · porta ${s.port || '?'} · id <code>${esc(s.id)}</code></div>
        ${mp ? `<div class="muted small" style="margin-top:.25rem">📦 modpack: <a href="${esc(mp.url)}" target="_blank" rel="noopener">${esc(mp.name)}</a> v${esc(mp.version)} — <b>jogadores instalam este pack no cliente</b> <button class="ghost sm btn-copy" data-url="${esc(mp.url)}">copiar link</button></div>` : ''}
      </div>
      <div class="inst-actions">
        <button class="ghost sm btn-clone">Clonar</button>
        <button class="danger sm btn-del">Apagar</button>
      </div>`;
    const cp = el.querySelector('.btn-copy');
    if (cp) cp.onclick = () => { navigator.clipboard && navigator.clipboard.writeText(cp.dataset.url); toast('Link do pack copiado — manda pros jogadores.'); };
    el.querySelector('.btn-clone').onclick = async () => {
      const nome = prompt('Nome da cópia:', s.name + ' (cópia)'); if (!nome) return;
      const r = await api('/api/servers/clone', { method: 'POST', body: JSON.stringify({ id: s.id, name: nome }) });
      if (!r.ok) return alert(r.data.error || 'falha ao clonar');
      toast('Servidor clonado.'); await loadServers(); renderServerManager();
    };
    el.querySelector('.btn-del').onclick = async () => {
      if (!confirm(`Apagar o servidor "${s.name}"? Isso remove o mundo e tudo dele. Não dá pra desfazer.`)) return;
      const r = await api('/api/servers?id=' + encodeURIComponent(s.id), { method: 'DELETE' });
      if (!r.ok) return alert(r.data.error || 'falha ao apagar');
      toast('Servidor apagado.'); await loadServers(); reloadAll(); renderServerManager();
    };
    box.append(el);
  });
}

// ---- tabs ----
document.querySelectorAll('.tab').forEach(t => t.addEventListener('click', () => {
  document.querySelectorAll('.tab').forEach(x => x.classList.remove('active'));
  t.classList.add('active');
  document.querySelectorAll('.panel').forEach(p => p.hidden = true);
  $('#' + t.dataset.tab).hidden = false;
}));

// ---- status ----
function pct(a, b) { return b ? Math.min(100, Math.round(a / b * 100)) : 0; }
async function refreshStatus() {
  const { ok, data } = await api('/api/status');
  if (!ok) return;
  const active = data.active;
  const dot = $('#dot'), pillText = $('#pillText');
  dot.className = 'dot ' + (active === 'active' ? 'on' : active === 'activating' ? 'act' : 'idle');
  pillText.textContent = active === 'active' ? 'no ar' : active === 'activating' ? 'iniciando…' : 'desligado';
  $('#svcState').textContent = active === 'active' ? 'No ar' : active === 'activating' ? 'Iniciando…' : 'Desligado';
  $('#svcMeta').textContent = active === 'active'
    ? `no ar há ${fmtDur(data.uptime)}${data.players ? ` · ${data.players.online} jogador(es)` : ' · sem jogadores'}`
    : (active === 'activating' ? 'subindo o servidor…' : 'servidor desligado — clique em Ligar pra iniciar');
  $('#players').textContent = data.players ? `${data.players.online} / ${data.players.max}` : (active === 'active' ? '0 / ?' : '—');

  const s = data.system || {};
  if (s.cpu) {
    $('#cpuFreq').textContent = s.cpu.curMHz ?? '—';
    $('#cpuMeta').textContent = `máx ${s.cpu.maxMHz ?? '?'} MHz · ${s.cpus} threads · load ${s.load ? s.load[0] : '—'}`;
    const warn = $('#cpuWarn');
    if (s.cpu.curMHz && s.cpu.maxMHz && s.cpu.curMHz < s.cpu.maxMHz * 0.45) {
      warn.hidden = false; warn.textContent = '⚠ CPU parece presa perto do mínimo (verifique carregador/bateria).';
    } else warn.hidden = true;
  }
  $('#temp').textContent = s.tempC ?? '—';
  $('#loadMeta').textContent = s.load ? `load ${s.load.join(' / ')}` : '—';
  if (s.mem) {
    $('#memUsed').textContent = s.mem.used;
    $('#memMeta').textContent = `de ${s.mem.total} MB (${pct(s.mem.used, s.mem.total)}%)`;
    $('#memBar').style.width = pct(s.mem.used, s.mem.total) + '%';
    $('#memBar').classList.toggle('hot', pct(s.mem.used, s.mem.total) > 88);
  }
  if (s.disk) {
    $('#diskUsed').textContent = s.disk.usedGB;
    $('#diskMeta').textContent = `de ${s.disk.totalGB} GB (${pct(s.disk.usedGB, s.disk.totalGB)}%)`;
    $('#diskBar').style.width = pct(s.disk.usedGB, s.disk.totalGB) + '%';
  }
}
let statusTimer;
function startStatus() { refreshStatus(); clearInterval(statusTimer); statusTimer = setInterval(refreshStatus, 3000); }

// ---- power ----
async function power(action, btn) {
  const msg = $('#powerMsg');
  const labels = { start: 'ligando', restart: 'reiniciando', stop: 'desligando' };
  document.querySelectorAll('#painel button').forEach(b => b.disabled = true);
  msg.textContent = (labels[action] || 'executando') + '…';
  const { ok, data } = await api('/api/power', { method: 'POST', body: JSON.stringify({ action }) });
  msg.textContent = ok
    ? `✓ comando de ${labels[action] || action} enviado — acompanhe o estado acima`
    : ('erro: ' + (data.output || data.error || '')).slice(0, 200);
  setTimeout(() => { document.querySelectorAll('#painel button').forEach(b => b.disabled = false); refreshStatus(); }, 1500);
  setTimeout(() => { const m = $('#powerMsg'); if (m) m.textContent = ''; }, 6000);
}
$('#btnStart').addEventListener('click', () => power('start'));
$('#btnRestart').addEventListener('click', () => power('restart'));
$('#btnStop').addEventListener('click', () => { if (confirm('Desligar o servidor? Jogadores serão desconectados.')) power('stop'); });

// ---- config (server.properties) ----
const COMMON = [
  ['motd', 'Mensagem (MOTD)', 'text'],
  ['max-players', 'Máx. jogadores', 'number'],
  ['difficulty', 'Dificuldade', 'select', ['peaceful', 'easy', 'normal', 'hard']],
  ['gamemode', 'Modo de jogo', 'select', ['survival', 'creative', 'adventure', 'spectator']],
  ['pvp', 'PvP', 'bool'],
  ['online-mode', 'Modo online (autenticação)', 'bool'],
  ['view-distance', 'Distância de visão', 'number'],
  ['simulation-distance', 'Distância de simulação', 'number'],
  ['white-list', 'Whitelist', 'bool'],
  ['hardcore', 'Hardcore', 'bool'],
  ['spawn-protection', 'Proteção de spawn', 'number'],
  ['level-name', 'Nome do mundo', 'text'],
  ['level-seed', 'Seed', 'text'],
  ['allow-nether', 'Permitir Nether', 'bool'],
  ['spawn-monsters', 'Gerar monstros', 'bool'],
  ['enable-command-block', 'Command blocks', 'bool'],
];
let propsCache = {};
async function loadProps() {
  const { data } = await api('/api/properties');
  propsCache = data.properties || {};
  const form = $('#propsForm'); form.innerHTML = '';
  const shown = new Set();
  const field = (key, label, type, opts) => {
    const val = propsCache[key] ?? '';
    const d = document.createElement('div'); d.className = 'prop';
    let ctrl;
    if (type === 'bool') {
      ctrl = document.createElement('select');
      ['true', 'false'].forEach(o => { const op = new Option(o === 'true' ? 'sim' : 'não', o); ctrl.add(op); });
      ctrl.value = (val === 'true') ? 'true' : 'false';
    } else if (type === 'select') {
      ctrl = document.createElement('select');
      opts.forEach(o => ctrl.add(new Option(o, o)));
      ctrl.value = val;
    } else {
      ctrl = document.createElement('input'); ctrl.type = type === 'number' ? 'number' : 'text'; ctrl.value = val;
    }
    ctrl.dataset.key = key;
    d.append(Object.assign(document.createElement('label'), { textContent: label }), ctrl);
    form.append(d); shown.add(key);
  };
  COMMON.forEach(([k, l, t, o]) => { if (k in propsCache || t === 'bool' || t === 'select') field(k, l, t, o); });
  // demais chaves como texto
  Object.keys(propsCache).sort().forEach(k => { if (!shown.has(k)) field(k, k, 'text'); });
}
$('#cfgReload').addEventListener('click', loadProps);
$('#cfgSave').addEventListener('click', async () => {
  const updates = {};
  $('#propsForm').querySelectorAll('[data-key]').forEach(el => { updates[el.dataset.key] = String(el.value); });
  const { ok } = await api('/api/properties', { method: 'PUT', body: JSON.stringify(updates) });
  $('#cfgMsg').textContent = ok ? '✓ salvo — reinicie o servidor pra aplicar.' : 'erro ao salvar';
  setTimeout(() => $('#cfgMsg').textContent = '', 4000);
});

// ---- console (log ao vivo + comandos RCON, mesma tela) ----
let logBuf = '';   // log do servidor (atualiza sozinho)
let cmdBuf = '';   // comandos digitados + respostas (persistem entre refreshes)
function renderConsole() {
  const el = $('#consoleOut'); if (!el) return;
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  const parts = [];
  if (logBuf) parts.push(logBuf);
  if (cmdBuf) parts.push('──── comandos ────\n' + cmdBuf);
  el.textContent = parts.join('\n') || '(sem logs)';
  if (atBottom) el.scrollTop = el.scrollHeight;
}
$('#rconForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const cmd = $('#rconIn').value.trim(); if (!cmd) return;
  $('#rconIn').value = '';
  cmdBuf += `> ${cmd}\n`; renderConsole();
  const { ok, data } = await api('/api/rcon', { method: 'POST', body: JSON.stringify({ command: cmd }) });
  const resp = ok ? (data.response || '(ok)') : ('erro: ' + (data.error || ''));
  cmdBuf += resp.trim() + '\n'; renderConsole();
  const el = $('#consoleOut'); if (el) el.scrollTop = el.scrollHeight;
});

let logTimer;
async function refreshLogs() {
  if (!$('#autolog').checked) return;
  const { ok, data } = await api('/api/logs?lines=250');
  if (ok) { logBuf = data.log || ''; renderConsole(); }
}
function startLogs() { refreshLogs(); clearInterval(logTimer); logTimer = setInterval(refreshLogs, 3000); }

// ---- backups ----
async function loadBackups() {
  const { data } = await api('/api/backups');
  const body = $('#bkBody'); body.innerHTML = '';
  (data.backups || []).forEach(b => {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${b.name}</td><td>${b.sizeMB} MB</td><td>${new Date(b.mtime).toLocaleString('pt-BR')}</td>`;
    body.append(tr);
  });
  if (!(data.backups || []).length) body.innerHTML = '<tr><td colspan="3" class="muted">Nenhum backup ainda.</td></tr>';
}
$('#bkNow').addEventListener('click', async () => {
  $('#bkMsg').textContent = 'fazendo backup…'; $('#bkNow').disabled = true;
  const { ok } = await api('/api/backups', { method: 'POST' });
  $('#bkMsg').textContent = ok ? '✓ backup criado' : 'erro ao fazer backup';
  $('#bkNow').disabled = false; loadBackups();
  setTimeout(() => $('#bkMsg').textContent = '', 4000);
});

// ---- conteudo (mods/plugins) — loja estilo Prism/Modrinth ----
const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fmtNum = (n) => { n = n || 0; if (n >= 1e6) return (n / 1e6).toFixed(1).replace('.0', '') + 'M'; if (n >= 1e3) return (n / 1e3).toFixed(1).replace('.0', '') + 'k'; return '' + n; };
const verLabel = (v) => { v = String(v || ''); return /^v\d/i.test(v) ? v : 'v' + v; };
const CATS = [['optimization', 'Otimização'], ['utility', 'Utilidade'], ['management', 'Administração'], ['adventure', 'Aventura'], ['worldgen', 'Mundo'], ['magic', 'Magia'], ['technology', 'Tecnologia'], ['mobs', 'Mobs'], ['economy', 'Economia'], ['social', 'Social'], ['storage', 'Armazenamento'], ['library', 'Biblioteca'], ['minigame', 'Minigame'], ['game-mechanics', 'Mecânicas']];
const CAT_MAP = Object.fromEntries(CATS);
const catLabel = (c) => CAT_MAP[c] || c;
let activeCat = '';
let updatesMap = {};

function iconHtml(url, title) {
  if (url) return `<img class="c-icon" src="${esc(url)}" alt="" loading="lazy">`;
  return `<div class="c-icon ph">${esc((title || '?').trim().charAt(0).toUpperCase())}</div>`;
}
function toast(msg) {
  let t = $('#toast'); if (!t) { t = document.createElement('div'); t.id = 'toast'; document.body.append(t); }
  t.textContent = msg; t.classList.add('show'); clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove('show'), 5000);
}

async function loadContentInfo() {
  const { data } = await api('/api/content/info');
  $('#contentInfo').textContent = `${data.loader || '?'} · ${data.kind || ''} · MC ${data.mcVersion || '?'}`;
  $('#offlineToggle').checked = data.onlineMode === false;
  renderChips();
  doSearch();
}
function renderChips() {
  const box = $('#catChips'); if (!box || box.dataset.done) return;
  box.dataset.done = '1';
  const mk = (cat, label, active) => { const c = document.createElement('button'); c.type = 'button'; c.className = 'chip' + (active ? ' active' : ''); c.textContent = label; c.dataset.cat = cat; return c; };
  box.append(mk('', 'Todos', true));
  CATS.forEach(([slug, label]) => box.append(mk(slug, label, false)));
  box.querySelectorAll('.chip').forEach(ch => ch.onclick = () => {
    box.querySelectorAll('.chip').forEach(x => x.classList.remove('active'));
    ch.classList.add('active'); activeCat = ch.dataset.cat; doSearch();
  });
}

async function doSearch() {
  const q = $('#searchIn').value.trim(), sort = $('#sortSelect').value;
  const box = $('#searchResults'); box.innerHTML = '<div class="muted small" style="padding:1rem">buscando…</div>';
  const qs = `q=${encodeURIComponent(q)}&sort=${encodeURIComponent(sort)}&category=${encodeURIComponent(activeCat)}`;
  const { ok, data } = await api('/api/content/search?' + qs);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; return; }
  box.innerHTML = ''; data.results.forEach(r => box.append(storeCard(r)));
}
function storeCard(r) {
  const el = document.createElement('div'); el.className = 'store-card';
  const tags = (r.categories || []).slice(0, 3).map(c => `<span class="tag">${esc(catLabel(c))}</span>`).join('');
  el.innerHTML = `
    <div class="store-top">
      ${iconHtml(r.icon, r.title)}
      <div class="store-meta">
        <div class="store-title">${esc(r.title)}</div>
        <div class="muted small">${r.author ? 'por ' + esc(r.author) : ''}</div>
      </div>
    </div>
    <div class="store-desc muted small">${esc((r.description || '').slice(0, 150))}</div>
    <div class="store-tags">${tags}</div>
    <div class="store-foot">
      <span class="muted small">⬇ ${fmtNum(r.downloads)}</span>
      <span class="grow"></span>
      <button class="ghost sm btn-detail">Versões</button>
      <button class="ok sm btn-install">Instalar</button>
    </div>`;
  el.querySelector('.btn-install').onclick = (ev) => installSlug(r.slug, null, ev.currentTarget);
  el.querySelector('.btn-detail').onclick = () => openModal(r.slug, r.title);
  return el;
}

async function installSlug(slug, versionId, btn) {
  const old = btn ? btn.textContent : '';
  if (btn) { btn.disabled = true; btn.textContent = 'instalando…'; }
  const { ok, data } = await api('/api/content/install', { method: 'POST', body: JSON.stringify({ slug, versionId }) });
  if (btn) {
    if (ok) { btn.textContent = '✓ instalado'; }
    else { btn.disabled = false; btn.textContent = old; alert(data.error || 'falha ao instalar'); }
  }
  if (ok) {
    const deps = (data.installed || []).filter(x => x.dep).map(x => x.title);
    toast(deps.length ? `Instalado + ${deps.length} dependência(s): ${deps.join(', ')} — reinicie o servidor.` : 'Instalado — reinicie o servidor pra aplicar.');
    loadInstalled();
  }
  return ok;
}

// ---- modal de detalhes / seleção de versão ----
async function openModal(slug, title) {
  const ov = $('#modOverlay'), body = $('#modBody');
  ov.hidden = false;
  body.innerHTML = `<div class="muted" style="padding:2.5rem;text-align:center">carregando ${esc(title || slug)}…</div>`;
  const [pj, vs] = await Promise.all([
    api('/api/content/project?slug=' + encodeURIComponent(slug)),
    api('/api/content/versions?slug=' + encodeURIComponent(slug)),
  ]);
  if (!pj.ok) { body.innerHTML = `<div class="err" style="padding:1.5rem">${esc(pj.data.error || 'erro ao carregar')}</div>`; return; }
  const p = pj.data.project, versions = (vs.data && vs.data.versions) || [], mc = vs.data && vs.data.mcVersion;
  const gallery = (p.gallery || []).slice(0, 4).map(u => `<img src="${esc(u)}" class="gal" loading="lazy">`).join('');
  const opts = versions.map(v => `<option value="${esc(v.id)}">${esc(v.versionNumber)} · MC ${(v.gameVersions || []).slice(0, 3).join(', ')}${v.compatible ? ' ✓' : ''} · ${esc(v.versionType)}</option>`).join('');
  body.innerHTML = `
    <div class="mod-head">
      ${iconHtml(p.icon, p.title)}
      <div style="min-width:0">
        <div class="mod-title">${esc(p.title)}</div>
        <div class="muted small">⬇ ${fmtNum(p.downloads)} · ♥ ${fmtNum(p.follows)}${p.source ? ` · <a href="${esc(p.source)}" target="_blank" rel="noopener">código-fonte</a>` : ''}</div>
        <div class="store-tags" style="margin-top:.4rem">${(p.categories || []).map(c => `<span class="tag">${esc(catLabel(c))}</span>`).join('')}</div>
      </div>
    </div>
    <p class="muted small">${esc(p.description || '')}</p>
    ${gallery ? `<div class="gallery">${gallery}</div>` : ''}
    <div class="ver-row">
      <label class="muted small" style="flex:1;display:flex;flex-direction:column;gap:.3rem">
        <span>Versão pra instalar${mc ? ` (servidor roda MC ${esc(mc)})` : ''}</span>
        <select id="verSel">${opts || '<option value="">— sem versões compatíveis —</option>'}</select>
      </label>
      <button id="verInstall" class="ok"${versions.length ? '' : ' disabled'}>Instalar</button>
    </div>
    <div id="modMsg" class="muted small"></div>`;
  const sel = $('#verSel');
  const firstCompat = versions.find(v => v.compatible); if (firstCompat && sel) sel.value = firstCompat.id;
  const vi = $('#verInstall');
  if (vi) vi.onclick = async (ev) => { const ok = await installSlug(slug, sel.value, ev.currentTarget); if (ok) $('#modMsg').textContent = '✓ instalado'; };
}

// ---- instalados ----
async function loadInstalled() {
  const { data } = await api('/api/content/installed');
  const box = $('#installedList'); box.innerHTML = '';
  const items = (data && data.items) || [];
  if (!items.length) { box.innerHTML = '<div class="muted small">nenhum mod/plugin instalado ainda.</div>'; return; }
  items.forEach(it => {
    const upd = it.slug && updatesMap[it.slug];
    const el = document.createElement('div'); el.className = 'inst-row' + (it.disabled ? ' off' : '');
    el.innerHTML = `
      ${iconHtml(it.icon, it.title)}
      <div class="inst-meta">
        <div class="inst-title">${esc(it.title)} ${it.version ? `<span class="muted small">${esc(verLabel(it.version))}</span>` : ''}${it.managed ? '' : ' <span class="tag warn">externo</span>'}${it.disabled ? ' <span class="tag">desativado</span>' : ''}</div>
        <div class="muted small">${esc(it.file)} · ${it.sizeMB} MB</div>
      </div>
      <div class="inst-actions">
        ${upd ? `<button class="ok sm btn-upd">Atualizar → ${esc(verLabel(upd.latest))}</button>` : ''}
        <label class="tgl" title="Ativar / desativar"><input type="checkbox" class="tgl-in" ${it.disabled ? '' : 'checked'}><span class="tgl-track"></span></label>
        <button class="danger sm btn-rm">Remover</button>
      </div>`;
    el.querySelector('.tgl-in').onchange = async (ev) => {
      await api('/api/content/toggle', { method: 'POST', body: JSON.stringify({ file: it.filename, enabled: ev.target.checked }) });
      toast('Alterado — reinicie o servidor pra aplicar.'); loadInstalled();
    };
    el.querySelector('.btn-rm').onclick = async () => {
      if (!confirm('Remover ' + it.title + '?')) return;
      const qs = it.slug ? ('slug=' + encodeURIComponent(it.slug)) : ('file=' + encodeURIComponent(it.file));
      await api('/api/content/installed?' + qs, { method: 'DELETE' }); loadInstalled();
    };
    const ub = el.querySelector('.btn-upd');
    if (ub) ub.onclick = async (ev) => { const ok = await installSlug(it.slug, upd.versionId, ev.currentTarget); if (ok) { delete updatesMap[it.slug]; loadInstalled(); } };
    box.append(el);
  });
}
$('#searchForm').addEventListener('submit', (e) => { e.preventDefault(); doSearch(); });
$('#sortSelect').addEventListener('change', doSearch);
$('#reloadInstalled').addEventListener('click', loadInstalled);
$('#modClose').addEventListener('click', () => $('#modOverlay').hidden = true);
$('#modOverlay').addEventListener('click', (e) => { if (e.target === $('#modOverlay')) $('#modOverlay').hidden = true; });
$('#updateCheck').addEventListener('click', async () => {
  const b = $('#updateCheck'), old = b.textContent; b.disabled = true; b.textContent = 'verificando…'; $('#updMsg').textContent = '';
  const { ok, data } = await api('/api/content/updates');
  b.disabled = false; b.textContent = old;
  if (!ok) { $('#updMsg').textContent = data.error || 'erro'; return; }
  updatesMap = {}; (data.updates || []).forEach(u => updatesMap[u.slug] = u);
  const n = (data.updates || []).length;
  $('#updMsg').textContent = n ? `${n} atualização(ões) disponível(is) — botão "Atualizar" nos itens abaixo.` : 'Tudo atualizado. ✓';
  loadInstalled();
});

// ---- compatibilidade ----
$('#offlineToggle').addEventListener('change', async (e) => {
  const { ok, data } = await api('/api/compat/offline', { method: 'POST', body: JSON.stringify({ enabled: e.target.checked }) });
  $('#offlineMsg').textContent = ok ? `✓ modo offline ${e.target.checked ? 'ATIVADO' : 'desativado'} — reinicie o servidor.` : (data.error || 'erro');
  setTimeout(() => $('#offlineMsg').textContent = '', 5000);
});
$('#bedrockBtn').addEventListener('click', async () => {
  const btn = $('#bedrockBtn'), msg = $('#bedrockMsg');
  btn.disabled = true; msg.textContent = 'baixando Geyser + Floodgate…';
  const { ok, data } = await api('/api/compat/bedrock', { method: 'POST' });
  msg.textContent = ok ? `✓ instalado: ${data.installed.join(', ')} — reinicie o servidor.` : ('erro: ' + (data.error || ''));
  btn.disabled = false; loadInstalled();
});

// ---- bindings multi-servidor ----
$('#serverSelect').addEventListener('change', (e) => switchServer(e.target.value));
$('#serverManage').addEventListener('click', () => { $('#srvOverlay').hidden = false; renderServerManager(); });
$('#srvClose').addEventListener('click', () => $('#srvOverlay').hidden = true);
$('#srvOverlay').addEventListener('click', (e) => { if (e.target === $('#srvOverlay')) $('#srvOverlay').hidden = true; });
$('#srvCreateForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const name = $('#srvName').value.trim(); if (!name) return;
  const loader = $('#srvLoader').value, version = $('#srvVersion').value.trim();
  const btn = $('#srvCreateBtn'); btn.disabled = true; btn.textContent = 'criando… (baixando o servidor)';
  const r = await api('/api/servers/create', { method: 'POST', body: JSON.stringify({ name, loader, version }) });
  btn.disabled = false; btn.textContent = 'Criar servidor';
  if (!r.ok) { alert(r.data.error || 'falha ao criar'); return; }
  $('#srvName').value = ''; $('#srvVersion').value = '';
  toast('Servidor criado: ' + r.data.server.name + ' (porta ' + r.data.server.port + ')');
  await loadServers(); renderServerManager();
});

// ---- bindings modpack ----
document.querySelectorAll('.srv-tab').forEach(t => t.addEventListener('click', () => {
  document.querySelectorAll('.srv-tab').forEach(x => x.classList.remove('active'));
  t.classList.add('active');
  const mode = t.dataset.mode;
  document.querySelectorAll('#srvOverlay [data-panel]').forEach(p => { p.hidden = p.dataset.panel !== mode; });
}));
function mpCard(r) {
  const el = document.createElement('div'); el.className = 'store-card';
  const tags = (r.categories || []).slice(0, 3).map(c => `<span class="tag">${esc(catLabel(c))}</span>`).join('');
  el.innerHTML = `
    <div class="store-top">${iconHtml(r.icon, r.title)}
      <div class="store-meta"><div class="store-title">${esc(r.title)}</div><div class="muted small">${r.author ? 'por ' + esc(r.author) : ''}</div></div>
    </div>
    <div class="store-desc muted small">${esc((r.description || '').slice(0, 150))}</div>
    <div class="store-tags">${tags}</div>
    <div class="store-foot"><span class="muted small">⬇ ${fmtNum(r.downloads)}</span><span class="grow"></span>
      <button class="ok sm btn-mp-create">Criar servidor</button></div>`;
  el.querySelector('.btn-mp-create').onclick = async (ev) => {
    if (!confirm(`Criar um servidor a partir de "${r.title}"?\n\nBaixa todos os mods do pack (de segundos a alguns minutos, dependendo do tamanho).`)) return;
    const btn = ev.currentTarget; btn.disabled = true; btn.textContent = 'instalando…';
    const name = $('#mpName').value.trim();
    const { ok, data } = await api('/api/servers/create-modpack', { method: 'POST', body: JSON.stringify({ slug: r.slug, name }) });
    btn.disabled = false; btn.textContent = 'Criar servidor';
    if (!ok) { alert(data.error || 'falha ao criar'); return; }
    toast('Modpack instalado: ' + data.server.name + ' — porta ' + data.server.port);
    await loadServers(); renderServerManager();
  };
  return el;
}
$('#mpSearchForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const q = $('#mpSearch').value.trim(), loader = $('#mpLoader').value;
  const box = $('#mpResults'); box.innerHTML = '<div class="muted small" style="padding:1rem">buscando…</div>';
  const { ok, data } = await api(`/api/modpacks/search?q=${encodeURIComponent(q)}&loader=${encodeURIComponent(loader)}`);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; return; }
  box.innerHTML = ''; data.results.forEach(r => box.append(mpCard(r)));
});

// ---- integrações / rede (playit / tailscale / cloudflare) ----
function intgPill(on) { return `<span class="status-pill"><span class="dot ${on ? 'on' : 'idle'}"></span>${on ? 'ativo' : 'parado'}</span>`; }
async function intgAction(name, action) { return api('/api/integrations/' + action, { method: 'POST', body: JSON.stringify({ name }) }); }
async function loadIntegrations() { const { ok, data } = await api('/api/integrations'); if (ok) renderIntegrations(data); }
function renderIntegrations(d) {
  const box = $('#intgList'); if (!box) return; box.innerHTML = '';

  // ---- playit ----
  {
    const p = d.playit, el = document.createElement('div'); el.className = 'intg';
    let body = '';
    if (!p.installed) body = `<button class="ok sm act" data-a="install">Instalar</button>`;
    else if (!p.hasSecret) {
      body = `<div class="intg-note">Crie um agente em <a href="https://playit.gg" target="_blank" rel="noopener">playit.gg</a> (Account → Agents → <b>self-managed</b>), copie o <b>secret key</b> e cole aqui:</div>
        <div class="row" style="margin-top:.5rem"><input type="password" class="pl-secret" placeholder="secret key do playit.gg" style="flex:1"><button class="ghost sm act" data-a="secret">Salvar</button></div>`;
    } else if (!p.running) body = `<button class="ok sm act" data-a="start">Ligar túnel</button>`;
    else {
      if (p.address) body += `<div class="intg-note ok">Endereço público: <code>${esc(p.address)}</code> — é esse que os amigos usam no Minecraft.</div>`;
      else body += `<div class="muted small">túnel no ar. Configure a porta no painel do playit.gg; o endereço aparece aqui (clique em Atualizar).</div>`;
      body += `<div class="row" style="margin-top:.6rem"><button class="danger sm act" data-a="stop">Desligar</button></div>`;
    }
    el.innerHTML = `<div class="intg-head"><div class="intg-ic">🌍</div><div class="intg-meta"><div class="intg-title">playit.gg</div><div class="muted small">Servidor público sem abrir porta no roteador (túnel TCP). Ideal pra Minecraft.</div></div>${intgPill(p.running)}</div><div class="intg-body">${body}</div>`;
    el.querySelectorAll('.act').forEach(b => b.onclick = async (ev) => {
      const a = ev.currentTarget.dataset.a;
      if (a === 'secret') { const s = el.querySelector('.pl-secret').value.trim(); if (!s) return; const r = await api('/api/integrations/playit-secret', { method: 'POST', body: JSON.stringify({ secret: s }) }); toast(r.ok ? 'Secret salvo — agora ligue o túnel.' : (r.data.error || 'erro')); return loadIntegrations(); }
      ev.currentTarget.disabled = true;
      const r = await intgAction('playit', a);
      if (!r.ok) alert(r.data.error || 'erro');
      setTimeout(loadIntegrations, a === 'stop' ? 500 : 2000);
    });
    box.append(el);
  }

  // ---- tailscale ----
  {
    const t = d.tailscale, el = document.createElement('div'); el.className = 'intg';
    let body = '';
    if (!t.installed) body = `<div class="intg-note">Precisa instalar no sistema (root): <code>sudo dnf install tailscale && sudo systemctl enable --now tailscaled</code></div>`;
    else if (!t.running) body = `<button class="ok sm act" data-a="start">Conectar (login)</button><div class="muted small" style="margin-top:.4rem">Se pedir permissão, rode uma vez <code>sudo tailscale up</code>.</div>`;
    else body = `<div class="intg-note ok">Conectado. IP Tailscale: <code>${esc(t.ip || '?')}</code> — amigos na sua rede Tailscale entram por <code>${esc(t.ip || 'IP')}:PORTA</code>.</div><div class="row" style="margin-top:.6rem"><button class="danger sm act" data-a="stop">Desconectar</button></div>`;
    el.innerHTML = `<div class="intg-head"><div class="intg-ic">🔒</div><div class="intg-meta"><div class="intg-title">Tailscale</div><div class="muted small">VPN privada: só quem você convidar acessa. Ótimo pra jogar entre amigos.</div></div>${intgPill(t.running)}</div><div class="intg-body">${body}</div>`;
    el.querySelectorAll('.act').forEach(b => b.onclick = async (ev) => {
      const a = ev.currentTarget.dataset.a; ev.currentTarget.disabled = true;
      const r = await intgAction('tailscale', a);
      if (a === 'start' && r.ok && r.data.loginUrl) { window.open(r.data.loginUrl, '_blank'); toast('Abra o link pra fazer login no Tailscale.'); }
      else if (!r.ok) alert(r.data.error || 'erro');
      setTimeout(loadIntegrations, 900);
    });
    box.append(el);
  }

  // ---- cloudflare ----
  {
    const c = d.cloudflare, el = document.createElement('div'); el.className = 'intg';
    let body = '';
    if (!c.installed) body = `<button class="ok sm act" data-a="install">Instalar cloudflared</button>`;
    else {
      body += `<div class="intg-note">Domínio/túnel via Cloudflare. Ótimo pro <b>painel web</b> (HTTP). Pra porta do MC (TCP) o plano grátis é limitado (precisa Spectrum pago).</div>`;
      body += `<div class="row" style="margin-top:.5rem"><input type="password" class="cf-token" placeholder="token do túnel (Zero Trust → Tunnels)" style="flex:1"><button class="ghost sm act" data-a="token">Salvar token</button></div>`;
      if (!c.running) body += `<div class="row" style="margin-top:.5rem"><button class="ok sm act" data-a="start"${c.hasToken ? '' : ' disabled title="salve o token primeiro"'}>Ligar túnel</button></div>`;
      else body += `<div class="intg-note ok" style="margin-top:.5rem">Túnel ligado.</div><div class="row" style="margin-top:.5rem"><button class="danger sm act" data-a="stop">Desligar</button></div>`;
    }
    el.innerHTML = `<div class="intg-head"><div class="intg-ic">☁️</div><div class="intg-meta"><div class="intg-title">Cloudflare Tunnel</div><div class="muted small">Expõe o painel/serviço por um domínio, com túnel seguro.</div></div>${intgPill(c.running)}</div><div class="intg-body">${body}</div>`;
    el.querySelectorAll('.act').forEach(b => b.onclick = async (ev) => {
      const a = ev.currentTarget.dataset.a;
      if (a === 'token') { const tok = el.querySelector('.cf-token').value.trim(); if (!tok) return; const r = await api('/api/integrations/cloudflare-token', { method: 'POST', body: JSON.stringify({ token: tok }) }); toast(r.ok ? 'Token salvo.' : (r.data.error || 'erro')); return loadIntegrations(); }
      ev.currentTarget.disabled = true;
      const r = await intgAction('cloudflare', a);
      if (!r.ok) alert(r.data.error || 'erro');
      setTimeout(loadIntegrations, a === 'stop' ? 500 : 1500);
    });
    box.append(el);
  }
}
$('#intgReload').addEventListener('click', loadIntegrations);

boot();
