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
- [x] **Forge/NeoForge no "Novo servidor"** (sem precisar de modpack): resolve a build (promotions do Forge / maven do NeoForge), roda o instalador com a JDK certa. Validado: Forge 65.1.0 (MC 26.2) e NeoForge 21.1.251 (MC 1.21.1) sobem até o "Done". ✅
- [ ] **Modpacks do CurseForge** — implementado (busca, versões, manifest.json, fallback por sha1 no Modrinth, lista de mods pra baixar à mão); falta validar de ponta a ponta com uma chave de API real.
- [ ] Testar **Forge/NeoForge** de verdade (modpack) — Forge 1.20.1 valida instalação+boot com Java 25 (VM/demo); falta validar boot completo de um pack pesado.
- [x] **Pré-release `v1.0.0-rc1`** cortada — CI publica a ISO nos Releases (marcada como pre-release). ✅
- [ ] Depois do RC: validar o RC final numa VM (painel no boot + tela de boas-vindas), colher feedback e soltar a `v1.0.0` estável.

## Ideias / futuro
- [x] **Servidor Pumpkin** (Rust, binário único sem Java) como tipo **experimental** na criação de servidor ✅ — baixa o binário da release oficial, configura porta/MOTD/RCON via `pumpkin.toml` (config parcial), sobe em ~4s; RCON e Bedrock nativos funcionam pelo painel. Validado no demo.
  - Ressalvas: usa config próprio (o editor de `server.properties` e a loja de mods do painel não se aplicam); ignora SIGTERM (o "Desligar" espera o timeout do systemd antes do SIGKILL); é beta (0.1.0-dev). Alternativa parecida: SteelMC.
- [x] Stop do Pumpkin e UI adaptada ✅ — o Pumpkin atual já sai limpo com SIGTERM/SIGINT (~2 s, testado); com um servidor Pumpkin selecionado o painel esconde a loja, a aba Compatibilidade e o editor de `server.properties` (mostra um aviso sobre `pumpkin.toml`).
- [x] Reescrever o **backend do painel em Rust** ✅ (feito: `panel-rs`, binário único na ISO e no Docker desde a rc5).

## Fase final — otimização (o "endgame", é difícil e tudo bem)
- [x] **Reescrever o backend em Rust** ✅ — feito em `panel-rs/` (paridade com o Node validada; o Node ficou só como fallback). Objetivo: binário único, sem runtime, footprint menor de RAM/CPU — importante num notebook fraco.
  - Mesmas rotas/API do `server.js` atual (o frontend HTML/CSS/JS pode continuar igual).
  - Ideias de crates: um HTTP mínimo (ex: `tiny_http`/`axum`), TLS/HTTP client pra Modrinth/Fabric/Paper, `zip` pra `.mrpack`, RCON manual sobre TCP.
- [ ] **Compilar na instalação / no primeiro boot** — o appliance compila o backend na hora (ISO instala a toolchain Rust e roda `cargo build --release` com `RUSTFLAGS="-C target-cpu=native"`), gerando um binário otimizado pro CPU específico daquela máquina. Cair pra binário pré-compilado se a compilação falhar.

## Pendências abertas
- [x] **Escolher a versão do modpack ao criar** — se o pack tiver mais de 1 versão, abre um seletor (agrupado por MC, com loader/tipo/data) antes de criar; com 1 versão mantém o fluxo antigo. ✅
- [x] **Botão "Desligar tudo" (parada de emergência)** ✅ — na aba Servidores; `POST /api/servers/stop-all` para toda instância rodando (`stop` pelo RCON quando responde, senão `systemctl stop --no-block`). Se algo ainda estiver de pé depois de 3 min, oferece **forçar** (`{force:true}`: enfileira o stop e dá SIGKILL, pra o `Restart=on-failure` não religar). Sudoers ganhou `systemctl kill --signal=SIGKILL minecraft@*`.
- [x] **Fidelidade do status no Painel inicial** ✅ — `/api/status` e `/api/servers/<id>/status` só dizem `active` com o RCON respondendo (sonda de 2 s); antes disso `activating` ("Iniciando…"), e passando de 15 min sem resposta `stalled` ("Sem resposta", típico de swap). Sem RCON habilitado usa o "Done (" do `logs/latest.log`. Parando aparece "Desligando…". Validado: Fabric 1.20.1 ficou "Iniciando…" até o Done (66 s) e um processo congelado com SIGSTOP deixou de aparecer como "No ar".
- [~] **RAM 4 GB insuficiente pro servidor** (software feito; falta a RAM): subir o Paper (heap ~1,7 GB = RAM−2 GB) + painel + OS joga o notebook de 4 GB em **swap thrashing** no HD 5400rpm — a máquina inteira trava (até o SSH cai). Piora com `Restart=on-failure` (OOM → reinicia → trava de novo). Ações: (a) heap default menor / mais folga pra ≤4 GB; (b) `MemoryMax`/`MemoryHigh` no unit pra proteger o OS; (c) o real: **RAM 8 GB** (já no plano). Achado ao ligar `minecraft@axos-pudding` no craftbox real (13/09).
  - ✅ (a)+(b) feitos no `mc-install`: com ≤4,5 GB o heap do 1º servidor é 35% da RAM e `-Xms512M` (antes RAM−2 GB com Xms=Xmx); a unit tem `MemoryMax=90%` + `MemorySwapMax=1G` (o OOM mata só o Minecraft) e `StartLimitBurst=3` em 10 min (não fica em loop de OOM). Falta (c).
- [x] **`SuccessExitStatus=143 SIGTERM`** no `minecraft@.service` — parar o servidor saía como `failed` (JVM sai 143 no SIGTERM). Corrigido no mc-install. ✅
- [x] **Diagnóstico "Authentication servers are down"** — botão **Testar conexão com a Mojang** no painel (`GET /api/diag/mojang`), que testa se o servidor alcança `sessionserver.mojang.com`/`api.minecraftservices.com`. Achado num craftbox em Docker Desktop (Windows): jogadores premium válidos eram barrados com a Mojang no ar; a causa era o `C:` do host com <500 MB livres, que travou o Docker Desktop. ✅
- [x] **Aviso de disco baixo** no Painel (<2 GB livres). ✅
- [x] **Disco livre usava `bfree` em vez de `bavail`** — contava os blocos reservados pro root como livres; agora mostra o que o processo pode usar de fato. ✅
- [ ] **Detectar o disco do host no Docker Desktop** — limitação conhecida: dentro do container o painel vê o VHDX do WSL (cresce sob demanda), não o `C:` do Windows, então o aviso de disco baixo não pega o caso real. Ideia: montar um caminho do host ou pedir o valor via env/config.

- [x] **Java certo por versão do MC fora do Docker** ✅ — na ISO o painel sempre usava `/usr/bin/java` (26) e servidores 1.20.1 quebravam. Agora o `start.sh` escolhe `/usr/lib/jvm/java-N-openjdk` pela versão (exata, senão a menor acima, senão `/usr/bin/java`; no Docker o wrapper segue pela `CRAFTBOX_JAVA_MAJOR`), os instaladores Forge/NeoForge/Quilt usam a mesma escolha e o `mc-install` instala `jre8/17/21` além da mais nova (que continua sendo o `java` padrão). Servidores criados antes mantêm o `start.sh` antigo.
- [x] **Parada que não corrompe o mundo** ✅ — medido: salvar um Fabric 1.20.1 recém-gerado no HD levou 220 s (o Cursed Walking, 183 s); o `TimeoutStopSec=90` dava SIGKILL/SIGABRT no meio. Agora `TimeoutStopSec=600`, o modo Docker/exec espera até 10 min (`stop_grace_period: 10m`), o Desligar usa `stop` pelo RCON (o log mostra o salvamento) e o `systemctl stop` vai com `--no-block` (antes estourava o timeout de 15 s do painel).

- [x] **Login pirata no Paper quebrado pelo AuthMe 6** ✅ — a release 6.0.1 traz um .jar por plataforma e o painel baixava o primeiro (`AuthMe-6.0.1-Bungee.jar`, módulo de proxy que não carrega no Paper). Agora pega o `-Paper.jar`. Forge/NeoForge/Quilt recusam com mensagem (antes o AuthMe ia parar em `mods/`). Validado com um cliente offline (mineflayer) em Paper 1.21.11 (register/login/senha errada expulsa) e Fabric 1.21.11 (EasyAuth). No Paper 1.21.11+ o AuthMe 6 pede a senha numa **tela antes de entrar** (dialog), não pelo chat.

- [x] **Atualização da "distro" pelo GitHub** ✅ — `craftbox-update` + card **Atualização do craftbox** no painel (`GET/POST /api/system/update`, `GET /api/system/update/log`). O CI publica `craftbox-system-<tag>.tar.gz(.sha256)` em cada release; o updater confere o sha256, faz backup, aplica painel/scripts/units/sudoers (fonte única `usr/local/lib/craftbox/system.sh`, usada também pelo `mc-install`), roda `pacman -Syu` + pacotes do craftbox e reinicia só o painel, com rollback se ele não responder. Validado na VM com release falsa servida localmente: bootstrap de máquina antiga, update pelo painel, rollback de painel quebrado e recusa de sha256 errado.
  - [x] Primeira release real com o pacote (`rc7`, depois `rc8`) ✅
  - [ ] Rodar o bootstrap no PC da 3060 (estava offline).

- [x] **Instalar sem criar servidor** ✅ — a tela "Tipo de servidor" do instalador tem **Nenhum agora** (padrão): instala só sistema + painel, sem baixar servidor nem gerar mundo, e pula versão/detalhes/dificuldade/EULA. No 1º boot o painel mostra "Nenhum servidor ainda" com o botão **Criar o primeiro servidor** e esconde loja/compat/console/backups; as rotas que agem num servidor respondem 409 até existir um (antes caía num id fantasma `default`). Validado no `--dry-run` e no painel com `serversDir` vazio; falta uma instalação real em VM.

## Hardware (tarefas físicas)
- [ ] Trocar a bateria **CR2032** (código de 5 beeps / RTC resetando pra 2013).
- [x] Resolver o **clock preso da CPU** ✅ (era BD PROCHOT; resolvido por software com `craftbox-bdprochot`) (carregador/bateria principal — a CPU está travada perto do mínimo).
- [ ] Comprar a **RAM 8 GB DDR3L** antes de usar de verdade.
