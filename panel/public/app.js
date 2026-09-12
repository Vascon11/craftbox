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
  else {
    $('#login').hidden = false;
    if (data.multiUser) { const lu = $('#loginUser'); lu.hidden = false; lu.required = true; lu.focus(); }
    if (!data.configured) { $('#loginErr').hidden = false; $('#loginErr').textContent = 'Senha ainda não configurada no servidor (node server.js --hash).'; }
  }
}
$('#loginForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const body = { password: $('#pw').value };
  const lu = $('#loginUser'); if (!lu.hidden) body.user = lu.value.trim();
  const { ok, data } = await api('/api/login', { method: 'POST', body: JSON.stringify(body) });
  if (ok) { $('#login').hidden = true; showApp(); }
  else { $('#loginErr').hidden = false; $('#loginErr').textContent = data.error || 'Falha no login'; }
});
$('#logout').addEventListener('click', async () => { await api('/api/logout', { method: 'POST' }); location.reload(); });

async function showApp() { $('#app').hidden = false; await loadServers(); reloadAll(); }
function reloadAll() { loadProps(); startStatus(); startLogs(); loadBackups(); loadContentInfo(); loadInstalled(); loadIntegrations(); loadAudit(); loadUsers(); }

// ---- multi-servidor ----
async function loadServers() {
  const { ok, data } = await api('/api/servers');
  const wrap = $('#serverBar'), tabS = $('#tabServers');
  const multi = ok && data.multi;
  if (tabS) tabS.hidden = !multi;
  if (!multi) { if (wrap) wrap.hidden = true; currentServer = ''; return; }
  wrap.hidden = false;
  currentServer = data.activeId || (data.servers[0] && data.servers[0].id) || '';
  const sel = $('#serverSelect'); sel.innerHTML = '';
  data.servers.forEach(s => {
    const label = `${s.name} · ${s.loader}${s.mcVersion ? ' ' + s.mcVersion : ''}${s.active === 'active' ? ' 🟢' : ''}`;
    const o = new Option(label, s.id); sel.add(o);
  });
  sel.value = currentServer;
  renderServersOverview(data.servers);
}
function renderServersOverview(servers) {
  const box = $('#serversGrid'); if (!box) return; box.innerHTML = '';
  servers.forEach(s => {
    const running = s.active === 'active';
    const el = document.createElement('div'); el.className = 'store-card';
    el.innerHTML = `
      <div class="store-top" style="justify-content:space-between">
        <div class="store-title">${esc(s.name)}</div>
        <span class="status-pill"><span class="dot ${running ? 'on' : 'idle'}"></span>${running ? 'no ar' : 'desligado'}</span>
      </div>
      <div class="muted small">${esc(s.loader)}${s.mcVersion ? ' ' + esc(s.mcVersion) : ''} · porta ${s.port || '?'}${s.selected ? ' · <b>selecionado</b>' : ''}${s.modpack ? ' · 📦 modpack' : ''}</div>
      <div class="store-foot">
        <button class="ghost sm act-sel">Selecionar</button><span class="grow"></span>
        ${running ? '<button class="danger sm act-stop">Desligar</button>' : '<button class="ok sm act-start">Ligar</button>'}
      </div>`;
    el.querySelector('.act-sel').onclick = () => { switchServer(s.id); const t = document.querySelector('.tab[data-tab="painel"]'); if (t) t.click(); };
    const st = el.querySelector('.act-start');
    if (st) st.onclick = async (ev) => { ev.currentTarget.disabled = true; await api('/api/power?server=' + encodeURIComponent(s.id), { method: 'POST', body: JSON.stringify({ action: 'start' }) }); toast('Ligando ' + s.name + '…'); setTimeout(loadServers, 2500); };
    const sp = el.querySelector('.act-stop');
    if (sp) sp.onclick = async (ev) => { if (!await confirmDialog('Desligar "' + s.name + '"?', { okText: 'Desligar', danger: true })) return; ev.currentTarget.disabled = true; await api('/api/power?server=' + encodeURIComponent(s.id), { method: 'POST', body: JSON.stringify({ action: 'stop' }) }); toast('Desligando ' + s.name + '…'); setTimeout(loadServers, 1500); };
    box.append(el);
  });
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
      const nome = await promptDialog('Nome da cópia:', s.name + ' (cópia)'); if (!nome) return;
      const r = await api('/api/servers/clone', { method: 'POST', body: JSON.stringify({ id: s.id, name: nome }) });
      if (!r.ok) return toast(r.data.error || 'falha ao clonar', 'err');
      toast('Servidor clonado.'); await loadServers(); renderServerManager();
    };
    el.querySelector('.btn-del').onclick = async () => {
      if (!await confirmDialog(`Apagar o servidor "${s.name}"? Isso remove o mundo e tudo dele. Não dá pra desfazer.`, { okText: 'Apagar', danger: true })) return;
      const r = await api('/api/servers?id=' + encodeURIComponent(s.id), { method: 'DELETE' });
      if (!r.ok) return toast(r.data.error || 'falha ao apagar', 'err');
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
$('#btnStop').addEventListener('click', async () => { if (await confirmDialog('Desligar o servidor? Jogadores serão desconectados.', { okText: 'Desligar', danger: true })) power('stop'); });

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

// ---- trocar senha do painel ----
$('#pwSave').addEventListener('click', async () => {
  const cur = $('#pwCur').value, np = $('#pwNew').value, np2 = $('#pwNew2').value, msg = $('#pwMsg');
  if (!cur || !np) { msg.textContent = 'preencha os campos.'; return; }
  if (np !== np2) { msg.textContent = 'a confirmação não bate.'; return; }
  if (np.length < 4) { msg.textContent = 'mínimo 4 caracteres.'; return; }
  $('#pwSave').disabled = true; msg.textContent = 'salvando…';
  const { ok, data } = await api('/api/change-password', { method: 'POST', body: JSON.stringify({ current: cur, newPassword: np }) });
  $('#pwSave').disabled = false;
  if (ok) { msg.textContent = '✓ senha trocada'; $('#pwCur').value = $('#pwNew').value = $('#pwNew2').value = ''; toast('Senha do painel trocada.'); }
  else { msg.textContent = data.error || 'erro'; }
  setTimeout(() => msg.textContent = '', 6000);
});

// ---- usuários do painel ----
async function loadUsers() {
  const box = $('#usersList'); if (!box) return;
  const { ok, data } = await api('/api/users');
  if (!ok) { box.innerHTML = ''; return; }
  const form = $('#userAddForm'); if (form) form.style.display = data.isAdmin ? '' : 'none';
  box.innerHTML = '';
  if (!data.multiUser) { box.innerHTML = '<div class="muted small">Nenhuma conta ainda — o painel usa senha única. Crie o 1º usuário pra ativar contas.</div>'; return; }
  data.users.forEach(u => {
    const el = document.createElement('div'); el.className = 'inst-row';
    el.innerHTML = `<div class="inst-meta"><div class="inst-title">${esc(u.user)} ${u.role === 'admin' ? '<span class="tag">admin</span>' : ''}${u.user === data.me ? ' <span class="tag">você</span>' : ''}</div></div>`;
    if (data.isAdmin && u.user !== data.me) {
      const act = document.createElement('div'); act.className = 'inst-actions';
      const b = document.createElement('button'); b.className = 'danger sm'; b.textContent = 'Remover';
      b.onclick = async () => {
        if (!await confirmDialog('Remover o usuário "' + u.user + '"?', { okText: 'Remover', danger: true })) return;
        const r = await api('/api/users?user=' + encodeURIComponent(u.user), { method: 'DELETE' });
        if (!r.ok) return toast(r.data.error || 'erro', 'err');
        toast('Usuário removido.'); loadUsers();
      };
      act.append(b); el.append(act);
    }
    box.append(el);
  });
}
$('#userAddForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const user = $('#uAddName').value.trim(), password = $('#uAddPass').value, role = $('#uAddRole').value;
  if (!user || !password) { $('#usersMsg').textContent = 'preencha usuário e senha.'; return; }
  const { ok, data } = await api('/api/users', { method: 'POST', body: JSON.stringify({ user, password, role }) });
  if (!ok) { $('#usersMsg').textContent = data.error || 'erro'; return; }
  $('#uAddName').value = ''; $('#uAddPass').value = ''; $('#usersMsg').textContent = '';
  toast(data.firstUser ? 'Contas ativadas — você agora é admin.' : 'Usuário adicionado.');
  loadUsers();
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
function toast(msg, type) {
  let t = $('#toast'); if (!t) { t = document.createElement('div'); t.id = 'toast'; document.body.append(t); }
  t.textContent = msg; t.className = 'show' + (type === 'err' ? ' err' : ''); clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove('show'), 5000);
}
// modal de confirmação no tema do painel (substitui o confirm() do navegador)
function confirmDialog(message, opts = {}) {
  return new Promise((resolve) => {
    const ov = document.createElement('div'); ov.className = 'modal';
    ov.innerHTML = `<div class="modal-card confirm-card"><div class="confirm-msg"></div>
      <div class="confirm-actions"><button class="ghost" data-x="0"></button><button data-x="1"></button></div></div>`;
    ov.querySelector('.confirm-msg').textContent = message;
    ov.querySelector('[data-x="0"]').textContent = opts.cancelText || 'Cancelar';
    const okb = ov.querySelector('[data-x="1"]'); okb.textContent = opts.okText || 'Confirmar'; okb.className = opts.danger ? 'danger' : 'ok';
    document.body.append(ov);
    const done = (v) => { ov.remove(); document.removeEventListener('keydown', onKey); resolve(v); };
    const onKey = (e) => { if (e.key === 'Escape') done(false); if (e.key === 'Enter') done(true); };
    document.addEventListener('keydown', onKey);
    ov.addEventListener('click', (e) => { if (e.target === ov) done(false); });
    ov.querySelector('[data-x="0"]').onclick = () => done(false);
    okb.onclick = () => done(true); okb.focus();
  });
}
// modal de input (substitui o prompt() do navegador)
function promptDialog(message, def = '') {
  return new Promise((resolve) => {
    const ov = document.createElement('div'); ov.className = 'modal';
    ov.innerHTML = `<div class="modal-card confirm-card"><div class="confirm-msg"></div>
      <input class="pd-in" style="width:100%;margin:.9rem 0">
      <div class="confirm-actions"><button class="ghost" data-x="0">Cancelar</button><button class="ok" data-x="1">OK</button></div></div>`;
    ov.querySelector('.confirm-msg').textContent = message;
    const inp = ov.querySelector('.pd-in'); inp.value = def;
    document.body.append(ov);
    const done = (v) => { ov.remove(); resolve(v); };
    ov.addEventListener('click', (e) => { if (e.target === ov) done(null); });
    ov.querySelector('[data-x="0"]').onclick = () => done(null);
    ov.querySelector('[data-x="1"]').onclick = () => done(inp.value);
    inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') done(inp.value); if (e.key === 'Escape') done(null); });
    inp.focus(); inp.select();
  });
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

async function doSearch(offset) {
  const off = typeof offset === 'number' ? offset : 0;
  const q = $('#searchIn').value.trim(), sort = $('#sortSelect').value;
  const box = $('#searchResults'); box.innerHTML = '<div class="muted small" style="padding:1rem">buscando…</div>';
  const pgr = $('#searchPager'); if (pgr) pgr.innerHTML = '';
  const qs = `q=${encodeURIComponent(q)}&sort=${encodeURIComponent(sort)}&category=${encodeURIComponent(activeCat)}&offset=${off}`;
  const { ok, data } = await api('/api/content/search?' + qs);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results || !data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; return; }
  box.innerHTML = ''; data.results.forEach(r => box.append(storeCard(r)));
  renderPager(data.total || 0, data.offset || 0, data.limit || 24);
}
function renderPager(total, offset, limit) {
  const box = $('#searchPager'); if (!box) return; box.innerHTML = '';
  if (total <= limit) return;
  const page = Math.floor(offset / limit) + 1, pages = Math.ceil(total / limit);
  const prev = document.createElement('button'); prev.className = 'ghost sm'; prev.textContent = '◀ Anterior'; prev.disabled = offset <= 0;
  prev.onclick = () => { doSearch(Math.max(0, offset - limit)); $('#searchResults').scrollIntoView({ block: 'nearest' }); };
  const info = document.createElement('span'); info.className = 'muted small'; info.textContent = `página ${page} de ${pages} · ${total} resultados`;
  const next = document.createElement('button'); next.className = 'ghost sm'; next.textContent = 'Próxima ▶'; next.disabled = page >= pages;
  next.onclick = () => { doSearch(offset + limit); $('#searchResults').scrollIntoView({ block: 'nearest' }); };
  box.append(prev, info, next);
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
    else { btn.disabled = false; btn.textContent = old; toast(data.error || 'falha ao instalar', 'err'); }
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
      if (!await confirmDialog('Remover ' + it.title + '?', { okText: 'Remover', danger: true })) return;
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
$('#serversManage').addEventListener('click', () => { $('#srvOverlay').hidden = false; renderServerManager(); });
$('#tabServers').addEventListener('click', loadServers);
$('#srvClose').addEventListener('click', () => $('#srvOverlay').hidden = true);
$('#srvOverlay').addEventListener('click', (e) => { if (e.target === $('#srvOverlay')) $('#srvOverlay').hidden = true; });
$('#srvCreateForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const name = $('#srvName').value.trim(); if (!name) return;
  const loader = $('#srvLoader').value, version = $('#srvVersion').value.trim();
  const btn = $('#srvCreateBtn'); btn.disabled = true; btn.textContent = 'criando… (baixando o servidor)';
  const r = await api('/api/servers/create', { method: 'POST', body: JSON.stringify({ name, loader, version }) });
  btn.disabled = false; btn.textContent = 'Criar servidor';
  if (!r.ok) { toast(r.data.error || 'falha ao criar', 'err'); return; }
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
    if (!await confirmDialog(`Criar um servidor a partir de "${r.title}"?\n\nBaixa todos os mods do pack (de segundos a alguns minutos, dependendo do tamanho).`, { okText: 'Criar' })) return;
    const btn = ev.currentTarget; btn.disabled = true; btn.textContent = 'instalando…';
    const name = $('#mpName').value.trim();
    const { ok, data } = await api('/api/servers/create-modpack', { method: 'POST', body: JSON.stringify({ slug: r.slug, name }) });
    btn.disabled = false; btn.textContent = 'Criar servidor';
    if (!ok) { toast(data.error || 'falha ao criar', 'err'); return; }
    toast('Modpack instalado: ' + data.server.name + ' — porta ' + data.server.port);
    await loadServers(); renderServerManager();
  };
  return el;
}
function renderPagerInto(sel, total, offset, limit, go) {
  const box = $(sel); if (!box) return; box.innerHTML = '';
  if (total <= limit) return;
  const page = Math.floor(offset / limit) + 1, pages = Math.ceil(total / limit);
  const prev = document.createElement('button'); prev.className = 'ghost sm'; prev.textContent = '◀ Anterior'; prev.disabled = offset <= 0;
  prev.onclick = () => go(Math.max(0, offset - limit));
  const info = document.createElement('span'); info.className = 'muted small'; info.textContent = `página ${page} de ${pages} · ${total} resultados`;
  const next = document.createElement('button'); next.className = 'ghost sm'; next.textContent = 'Próxima ▶'; next.disabled = page >= pages;
  next.onclick = () => go(offset + limit);
  box.append(prev, info, next);
}
async function doMpSearch(offset) {
  const off = typeof offset === 'number' ? offset : 0;
  const q = $('#mpSearch').value.trim(), loader = $('#mpLoader').value;
  const box = $('#mpResults'); box.innerHTML = '<div class="muted small" style="padding:1rem">buscando…</div>';
  const pgr = $('#mpPager'); if (pgr) pgr.innerHTML = '';
  const { ok, data } = await api(`/api/modpacks/search?q=${encodeURIComponent(q)}&loader=${encodeURIComponent(loader)}&offset=${off}`);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results || !data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; return; }
  box.innerHTML = ''; data.results.forEach(r => box.append(mpCard(r)));
  renderPagerInto('#mpPager', data.total || 0, data.offset || 0, data.limit || 24, doMpSearch);
}
$('#mpSearchForm').addEventListener('submit', (e) => { e.preventDefault(); doMpSearch(0); });

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
      if (!r.ok) toast(r.data.error || 'erro', 'err');
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
    else body = `<div class="intg-note ok">Conectado. IP Tailscale: <code>${esc(t.ip || '?')}</code> — amigos na sua rede Tailscale entram por <code>${esc(t.ip || 'IP')}:PORTA</code>.</div><div class="row" style="margin-top:.6rem"><button class="danger sm act" data-a="stop">Desconectar</button></div><div class="muted small" style="margin-top:.4rem">Se o botão der erro de permissão, rode uma vez: <code>sudo tailscale set --operator=$USER</code></div>`;
    el.innerHTML = `<div class="intg-head"><div class="intg-ic">🔒</div><div class="intg-meta"><div class="intg-title">Tailscale</div><div class="muted small">VPN privada: só quem você convidar acessa. Ótimo pra jogar entre amigos.</div></div>${intgPill(t.running)}</div><div class="intg-body">${body}</div>`;
    el.querySelectorAll('.act').forEach(b => b.onclick = async (ev) => {
      const a = ev.currentTarget.dataset.a; ev.currentTarget.disabled = true;
      const r = await intgAction('tailscale', a);
      if (a === 'start' && r.ok && r.data.loginUrl) { window.open(r.data.loginUrl, '_blank'); toast('Abra o link pra fazer login no Tailscale.'); }
      else if (!r.ok) toast(r.data.error || 'erro', 'err');
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
      if (!r.ok) toast(r.data.error || 'erro', 'err');
      setTimeout(loadIntegrations, a === 'stop' ? 500 : 1500);
    });
    box.append(el);
  }
}
$('#intgReload').addEventListener('click', loadIntegrations);

// ---- auditoria ----
async function loadAudit() {
  const { ok, data } = await api('/api/audit?lines=300');
  const body = $('#auditBody'); if (!body) return;
  body.innerHTML = '';
  const items = (data && data.items) || [];
  if (!ok || !items.length) { body.innerHTML = '<tr><td colspan="6" class="muted small">nada registrado ainda.</td></tr>'; return; }
  items.forEach(it => {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td class="muted small" style="white-space:nowrap">${esc(new Date(it.ts).toLocaleString('pt-BR'))}</td><td><span class="tag">${esc(it.action)}</span></td><td>${esc(it.detail || '')}</td><td class="muted small">${esc(it.user || '-')}</td><td class="muted small">${esc(it.server || '-')}</td><td class="muted small">${esc(it.ip || '-')}</td>`;
    body.append(tr);
  });
}
$('#auditReload').addEventListener('click', loadAudit);
$('#auditClear').addEventListener('click', async () => {
  if (!await confirmDialog('Limpar todo o histórico? Isso apaga os registros e não dá pra desfazer.', { okText: 'Limpar', danger: true })) return;
  const { ok, data } = await api('/api/audit', { method: 'DELETE' });
  if (!ok) return toast(data.error || 'erro ao limpar', 'err');
  toast('Histórico limpo.'); loadAudit();
});

// sub-abas do Console (Servidor / Auditoria)
document.querySelectorAll('.con-tab').forEach(t => t.addEventListener('click', () => {
  document.querySelectorAll('.con-tab').forEach(x => x.classList.remove('active'));
  t.classList.add('active');
  const mode = t.dataset.con;
  document.querySelectorAll('#console [data-con-panel]').forEach(p => { p.hidden = p.dataset.conPanel !== mode; });
  const aw = $('#autologWrap'); if (aw) aw.hidden = mode !== 'mc';
  if (mode === 'audit') loadAudit();
}));

boot();
