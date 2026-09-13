#!/usr/bin/env node
'use strict';
/*
 * init do container: garante o /data/config.json (cria ou atualiza campos de
 * caminho do painel), aplica a senha inicial (CRAFTBOX_PASSWORD) e semeia os
 * binários de túnel no integrationsDir. Roda antes do server.js no entrypoint.
 */
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const cfgPath = process.env.CRAFTBOX_PANEL_CONFIG || '/data/config.json';
const serversDir = process.env.CRAFTBOX_SERVERS_DIR || '/data/servers';
const integrationsDir = process.env.CRAFTBOX_INTEGRATIONS_DIR || '/data/craftbox-integrations';
const runDir = process.env.CRAFTBOX_RUN_DIR || '/data/craftbox-run';
const port = parseInt(process.env.CRAFTBOX_PORT || '8080', 10);
const password = process.env.CRAFTBOX_PASSWORD || '';
const tunnelSeed = '/usr/local/lib/craftbox/tunnels';

function ensureDir(d) { try { fs.mkdirSync(d, { recursive: true }); } catch {} }

[serversDir, integrationsDir, runDir, path.dirname(cfgPath)].forEach(ensureDir);

let cfg = {};
let existed = true;
try { cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8')); } catch { existed = false; }

cfg.port = port;
cfg.host = '0.0.0.0';
cfg.serversDir = serversDir;
cfg.integrationsDir = integrationsDir;
cfg.runDir = runDir;
cfg.serviceTemplate = cfg.serviceTemplate || 'minecraft@';
if (!cfg.sessionSecret) cfg.sessionSecret = crypto.randomBytes(32).toString('hex');
if (!cfg.auth) cfg.auth = { salt: '', hash: '' };

// senha inicial: só aplica se ainda não houver senha configurada
if (!cfg.auth.hash && password) {
  const salt = crypto.randomBytes(16).toString('hex');
  cfg.auth = { salt, hash: crypto.scryptSync(password, salt, 64).toString('hex') };
}

try { fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n'); } catch (e) { console.error('[init] ERRO ao gravar', cfgPath, e.message); process.exit(1); }

// semeia playit/cloudflared no integrationsDir (o painel os detecta ali)
try {
  for (const bin of fs.readdirSync(tunnelSeed)) {
    const target = path.join(integrationsDir, bin);
    if (!fs.existsSync(target)) fs.copyFileSync(path.join(tunnelSeed, bin), target);
    fs.chmodSync(target, 0o755);
  }
} catch { /* seeder opcional */ }

console.log(`[init] config.json: ${cfgPath} ${existed ? '(atualizado)' : '(criado)'}`);
console.log(`[init] servidores: ${serversDir} | integrações: ${integrationsDir} | runner: exec`);
console.log(cfg.auth.hash ? '[init] senha do painel configurada' : '[init] ATENÇÃO: defina CRAFTBOX_PASSWORD na primeira subida p/ criar a senha');