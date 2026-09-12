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
- [x] **Lista de servidores no painel** — aba "Servidores" com cards (status + Ligar/Desligar/Selecionar). ✅
- [x] **Paginação na busca de modpacks**. ✅
- [x] **Login de jogadores (pirata + premium)** em Compatibilidade — offline + plugin de auth (EasyAuth/Fabric, AuthMeReloaded/Paper): pirata usa `/register` `/login`, premium entra por autologin. Bloqueia se não houver versão compatível. ✅
- [x] **Detalhes extras no Painel** — cards de **peso do mundo** (soma das pastas `world*` da instância) e **velocidade da internet** (teste de download sob demanda via Cloudflare), com um **botão que mostra/oculta** esses cards (preferência salva no `config.json`). ✅
- [ ] **Terminal da máquina** no Console — pausado: o classificador de segurança marca como RCE; precisa liberar permissão pra concluir.
- [x] **Tela de carregamento** no install de servidor/modpack — overlay com spinner, fase atual (baixando pack → baixando mods X/Y → aplicando → desativando client → instalando loader → finalizando), barra de progresso real e tempo decorrido. Backend expõe `GET /api/servers/create-progress`. ✅
- [x] **Fallback do loader Fabric** — se o loader fixado pelo pack for velho demais pro endpoint `server/jar` (dava HTTP 400, ex: packs 1.16.5), cai pro loader mais recente daquele MC. ✅
- [ ] **Tema "gaming"** inspirado nas artes de Minecraft dashboard (visual).
- [x] **Logo** do craftbox 🎨 — arte pronta (`panel/public/logo.png`, box estilo mesa de trabalho + Tux, paleta marrom/creme). Aparece na sidebar, na tela de login e como favicon (`favicon.png` 64px). ✅

## Appliance real (notebook Dell) — pra funcionar fora do demo
- [x] **systemd template** `minecraft@.service` + `mc-backup@` + regra de **sudoers** (`systemctl … minecraft@*`) no `mc-install`/`setup.sh`. ✅
- [x] `mc-install` cria o 1º servidor em `serversDir` (`/srv/minecraft/<id>`, multi-servidor por padrão) e **embarca + configura o painel** (sobe no boot). ✅
- [x] **Validado numa VM (UEFI/QEMU)**: a ISO boota, o instalador roda todas as etapas, instala (Arch + Java 26 + nodejs + Paper), e no reboot `minecraft@principal` e `sshd` sobem sozinhos e o painel responde (`configured:true`). ✅
  - 🐞 **Bug pego e corrigido**: o serviço `craftbox-panel` não tinha `[Install] WantedBy=`, então não subia no boot (só ao iniciar na mão). Corrigido no `mc-install`. **Rebuild da ISO** recomendado pra levar o fix.
- [x] **`INSTALL.md`** — guia passo a passo com **prints reais da interface** (Ventoy + boot + instalador + pós-instalação). ✅
- [x] **Bug crítico do install de modpack corrigido**: o `run()` usava `execFile` com `maxBuffer` padrão (1 MB); o instalador do Forge cospe ~1,2 MB, então o Node matava o processo e o painel apagava a instância inteira — mesmo com o Forge instalado com sucesso. Agora `maxBuffer: 256 MB`. Downloads dos mods agora em **paralelo** (pool de 6). ✅
- [x] **Removedor de mods client-only** no install de modpack — muitos packs são singleplayer e trazem mods de render/UI (ETF, Oculus, Xaero's, CITResewn, etc.) que quebram um servidor dedicado (`invalid dist DEDICATED_SERVER`). Detecta por `fabric.mod.json` (`environment:client`, pega até mods Fabric via Sinytra Connector) + lista de nomes conhecidos; desativa renomeando pra `.disabled` (reversível). É best-effort — packs muito "de cliente" podem não subir headless. ✅
- [ ] Testar **Forge/NeoForge** de verdade (modpack) — Forge 1.20.1 valida instalação+boot com Java 25 (VM/demo); falta validar boot completo de um pack pesado.
- [x] **Pré-release `v1.0.0-rc1`** cortada — CI publica a ISO nos Releases (marcada como pre-release). ✅
- [ ] Depois do RC: validar o RC final numa VM (painel no boot + tela de boas-vindas), colher feedback e soltar a `v1.0.0` estável.

## Ideias / futuro
- [x] **Servidor Pumpkin** (Rust, binário único sem Java) como tipo **experimental** na criação de servidor ✅ — baixa o binário da release oficial, configura porta/MOTD/RCON via `pumpkin.toml` (config parcial), sobe em ~4s; RCON e Bedrock nativos funcionam pelo painel. Validado no demo.
  - Ressalvas: usa config próprio (o editor de `server.properties` e a loja de mods do painel não se aplicam); ignora SIGTERM (o "Desligar" espera o timeout do systemd antes do SIGKILL); é beta (0.1.0-dev). Alternativa parecida: SteelMC.
- [ ] Suavizar o stop do Pumpkin (KillSignal/kill mais rápido) e adaptar a UI (esconder loja/props pra pumpkin).
- [ ] Reescrever o **backend do painel em Rust** (endgame de otimização).

## Fase final — otimização (o "endgame", é difícil e tudo bem)
- [ ] **Reescrever o backend em Rust** (hoje é Node.js puro). Objetivo: binário único, sem runtime, footprint menor de RAM/CPU — importante num notebook fraco.
  - Mesmas rotas/API do `server.js` atual (o frontend HTML/CSS/JS pode continuar igual).
  - Ideias de crates: um HTTP mínimo (ex: `tiny_http`/`axum`), TLS/HTTP client pra Modrinth/Fabric/Paper, `zip` pra `.mrpack`, RCON manual sobre TCP.
- [ ] **Compilar na instalação / no primeiro boot** — o appliance compila o backend na hora (ISO instala a toolchain Rust e roda `cargo build --release` com `RUSTFLAGS="-C target-cpu=native"`), gerando um binário otimizado pro CPU específico daquela máquina. Cair pra binário pré-compilado se a compilação falhar.

## Hardware (tarefas físicas)
- [ ] Trocar a bateria **CR2032** (código de 5 beeps / RTC resetando pra 2013).
- [ ] Resolver o **clock preso da CPU** (carregador/bateria principal — a CPU está travada perto do mínimo).
- [ ] Comprar a **RAM 8 GB DDR3L** antes de usar de verdade.
