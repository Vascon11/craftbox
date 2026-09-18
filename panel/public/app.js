'use strict';
const $ = (s) => document.querySelector(s);
let currentServer = '';
const api = async (path, opts = {}) => {
  let url = path;
  const skip = path === '/api/login' || path === '/api/logout' || path === '/api/authcheck' || path.startsWith('/api/servers');
  if (currentServer && path.startsWith('/api/') && !skip && !path.includes('server=')) {
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
function reloadAll() { loadProps(); startStatus(); startLogs(); loadBackups(); loadContentInfo(); loadInstalled(); loadIntegrations(); loadAudit(); loadUsers(); loadNet(); measureWorld(); }

// ---- multi-servidor ----
async function loadServers() {
  const { ok, data } = await api('/api/servers');
  if (!ok) return;
  const wrap = $('#serverBar');
  const multi = data.multi;
  if (wrap) wrap.hidden = !multi;
  if (multi) {
    currentServer = data.activeId || (data.servers[0] && data.servers[0].id) || '';
    const sel = $('#serverSelect'); sel.innerHTML = '';
    data.servers.forEach(s => {
      const label = `${s.name} · ${s.loader}${s.mcVersion ? ' ' + s.mcVersion : ''}${s.active === 'active' ? ' 🟢' : ''}`;
      const o = new Option(label, s.id); sel.add(o);
    });
    sel.value = currentServer;
    const cur = data.servers.find(s => s.id === currentServer);
    consoleSrv(cur ? cur.name : currentServer);
  } else currentServer = '';
  renderServersOverview(data.servers);
}
function serverPower(id, action) { return api('/api/servers/' + encodeURIComponent(id) + '/' + action, { method: 'POST', body: JSON.stringify({}) }); }
function renderServersOverview(servers) {
  const box = $('#serversGrid'); if (!box) return; box.innerHTML = '';
  const runningAny = (servers || []).find(s => s.active === 'active');
  const hint = $('#serversHint');
  if (hint) {
    hint.innerHTML = runningAny
      ? `<b>⚠</b> "Instâncias disponíveis. Por limitações de hardware, execute apenas um servidor por vez. Clique em \"Selecionar\" para gerenciar no painel acima."<br><span style="color:var(--danger,#e0604a)">Já tem <b>${esc(runningAny.name)}</b> no ar — ligar outro pode travar o hardware.</span>`
      : 'Instâncias disponíveis. Por limitações de hardware, execute apenas um servidor por vez. Clique em "Selecionar" para gerenciar no painel acima.';
  }
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
    if (st) st.onclick = async (ev) => {
      const other = (servers || []).find(x => x.active === 'active' && x.id !== s.id);
      if (other && !await confirmDialog('Já tem "' + other.name + '" no ar. Ligar "' + s.name + '" agora pode travar o hardware. Continuar?', { okText: 'Ligar mesmo assim', danger: true })) return;
      ev.currentTarget.disabled = true; await serverPower(s.id, 'start'); toast('Ligando ' + s.name + '…'); setTimeout(loadServers, 2500);
    };
    const sp = el.querySelector('.act-stop');
    if (sp) sp.onclick = async (ev) => { if (!await confirmDialog('Desligar "' + s.name + '"?', { okText: 'Desligar', danger: true })) return; ev.currentTarget.disabled = true; await serverPower(s.id, 'stop'); toast('Desligando ' + s.name + '…'); setTimeout(loadServers, 1500); };
    box.append(el);
  });
}
async function switchServer(id) {
  await api('/api/servers/select', { method: 'POST', body: JSON.stringify({ id }) });
  currentServer = id;
  resetConsole();
  consoleSrv(id);
  reloadAll();
  refreshLogs(true);
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
  if (t.dataset.tab === 'modpacks' && !$('#mpResults').children.length) doMpSearch(0);
  if (t.dataset.tab === 'painel') loadServers();
  if (t.dataset.tab === 'conteudo') { loadContentInfo(); loadInstalled(); }
  if (t.dataset.tab === 'console') refreshLogs(true);
  if (t.dataset.tab === 'integra') loadIntegrations();
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
    $('#diskBar').classList.toggle('hot', !!s.disk.low);
    const dw = $('#diskWarn');
    if (s.disk.low) {
      dw.hidden = false;
      dw.textContent = `⚠ Só ${s.disk.freeGB} GB livres (abaixo de ${s.disk.lowGB} GB): o mundo pode falhar ao salvar e o login dos jogadores pode travar. Libere espaço. No Docker Desktop esse número é o disco virtual do WSL, não o C: do Windows.`;
    } else dw.hidden = true;
  }
}
let statusTimer;
function startStatus() { refreshStatus(); clearInterval(statusTimer); statusTimer = setInterval(refreshStatus, 3000); }

// ---- power ----
async function power(action, serverId) {
  const msg = $('#powerMsg');
  const labels = { start: 'ligando', restart: 'reiniciando', stop: 'desligando' };
  const target = serverId || currentServer;
  document.querySelectorAll('#painel button').forEach(b => b.disabled = true);
  msg.textContent = (labels[action] || 'executando') + '…';
  const { ok, data } = await serverPower(target, action);
  msg.textContent = ok
    ? `✓ comando de ${labels[action] || action} enviado para ${target} — acompanhe o estado acima`
    : ('erro: ' + (data.output || data.error || '')).slice(0, 200);
  setTimeout(() => { document.querySelectorAll('#painel button').forEach(b => b.disabled = false); refreshStatus(); }, 1500);
  setTimeout(() => { const m = $('#powerMsg'); if (m) m.textContent = ''; }, 6000);
}
$('#btnStart').addEventListener('click', () => power('start', currentServer));
$('#btnRestart').addEventListener('click', () => power('restart', currentServer));
$('#btnStop').addEventListener('click', async () => { if (await confirmDialog('Desligar o servidor? Jogadores serão desconectados.', { okText: 'Desligar', danger: true })) power('stop', currentServer); });

let serversPolling = false;
async function pollServers() {
  if (serversPolling) return;
  serversPolling = true;
  try { await loadServers(); } catch {} finally { serversPolling = false; }
}
setInterval(pollServers, 8000);

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
const P_SEC = {
  'Rede & Acesso': ['server-ip', 'server-ipv6', 'server-port', 'query.port', 'enable-query', 'enable-rcon', 'rcon.port', 'rcon.password', 'broadcast-rcon-to-ops', 'online-mode', 'white-list', 'enforce-whitelist', 'announce-player-achievements'],
  'Gameplay & Regras': ['motd', 'difficulty', 'gamemode', 'pvp', 'hardcore', 'allow-nether', 'spawn-monsters', 'spawn-animals', 'spawn-npcs', 'spawn-protection', 'level-name', 'level-seed', 'level-type', 'generate-structures', 'allow-flight', 'force-gamemode', 'enable-command-block', 'max-world-size'],
  'Desempenho & Limites': ['max-players', 'view-distance', 'simulation-distance', 'max-tick-time', 'entity-broadcast-range-percentage', 'network-compression-threshold', 'player-idle-timeout'],
};
const SEC_ORDER = ['Rede & Acesso', 'Gameplay & Regras', 'Desempenho & Limites', 'Avançado'];
let propsCache = {};
async function loadProps() {
  const { data } = await api('/api/properties');
  propsCache = data.properties || {};
  renderProps();
}
function renderProps() {
  const form = $('#propsForm'); if (!form) return;
  form.innerHTML = '';
  const filter = ($('#cfgSearch').value.trim() || '').toLowerCase();
  const entryOf = (k) => COMMON.find(c => c[0] === k);
  const labelOf = (k) => (entryOf(k) && entryOf(k)[1]) || k;
  const typeOf = (k) => (entryOf(k) && entryOf(k)[2]) || 'text';
  const secOf = (k) => { for (const s of SEC_ORDER) if (s !== 'Avançado' && P_SEC[s].includes(k)) return s; return 'Avançado'; };
  const visible = (k) => !filter || k.toLowerCase().includes(filter) || labelOf(k).toLowerCase().includes(filter);
  const shown = (k) => (k in propsCache) || ['bool', 'select'].includes(typeOf(k));
  const secSet = Object.fromEntries(SEC_ORDER.map(s => [s, new Set()]));
  COMMON.forEach(([k]) => secSet[secOf(k)].add(k));
  Object.keys(propsCache).forEach(k => secSet[secOf(k)].add(k));
  let any = false;
  SEC_ORDER.forEach(sec => {
    let keys;
    if (sec === 'Avançado') keys = [...secSet[sec]].filter(k => !Object.values(P_SEC).flat().includes(k)).sort((a, b) => labelOf(a).localeCompare(labelOf(b)));
    else keys = P_SEC[sec].filter(k => secSet[sec].has(k));
    const items = keys.filter(k => shown(k) && visible(k));
    if (!items.length) return;
    any = true;
    const d = document.createElement('details'); d.className = 'cfg-sec'; d.open = true;
    const sum = document.createElement('summary'); sum.textContent = `${sec} · ${items.length} ${items.length === 1 ? 'campo' : 'campos'}`;
    const wrap = document.createElement('div'); wrap.className = 'props-sec';
    items.forEach(k => {
      const val = propsCache[k] ?? '';
      const el = document.createElement('div'); el.className = 'prop';
      let ctrl;
      if (typeOf(k) === 'bool') {
        ctrl = document.createElement('select');
        ['true', 'false'].forEach(o => ctrl.add(new Option(o === 'true' ? 'sim' : 'não', o)));
        ctrl.value = (val === 'true') ? 'true' : 'false';
      } else if (typeOf(k) === 'select') {
        ctrl = document.createElement('select');
        (entryOf(k)[3] || []).forEach(o => ctrl.add(new Option(o, o)));
        ctrl.value = val;
      } else {
        ctrl = document.createElement('input'); ctrl.type = typeOf(k) === 'number' ? 'number' : 'text'; ctrl.value = val;
      }
      ctrl.dataset.key = k;
      el.append(Object.assign(document.createElement('label'), { textContent: labelOf(k) }), ctrl);
      wrap.append(el);
    });
    d.append(sum, wrap);
    form.append(d);
  });
  if (!any) { const e = document.createElement('div'); e.className = 'muted small'; e.textContent = 'nenhuma configuração encontrada'; form.append(e); }
}
$('#cfgSearch').addEventListener('input', renderProps);
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

// ---- console (log ao vivo + comandos RCON, mesma tela, por servidor ativo) ----
let logBuf = '';   // log do servidor (atualiza sozinho)
let cmdBuf = '';   // comandos digitados + respostas (persistem entre refreshes)
let logsGen = 0;   // descarta respostas antigas (troca de servidor)
function logHighlight(text) {
  return String(text).split('\n').map(line => {
    let cls = '';
    if (/\[(?:ERROR|FATAL)\]/i.test(line) || /Exception in|Caused by:|Internal Exception|Failed to|Can't keep up/i.test(line)) cls = 'log-err';
    else if (/\[WARN\]/i.test(line) || /WARNING|deprecated/i.test(line)) cls = 'log-warn';
    return cls ? `<span class="${cls}">${esc(line)}</span>` : esc(line);
  }).join('\n');
}
let consolePinned = true;
function renderConsole() {
  const el = $('#consoleOut'); if (!el) return;
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  const parts = [];
  if (logBuf) parts.push(logHighlight(logBuf));
  if (cmdBuf) parts.push('──── comandos ────\n' + esc(cmdBuf));
  el.innerHTML = parts.join('\n') || '(sem logs)';
  const auto = !$('#autoScroll') || $('#autoScroll').checked;
  if (auto && (consolePinned || atBottom)) el.scrollTop = el.scrollHeight;
}
$('#autoScroll').addEventListener('change', (e) => { if (e.currentTarget.checked) { consolePinned = true; const el = $('#consoleOut'); if (el) el.scrollTop = el.scrollHeight; } });
{
  const el = $('#consoleOut');
  if (el) el.addEventListener('scroll', () => { const near = el.scrollHeight - el.scrollTop - el.clientHeight < 50; consolePinned = near; });
}
function consoleSrv(srv) {
  const el = $('#consoleSrvName'); if (el) el.textContent = srv ? 'console: ' + srv : '';
}
function resetConsole() { logBuf = ''; cmdBuf = ''; logsGen++; consolePinned = true; $('#consoleOut').textContent = '(sem logs)'; }
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
async function refreshLogs(force) {
  if (!force && !$('#autolog').checked) return;
  const g = ++logsGen;
  const { ok, data } = await api('/api/logs?lines=250');
  if (ok && g === logsGen) { logBuf = data.log || ''; renderConsole(); }
}
function startLogs() { clearInterval(logTimer); logTimer = setInterval(refreshLogs, 3000); refreshLogs(true); }

async function copyLogs() {
  const el = $('#consoleOut');
  const text = (el && el.innerText.trim()) ? el.innerText : '(console vazio)';
  let ok = false;
  try { if (navigator.clipboard && window.isSecureContext) { await navigator.clipboard.writeText(text); ok = true; } } catch {}
  if (!ok) {
    const ta = document.createElement('textarea');
    ta.value = text; ta.style.cssText = 'position:fixed;top:0;left:0;opacity:0;pointer-events:none';
    document.body.append(ta); ta.focus(); ta.select();
    try { ok = document.execCommand('copy'); } catch {}
    ta.remove();
  }
  const btn = $('#btnCopyLogs');
  if (btn) {
    const old = btn.textContent;
    btn.textContent = ok ? 'Copiado!' : 'Falha ao copiar';
    btn.classList.toggle('ok', ok);
    setTimeout(() => { if (btn) { btn.textContent = old; btn.classList.remove('ok'); } }, 2000);
  }
  toast(ok ? 'Logs copiados para a área de transferência' : 'Não foi possível copiar os logs', ok ? '' : 'err');
}
$('#btnCopyLogs').addEventListener('click', copyLogs);

// ---- backups ----
async function loadBackups() {
  const { data } = await api('/api/backups');
  const body = $('#bkBody'); body.innerHTML = '';
  const used = $('#bkUsed');
  if (used) used.textContent = data.totalMB ? `~${data.totalMB} MB usados em disco` : '';
  (data.backups || []).forEach(b => {
    const tr = document.createElement('tr');
    const dl = document.createElement('a'); dl.className = 'ghost sm'; dl.href = `/api/backups/download?server=${encodeURIComponent(currentServer)}&name=${encodeURIComponent(b.name)}`; dl.textContent = 'Baixar';
    const rs = document.createElement('button'); rs.className = 'ghost sm danger'; rs.textContent = 'Restaurar';
    rs.onclick = () => restoreBackup(b.name);
    const act = document.createElement('td');
    act.append(dl, ' ', rs);
    tr.append(Object.assign(document.createElement('td'), { textContent: b.name }), Object.assign(document.createElement('td'), { textContent: b.sizeMB + ' MB' }), Object.assign(document.createElement('td'), { textContent: new Date(b.mtime).toLocaleString('pt-BR') }), act);
    body.append(tr);
  });
  if (!(data.backups || []).length) body.innerHTML = '<tr><td colspan="4" class="muted">Nenhum backup ainda.</td></tr>';
}
async function restoreBackup(name) {
  const ok = await confirmDialog(`Restaurar "${name}"? O mundo atual será sobrescrito pelos arquivos do backup. Recomenda-se gerar um backup antes. O servidor precisa estar parado.`, { okText: 'Restaurar', danger: true });
  if (!ok) return;
  doRestore(name);
}
async function doRestore(name) {
  $('#bkMsg').textContent = 'restaurando…';
  const { ok, error } = await api('/api/backups/restore', { method: 'POST', body: JSON.stringify({ name }) });
  $('#bkMsg').textContent = ok ? '✓ restaurado' : (error || 'erro ao restaurar');
  setTimeout(() => $('#bkMsg').textContent = '', 5000);
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
  ['#searchPager', '#searchPagerTop'].forEach(s => { const x = $(s); if (x) x.innerHTML = ''; });
  const qs = `q=${encodeURIComponent(q)}&sort=${encodeURIComponent(sort)}&category=${encodeURIComponent(activeCat)}&offset=${off}`;
  const { ok, data } = await api('/api/content/search?' + qs);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results || !data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; return; }
  box.innerHTML = ''; data.results.forEach(r => box.append(storeCard(r)));
  renderPager(data.total || 0, data.offset || 0, data.limit || 24);
}
function renderPager(total, offset, limit) {
  const go = (o) => { doSearch(o); $('#searchResults').scrollIntoView({ block: 'nearest' }); };
  renderPagerInto('#searchPagerTop', total, offset, limit, go);
  renderPagerInto('#searchPager', total, offset, limit, go);
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
let installedItems = [];
function renderInstalledList() {
  const box = $('#installedList'); box.innerHTML = '';
  const q = ($('#instFilter') || {}).value || '';
  const query = q.trim().toLowerCase();
  const list = query
    ? installedItems.filter(it => (it.title || '').toLowerCase().includes(query) || (it.slug || '').toLowerCase().includes(query))
    : installedItems;
  if (!list.length) {
    box.innerHTML = query
      ? `<div class="muted small">nada instalado com "${esc(query)}"</div>`
      : '<div class="muted small">nenhum mod/plugin instalado ainda.</div>';
    return;
  }
  list.forEach(it => {
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
async function loadInstalled() {
  const { data } = await api('/api/content/installed');
  installedItems = (data && data.items) || [];
  renderInstalledList();
}
$('#instFilter').addEventListener('input', renderInstalledList);
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
$('#authBtn').addEventListener('click', async () => {
  const btn = $('#authBtn'), msg = $('#authMsg');
  btn.disabled = true; msg.textContent = 'instalando o plugin de login…';
  const { ok, data } = await api('/api/compat/auth', { method: 'POST' });
  msg.textContent = ok
    ? `✓ login ativado (${data.installed.join(', ')}), servidor em offline — reinicie. Pirata: /register e /login. Premium: autologin.`
    : ('erro: ' + (data.error || ''));
  btn.disabled = false; loadInstalled();
  if (ok) $('#offlineToggle') && ($('#offlineToggle').checked = true);
});

// ---- bindings multi-servidor ----
$('#serverSelect').addEventListener('change', (e) => switchServer(e.target.value));
$('#serverManage').addEventListener('click', () => { $('#srvOverlay').hidden = false; renderServerManager(); });
$('#serversManage').addEventListener('click', () => { $('#srvOverlay').hidden = false; renderServerManager(); });
const ts = $('#tabServers'); if (ts) ts.addEventListener('click', loadServers);
$('#srvClose').addEventListener('click', () => $('#srvOverlay').hidden = true);
$('#srvOverlay').addEventListener('click', (e) => { if (e.target === $('#srvOverlay')) $('#srvOverlay').hidden = true; });
// ---- tela de carregamento (install de servidor/modpack) ----
let loadingPollTimer = null, loadingStart = 0;
function fmtElapsed(ms) { const s = Math.floor(ms / 1000); return s < 60 ? s + 's' : Math.floor(s / 60) + 'm ' + (s % 60) + 's'; }
function showLoading(title, poll) {
  loadingStart = Date.now();
  $('#loadingTitle').textContent = title || 'Instalando…';
  $('#loadingPhase').textContent = 'Preparando…';
  $('#loadingMeta').textContent = 'iniciando…';
  const fill = $('#loadingFill'); fill.style.width = '0%'; fill.classList.add('indeterminate');
  $('#loadingOverlay').hidden = false;
  clearInterval(loadingPollTimer); loadingPollTimer = null;
  if (poll) { loadingPollTimer = setInterval(pollProgress, 800); pollProgress(); }
  else { tickElapsed(); loadingPollTimer = setInterval(tickElapsed, 1000); }
}
function hideLoading() { clearInterval(loadingPollTimer); loadingPollTimer = null; $('#loadingOverlay').hidden = true; }
function tickElapsed() { $('#loadingMeta').textContent = 'tempo: ' + fmtElapsed(Date.now() - loadingStart); }
async function pollProgress() {
  const { ok, data } = await api('/api/servers/create-progress');
  const elapsed = Date.now() - loadingStart;
  if (!ok || !data || !data.phase) { tickElapsed(); return; }
  $('#loadingPhase').textContent = data.phase;
  const fill = $('#loadingFill');
  if (data.total > 0) {
    fill.classList.remove('indeterminate');
    const pc = Math.min(100, Math.round(data.done / data.total * 100));
    fill.style.width = pc + '%';
    $('#loadingMeta').textContent = `${data.done} / ${data.total} mods · ${pc}% · ${fmtElapsed(elapsed)}`;
  } else {
    fill.classList.add('indeterminate');
    $('#loadingMeta').textContent = 'tempo: ' + fmtElapsed(elapsed);
  }
}

$('#srvCreateForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const name = $('#srvName').value.trim(); if (!name) return;
  const loader = $('#srvLoader').value, version = $('#srvVersion').value.trim();
  const btn = $('#srvCreateBtn'); btn.disabled = true; btn.textContent = 'criando…';
  showLoading('Criando "' + name + '"…', false);
  const r = await api('/api/servers/create', { method: 'POST', body: JSON.stringify({ name, loader, version }) });
  hideLoading();
  btn.disabled = false; btn.textContent = 'Criar servidor';
  if (!r.ok) { toast(r.data.error || 'falha ao criar', 'err'); return; }
  $('#srvName').value = ''; $('#srvVersion').value = '';
  toast('Servidor criado: ' + r.data.server.name + ' (porta ' + r.data.server.port + ')');
  await loadServers(); renderServerManager();
});

// ---- bindings modpack ----
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
    const btn = ev.currentTarget; // capturar antes do await (depois vira null)
    const { ok, data } = await api('/api/modpacks/versions?slug=' + encodeURIComponent(r.slug));
    const versions = (ok && data.versions) || [];
    if (versions.length <= 1) {
      if (!await confirmDialog(`Criar um servidor a partir de "${r.title}"?\n\nBaixa todos os mods do pack (de segundos a alguns minutos, dependendo do tamanho).`, { okText: 'Criar' })) return;
      return doMpCreate(r, versions[0] && versions[0].id, '', btn);
    }
    const pick = await mpPickVersion(r, versions);
    if (!pick) return; // cancelado
    doMpCreate(r, pick.version.id, pick.name, btn);
  };
  return el;
}

// seletor de versão do modpack (reusa o modal de versões da loja de mods)
function mpPickVersion(r, versions) {
  return new Promise((resolve) => {
    const ov = $('#modOverlay'), body = $('#modBody');
    const def = versions.find(v => v.versionType === 'release') || versions[0];
    const groups = {};
    versions.forEach(v => { const mc = (v.gameVersions && v.gameVersions[0]) || '?'; (groups[mc] = groups[mc] || []).push(v); });
    const opts = Object.keys(groups).sort().reverse().map(mc =>
      `<optgroup label="MC ${esc(mc)}">${groups[mc].map(v =>
        `<option value="${esc(v.id)}">${esc(v.versionNumber)}${v.loaders && v.loaders[0] ? ' · ' + esc(v.loaders[0]) : ''} · ${esc(v.versionType)}${v.datePublished ? ' · ' + esc(v.datePublished.slice(0, 10)) : ''}</option>`).join('')}</optgroup>`).join('');
    body.innerHTML = `
      <div class="mod-head">
        ${iconHtml(r.icon, r.title)}
        <div style="min-width:0">
          <div class="mod-title">${esc(r.title)}</div>
          <div class="muted small">⬇ ${fmtNum(r.downloads)}${r.author ? ' · por ' + esc(r.author) : ''}</div>
        </div>
      </div>
      <p class="muted small">${esc((r.description || '').slice(0, 180))}</p>
      <div class="ver-row">
        <label class="muted small" style="flex:1;display:flex;flex-direction:column;gap:.3rem">
          <span>Escolha a versão do modpack</span>
          <select id="mpVerSel">${opts || '<option value="">— sem versões —</option>'}</select>
        </label>
      </div>
      <input type="text" id="mpNameModal" placeholder="Nome do servidor (opcional — usa o nome do pack)" style="width:100%;margin-top:.9rem">
      <div id="mpVerInfo" class="muted small" style="margin-top:.5rem"></div>
      <div class="ver-row" style="margin-top:1rem">
        <button class="ghost" id="mpVerCancel" style="flex:1">Cancelar</button>
        <button class="ok" id="mpVerCreate">Criar servidor</button>
      </div>`;
    ov.hidden = false;
    const sel = $('#mpVerSel'), info = $('#mpVerInfo');
    const describe = (v) => {
      if (!v) { info.textContent = ''; return; }
      const bits = [];
      if (v.loaders && v.loaders.length) bits.push('loader: ' + v.loaders.join(', '));
      if (v.gameVersions && v.gameVersions.length) bits.push('MC ' + v.gameVersions.join(', '));
      if (v.versionType) bits.push(v.versionType);
      if (v.datePublished) bits.push(v.datePublished.slice(0, 10));
      info.textContent = bits.join(' · ');
    };
    if (sel) { sel.value = def.id; describe(versions.find(v => v.id === sel.value) || def); sel.onchange = () => describe(versions.find(v => v.id === sel.value)); }
    const done = (v) => { ov.hidden = true; body.innerHTML = ''; document.removeEventListener('keydown', onKey); ov.removeEventListener('click', onClick); resolve(v); };
    const onKey = (e) => { if (e.key === 'Escape') done(null); };
    const onClick = (e) => { if (e.target === ov) done(null); };
    document.addEventListener('keydown', onKey);
    ov.addEventListener('click', onClick);
    $('#mpVerCancel').onclick = () => done(null);
    $('#mpVerCreate').onclick = () => done({ version: versions.find(v => v.id === sel.value), name: $('#mpNameModal').value.trim() });
  });
}

async function doMpCreate(r, versionId, name, btn) {
  btn.disabled = true; btn.textContent = 'instalando…';
  showLoading('Instalando "' + r.title + '"…', true);
  const { ok, data } = await api('/api/servers/create-modpack', { method: 'POST', body: JSON.stringify({ slug: r.slug, name, versionId }) });
  hideLoading();
  btn.disabled = false; btn.textContent = 'Criar servidor';
  if (!ok) { toast(data.error || 'falha ao criar', 'err'); return; }
  const strip = (data.server.strippedMods || []).length;
  toast('Modpack instalado: ' + data.server.name + ' — porta ' + data.server.port + (strip ? ` · ${strip} mods client-only desativados` : ''));
  await loadServers(); renderServerManager();
  const st = $('#tabServers');
  if (st && !st.hidden) st.click();
  else { const pi = document.querySelector('.side-nav .tab[data-tab="painel"]'); if (pi) pi.click(); }
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
  ['#mpPager', '#mpPagerTop'].forEach(s => { const x = $(s); if (x) x.innerHTML = ''; });
  const info = $('#mpInfo'); if (info) info.textContent = '—';
  const { ok, data } = await api(`/api/modpacks/search?q=${encodeURIComponent(q)}&loader=${encodeURIComponent(loader)}&offset=${off}`);
  if (!ok) { box.innerHTML = `<div class="err small" style="padding:1rem">${esc(data.error || 'erro')}</div>`; return; }
  if (!data.results || !data.results.length) { box.innerHTML = '<div class="muted small" style="padding:1rem">nada encontrado</div>'; if (info) info.textContent = '0'; return; }
  if (info) info.textContent = `${data.total || 0} resultado(s)`;
  box.innerHTML = ''; data.results.forEach(r => box.append(mpCard(r)));
  renderPagerInto('#mpPagerTop', data.total || 0, data.offset || 0, data.limit || 24, doMpSearch);
  renderPagerInto('#mpPager', data.total || 0, data.offset || 0, data.limit || 24, doMpSearch);
}
$('#mpSearchForm').addEventListener('submit', (e) => { e.preventDefault(); doMpSearch(0); });
$('#mpLoader').addEventListener('change', () => doMpSearch(0));

// ---- integrações / rede (playit / tailscale / cloudflare) ----
function intgPill(on) { return `<span class="status-pill"><span class="dot ${on ? 'on' : 'idle'}"></span>${on ? 'ativo' : 'parado'}</span>`; }
async function intgAction(name, action) { return api('/api/integrations/' + action, { method: 'POST', body: JSON.stringify({ name }) }); }
async function loadIntegrations() { const { ok, data } = await api('/api/integrations'); if (ok) renderIntegrations(data); }
function renderIntegrations(d) {
  const box = $('#intgList'); if (!box) return; box.innerHTML = '';
  const srv = d.server;
  if (srv) {
    const note = document.createElement('div');
    note.className = 'muted small';
    note.style.cssText = 'margin:.2rem 0 .6rem';
    note.textContent = `Túneis configurados para o servidor ${srv.name || srv.id}${srv.port ? ' (porta ' + srv.port + ')' : ''}. Ao trocar de servidor, o status e o endereço abaixo passam a ser do servidor selecionado.`;
    box.append(note);
  }

  // ---- playit ----
  {
    const p = d.playit, el = document.createElement('div'); el.className = 'intg';
    let body = '';
    if (!p.installed) body = `<button class="ok sm act" data-a="install">Instalar</button>`;
    else if (!p.hasSecret) {
      body = `<div class="intg-note">Crie um agente em <a href="https://playit.gg" target="_blank" rel="noopener">playit.gg</a> (Account → Agents → <b>self-managed</b>), copie o <b>secret key</b> e cole aqui (fica salvo só pra este servidor):</div>
        <div class="row" style="margin-top:.5rem"><input type="password" class="pl-secret" placeholder="secret key do playit.gg" style="flex:1"><button class="ghost sm act" data-a="secret">Salvar</button></div>`;
    } else if (!p.running) body = `<button class="ok sm act" data-a="start">Ligar túnel</button>`;
    else {
      if (p.address) body += `<div class="intg-note ok">Endereço público: <code>${esc(p.address)}</code> — é esse que os amigos usam no Minecraft (porta no painel do playit.gg).</div>`;
      else body += `<div class="muted small">túnel no ar. Configure a porta no painel do playit.gg; o endereço aparece aqui (clique em Atualizar).</div>`;
      body += `<div class="row" style="margin-top:.6rem"><button class="danger sm act" data-a="stop">Desligar</button></div>`;
    }
    body += `<label class="muted small" style="display:block;margin-top:.6rem"><input type="checkbox" class="pl-onstart"${p.onStart ? ' checked' : ''}> iniciar o túnel junto quando eu ligar este servidor</label>`;
    el.innerHTML = `<div class="intg-head"><div class="intg-ic"><img src="/logos/playit.svg" alt=""></div><div class="intg-meta"><div class="intg-title">playit.gg</div><div class="muted small">Servidor público sem abrir porta no roteador (túnel TCP). Ideal pra Minecraft. Este túnel é <b>deste servidor</b>.</div></div>${intgPill(p.running)}</div><div class="intg-body">${body}</div>`;
    const os = el.querySelector('.pl-onstart');
    if (os) os.onchange = async (e) => { const r = await api('/api/integrations/playit-conf', { method: 'POST', body: JSON.stringify({ onStart: e.currentTarget.checked }) }); toast(r.ok ? (e.currentTarget.checked ? 'Túnel ligará junto com o servidor.' : 'Túnel não ligará mais junto.') : (r.data.error || 'erro')); if (!r.ok) { e.currentTarget.checked = !e.currentTarget.checked; } };
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
    el.innerHTML = `<div class="intg-head"><div class="intg-ic"><img src="/logos/tailscale.svg" alt=""></div><div class="intg-meta"><div class="intg-title">Tailscale</div><div class="muted small">VPN privada: só quem você convidar acessa. Ótimo pra jogar entre amigos.</div></div>${intgPill(t.running)}</div><div class="intg-body">${body}</div>`;
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
    el.innerHTML = `<div class="intg-head"><div class="intg-ic"><img src="/logos/cloudflare.svg" alt=""></div><div class="intg-meta"><div class="intg-title">Cloudflare Tunnel</div><div class="muted small">Expõe o painel/serviço por um domínio, com túnel seguro.</div></div>${intgPill(c.running)}</div><div class="intg-body">${body}</div>`;
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

// ---- rede / wi-fi ----
async function loadNet() {
  const box = $('#netStatus'), wbox = $('#netWifi'); if (!box) return;
  const { ok, data } = await api('/api/net');
  if (!ok) { box.textContent = 'não consegui ler a rede.'; wbox.innerHTML = ''; return; }
  const parts = [data.ip ? `IP: <code>${esc(data.ip)}</code>` : 'sem IP'];
  if (data.ssid) parts.push(`Wi-Fi: <b>${esc(data.ssid)}</b>`);
  parts.push(`internet: ${esc(data.connectivity || '?')}`);
  box.innerHTML = parts.join(' · ');
  wbox.innerHTML = '';
  if (!data.hasWifi) { wbox.innerHTML = '<div class="muted small">Sem placa Wi-Fi detectada (provavelmente cabo). 👍</div>'; return; }
  const btn = document.createElement('button'); btn.className = 'ghost sm'; btn.textContent = '📶 Procurar redes Wi-Fi';
  btn.onclick = scanWifi; wbox.append(btn);
}
async function scanWifi() {
  const wbox = $('#netWifi'); wbox.innerHTML = '<div class="muted small" style="padding:.4rem 0">procurando redes…</div>';
  const { ok, data } = await api('/api/net/scan');
  if (!ok) { wbox.innerHTML = `<div class="err small">${esc(data.error || 'erro')}</div>`; return; }
  wbox.innerHTML = '';
  if (!data.networks || !data.networks.length) { wbox.innerHTML = '<div class="muted small">nenhuma rede encontrada.</div>'; return; }
  data.networks.forEach(n => {
    const el = document.createElement('div'); el.className = 'inst-row';
    const locked = /wpa|wep|802/i.test(n.security);
    el.innerHTML = `<div class="inst-meta"><div class="inst-title">${esc(n.ssid)} ${locked ? '🔒' : ''}</div><div class="muted small">sinal ${n.signal}% · ${esc(n.security)}</div></div>`;
    const act = document.createElement('div'); act.className = 'inst-actions';
    const b = document.createElement('button'); b.className = 'ok sm'; b.textContent = 'Conectar';
    b.onclick = async () => {
      let pass = '';
      if (locked) { pass = await promptDialog(`Senha do Wi-Fi "${n.ssid}":`); if (pass === null) return; }
      b.disabled = true; b.textContent = 'conectando…';
      const r = await api('/api/net/wifi', { method: 'POST', body: JSON.stringify({ ssid: n.ssid, password: pass }) });
      if (r.ok) { toast('Conectado a ' + n.ssid + '.'); loadNet(); }
      else { b.disabled = false; b.textContent = 'Conectar'; toast(r.data.error || 'falha ao conectar', 'err'); }
    };
    act.append(b); el.append(act); wbox.append(el);
  });
}
$('#netReload').addEventListener('click', loadNet);

// ---- diagnóstico: alcance dos servidores de login da Mojang ----
async function runMojangDiag() {
  const btn = $('#mojangDiagBtn'), box = $('#mojangDiag'); if (!box) return;
  btn.disabled = true; box.innerHTML = '<div class="muted small" style="margin-top:.5rem">testando… (até ~6s)</div>';
  const { ok, data } = await api('/api/diag/mojang');
  btn.disabled = false;
  if (!ok) { box.innerHTML = `<div class="err small" style="margin-top:.5rem">${esc(data.error || 'erro')}</div>`; return; }
  const rows = (data.targets || []).map(t => {
    const host = (() => { try { return new URL(t.url).host; } catch { return t.name; } })();
    const res = t.ok ? `alcançável · HTTP ${t.status} · ${t.ms} ms` : `falhou · ${esc(t.error || '?')} · ${t.ms} ms`;
    return `<div class="intg-note ${t.ok ? 'ok' : 'warn'}">${t.ok ? '✓' : '✗'} <code>${esc(host)}</code> — ${res}</div>`;
  }).join('');
  let tip;
  if (data.ok) tip = 'Tudo alcançável. Se ainda aparecer "Authentication servers are down", pode ser instabilidade momentânea da própria Mojang — tente de novo.';
  else tip = `O painel/servidor não consegue falar com a Mojang: o problema é a <b>rede, o DNS ou o Docker deste host</b> (ex.: disco cheio, Docker travado, sem internet) — <b>não</b> a conta do jogador.${data.onlineMode === false ? ' (Este servidor está em <code>online-mode=false</code>, então o login não depende disso.)' : ' Com <code>online-mode=true</code> ninguém consegue entrar enquanto isso falhar.'}`;
  box.innerHTML = `<div style="margin-top:.5rem">${rows}</div><div class="muted small" style="margin-top:.4rem">${tip}</div>`;
}
$('#mojangDiagBtn').addEventListener('click', runMojangDiag);

// ---- detalhes extras (peso do mundo, velocidade) ----
function fmtBytes(n) {
  n = n || 0;
  if (n >= 1073741824) return (n / 1073741824).toFixed(2) + ' GB';
  if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(0) + ' KB';
  return n + ' B';
}
async function measureWorld() {
  const el = $('#worldSize'), meta = $('#worldMeta'); if (!el) return;
  el.textContent = '…'; meta.textContent = 'medindo…';
  const { ok, data } = await api('/api/worldsize');
  if (!ok) { el.textContent = '—'; meta.textContent = 'não consegui medir'; return; }
  el.textContent = fmtBytes(data.bytes);
  meta.textContent = data.dirs && data.dirs.length ? data.dirs.join(', ') : 'sem mundo ainda';
}
let speedBusy = false;
async function runSpeed() {
  if (speedBusy) return;
  speedBusy = true;
  const el = $('#netSpeed'), meta = $('#speedMeta');
  el.textContent = '…'; meta.textContent = 'testando…';
  const { ok, data } = await api('/api/speedtest', { method: 'POST' });
  speedBusy = false;
  if (!ok) { el.textContent = '—'; meta.textContent = data.error || 'falhou · clique pra tentar'; return; }
  el.textContent = data.mbps;
  meta.textContent = `${fmtBytes(data.bytes)} em ${data.secs}s · clique pra refazer`;
}
$('#worldCard').addEventListener('click', measureWorld);
$('#speedCard').addEventListener('click', runSpeed);

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
