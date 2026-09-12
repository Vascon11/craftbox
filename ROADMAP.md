# Roadmap / Pendências — craftbox

Lista de coisas pra fazer depois. Marque com `[x]` conforme for concluindo.

## Painel — novas funcionalidades
- [x] **Mesclar Logs + Console** numa aba só (log ao vivo + campo de comando RCON juntos, tipo um terminal de servidor de verdade). ✅
- [x] **Página "Integrações / Rede"** — cards com status ao vivo + instalar/ligar, rodando como serviços `systemctl --user` (rootless): ✅
  - [x] **playit.gg** — servidor público sem abrir porta no roteador (túnel TCP); instala o binário + secret key do playit.gg.
  - [x] **Cloudflare Tunnel** — instala o `cloudflared` + token do túnel (ótimo pro painel via domínio).
  - [x] **Tailscale** — detecta, mostra IP/status e conecta/desconecta (VPN privada entre amigos). Controle rootless via `sudo -n` + provisão no `setup.sh`.
- [x] **Histórico de ações** (auditoria) dentro do Console: registra login/power/config/mods/integrações/backups com data, servidor e IP; com botão **Limpar**. ✅
- [x] **Paginação** na loja de mods/plugins (Anterior/Próxima + total). ✅
- [x] **Confirmações/avisos visuais** no tema do painel (modais e toasts no lugar dos popups do navegador). ✅
- [x] **Gerenciar acesso / usuários** (aba Configuração → Acesso): ✅
  - [x] Trocar a senha do painel pela própria interface.
  - [x] Contas de usuário (usuário/email + senha, papéis admin/user) — opt-in: sem contas usa senha única; ao criar a 1ª conta o login passa a pedir usuário+senha. Histórico registra quem fez cada ação.
- [ ] **Lista de servidores no painel** — uma visão/aba com cards de todas as instâncias (além do seletor no topo).
- [x] **Paginação na busca de modpacks**. ✅
- [ ] **Terminal da máquina** no Console — pausado: o classificador de segurança marca como RCE; precisa liberar permissão pra concluir.
- [ ] **Tema "gaming"** inspirado nas artes de Minecraft dashboard (visual).
- [ ] **Logo** do craftbox 🎨 — encaixe pronto no painel (basta soltar `panel/public/logo.png`); arte com um amigo.

## Appliance real (notebook Dell) — pra funcionar fora do demo
- [ ] Instalar no sistema o **systemd template** `minecraft@.service` + regra de **sudoers** (`systemctl start/stop/restart minecraft@*`).
- [ ] Ajustar `mc-install` (ISO) e `setup.sh` pra criar o 1º servidor já em `serversDir` (multi-servidor por padrão).
- [ ] Testar **Forge/NeoForge** de verdade (modpack) com uma versão de Java compatível.
- [ ] Soltar uma **release/tag** pra o CI publicar a ISO nova nos Releases.

## Fase final — otimização (o "endgame", é difícil e tudo bem)
- [ ] **Reescrever o backend em Rust** (hoje é Node.js puro). Objetivo: binário único, sem runtime, footprint menor de RAM/CPU — importante num notebook fraco.
  - Mesmas rotas/API do `server.js` atual (o frontend HTML/CSS/JS pode continuar igual).
  - Ideias de crates: um HTTP mínimo (ex: `tiny_http`/`axum`), TLS/HTTP client pra Modrinth/Fabric/Paper, `zip` pra `.mrpack`, RCON manual sobre TCP.
- [ ] **Compilar na instalação / no primeiro boot** — o appliance compila o backend na hora (ISO instala a toolchain Rust e roda `cargo build --release` com `RUSTFLAGS="-C target-cpu=native"`), gerando um binário otimizado pro CPU específico daquela máquina. Cair pra binário pré-compilado se a compilação falhar.

## Hardware (tarefas físicas)
- [ ] Trocar a bateria **CR2032** (código de 5 beeps / RTC resetando pra 2013).
- [ ] Resolver o **clock preso da CPU** (carregador/bateria principal — a CPU está travada perto do mínimo).
- [ ] Comprar a **RAM 8 GB DDR3L** antes de usar de verdade.
