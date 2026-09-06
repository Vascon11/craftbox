# Roadmap / Pendências — craftbox

Lista de coisas pra fazer depois. Marque com `[x]` conforme for concluindo.

## Painel — novas funcionalidades
- [ ] **Mesclar Logs + Console** numa aba só (log ao vivo + campo de comando RCON juntos, tipo um terminal de servidor de verdade).
- [ ] **Página "Acesso / Rede"**:
  - [ ] **Domínio via Cloudflare** — apontar um domínio pro servidor (Cloudflare Tunnel / DNS) com autenticação, sem abrir porta no roteador.
  - [ ] **VPN (Tailscale)** — mostrar status, ligar/desligar e gerar o endereço de acesso pros amigos entrarem com segurança.
- [ ] **Gerenciar acesso / usuários**:
  - [ ] Trocar a senha do painel pela própria interface (hoje só via `node server.js --hash`).
  - [ ] Contas de usuário (email + senha) pra controlar quem acessa o painel.
- [ ] **Lista de servidores no painel** — uma visão/aba com cards de todas as instâncias (além do seletor no topo).
- [ ] **Logo** do craftbox. 🎨

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
