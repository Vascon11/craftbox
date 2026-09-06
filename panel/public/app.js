'use strict';
const $ = (s) => document.querySelector(s);
const api = async (path, opts = {}) => {
  const r = await fetch(path, { headers: { 'Content-Type': 'application/json' }, ...opts });
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

function showApp() { $('#app').hidden = false; loadProps(); startStatus(); startLogs(); loadBackups(); loadContentInfo(); loadInstalled(); }

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
  dot.className = 'dot ' + (active === 'active' ? 'on' : active === 'activating' ? 'act' : 'off');
  pillText.textContent = active === 'active' ? 'no ar' : active === 'activating' ? 'iniciando…' : 'parado';
  $('#svcState').textContent = active === 'active' ? 'No ar' : active === 'activating' ? 'Iniciando…' : 'Parado';
  $('#svcMeta').textContent = active === 'active' ? `há ${fmtDur(data.uptime)}` : '—';
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
  document.querySelectorAll('#painel button').forEach(b => b.disabled = true);
  msg.textContent = 'executando…';
  const { ok, data } = await api('/api/power', { method: 'POST', body: JSON.stringify({ action }) });
  msg.textContent = ok ? 'ok' : ('erro: ' + (data.output || data.error || '')).slice(0, 200);
  setTimeout(() => { document.querySelectorAll('#painel button').forEach(b => b.disabled = false); refreshStatus(); }, 1500);
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

// ---- console (rcon) ----
$('#rconForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const cmd = $('#rconIn').value.trim(); if (!cmd) return;
  const out = $('#rconOut');
  out.textContent += `> ${cmd}\n`;
  $('#rconIn').value = '';
  const { ok, data } = await api('/api/rcon', { method: 'POST', body: JSON.stringify({ command: cmd }) });
  out.textContent += (ok ? (data.response || '(ok)') : ('erro: ' + (data.error || ''))) + '\n';
  out.scrollTop = out.scrollHeight;
});

// ---- logs ----
let logTimer;
async function refreshLogs() {
  if (!$('#autolog').checked) return;
  const { ok, data } = await api('/api/logs?lines=250');
  if (ok) { const el = $('#logOut'); const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40; el.textContent = data.log || '(sem logs)'; if (atBottom) el.scrollTop = el.scrollHeight; }
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

// ---- conteudo (mods/plugins) ----
let contentKind = 'plugins';
async function loadContentInfo() {
  const { data } = await api('/api/content/info');
  contentKind = data.kind || 'plugins';
  $('#contentInfo').textContent = `${data.loader || '?'} · ${contentKind} · MC ${data.mcVersion || '?'}`;
  $('#offlineToggle').checked = data.onlineMode === false;
}
$('#searchForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const q = $('#searchIn').value.trim();
  const box = $('#searchResults'); box.innerHTML = '<div class="muted small">buscando…</div>';
  const { ok, data } = await api('/api/content/search?q=' + encodeURIComponent(q));
  if (!ok) { box.innerHTML = `<div class="err small">${data.error || 'erro'}</div>`; return; }
  if (!data.results.length) { box.innerHTML = '<div class="muted small">nada encontrado</div>'; return; }
  box.innerHTML = '';
  data.results.forEach(r => {
    const el = document.createElement('div'); el.className = 'result';
    el.innerHTML = `<div class="result-body"><b>${r.title}</b> <span class="muted small">· ${(r.downloads || 0).toLocaleString('pt-BR')} downloads</span>
      <div class="muted small">${(r.description || '').slice(0, 120)}</div></div>`;
    const b = document.createElement('button'); b.className = 'ok'; b.textContent = 'Instalar';
    b.onclick = async () => {
      b.disabled = true; b.textContent = 'instalando…';
      const res = await api('/api/content/install', { method: 'POST', body: JSON.stringify({ slug: r.slug }) });
      b.textContent = res.ok ? '✓ instalado' : 'erro';
      if (!res.ok) { b.disabled = false; b.textContent = 'Instalar'; alert(res.data.error || 'falha'); }
      loadInstalled();
    };
    el.append(b); box.append(el);
  });
});
async function loadInstalled() {
  const { data } = await api('/api/content/installed');
  const box = $('#installedList'); box.innerHTML = '';
  const files = (data && data.files) || [];
  if (!files.length) { box.innerHTML = '<div class="muted small">nenhum arquivo instalado.</div>'; return; }
  files.forEach(f => {
    const el = document.createElement('div'); el.className = 'result';
    el.innerHTML = `<div class="result-body">${f.name} <span class="muted small">· ${f.sizeMB} MB</span></div>`;
    const b = document.createElement('button'); b.className = 'danger'; b.textContent = 'Remover';
    b.onclick = async () => { if (!confirm('Remover ' + f.name + '?')) return; await api('/api/content/installed?file=' + encodeURIComponent(f.name), { method: 'DELETE' }); loadInstalled(); };
    el.append(b); box.append(el);
  });
}
$('#reloadInstalled').addEventListener('click', loadInstalled);

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

boot();
