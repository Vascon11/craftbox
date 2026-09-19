<p align="center">
  <img src="panel/public/logo.png" alt="craftbox" width="160">
</p>

<h1 align="center">craftbox</h1>

<p align="center"><b>Uma mini-distro Linux que transforma um notebook velho num servidor de Minecraft dedicado.</b></p>

`craftbox` é um *spin* enxuto do [Arch Linux](https://archlinux.org): você grava a ISO num pendrive, dá boot, responde algumas perguntas, e no fim tem uma máquina headless que existe pra uma coisa só — rodar um servidor de Minecraft (Paper, Fabric ou o experimental Pumpkin) que sobe sozinho no boot.

Sem ambiente gráfico, sem peso: toda a RAM sobra pro jogo.

![Build ISO](https://github.com/Vascon11/craftbox/actions/workflows/build-iso.yml/badge.svg)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)

---

## Interface — painel web `craftbox-panel`

O craftbox vem com um **painel web** (**Node.js puro, zero dependências**) pra gerenciar tudo pelo navegador, com cara de painel de hosting profissional e login por senha. Veja [`panel/`](panel/).

![Painel — dashboard](screenshots/painel-dashboard.jpg)

- 🖥️ **Multi-servidor** — crie/clone/apague vários servidores (**Paper**, **Fabric** ou **Pumpkin** — servidor em Rust, experimental), cada um com loader, versão e porta próprios; troca entre eles por um seletor. Filosofia "um rodando por vez" pra hardware fraco.
- 📊 **Painel** — status (no ar/desligado), jogadores online (RCON) e stats em tempo real: frequência de CPU, RAM, disco, temperatura, load, **peso do mundo** e **velocidade da internet** (sob demanda).
- ⏻ **Ligar / reiniciar / desligar** via systemd (ou `systemctl --user`, modo **rootless** sem sudo; no Docker, runner de processos diretos).
- 🧩 **Loja de mods/plugins** estilo Prism/Modrinth — busca no [Modrinth](https://modrinth.com) com ícones, categorias e ordenação, **escolha de versão**, **dependências automáticas** e gestão dos instalados (ativar/desativar, atualizar, remover).
- 📦 **Modpacks** — cria um servidor a partir de um modpack do **Modrinth** (`.mrpack`) ou do **CurseForge** (precisa de uma chave grátis da API, colada na própria aba ou via `CRAFTBOX_CURSEFORGE_KEY`) em **Fabric/Quilt/Forge/NeoForge**. Mods que o autor bloqueou pra download automático são procurados no Modrinth pelo hash; os que sobrarem aparecem listados com link pra baixar à mão. Além disso, baixa os mods **em paralelo** (com **tela de carregamento** e progresso), **desativa sozinho os mods client-only** que derrubariam um servidor dedicado, monta a instância e mostra o link do pack pros jogadores instalarem o mesmo no cliente.
- 🌐 **Integrações / Rede** — conecte o servidor sem abrir porta no roteador: **playit.gg** (túnel), **Cloudflare Tunnel** e **Tailscale** (VPN privada entre amigos), além de configurar **Wi-Fi** pelo próprio painel (`nmcli`).
- 🎮 **Compatibilidade** — **modo offline** ("pirata", com plugin de login por usuário/senha) e **Bedrock** (Geyser + Floodgate) em 1 clique.
- 🔐 **Acesso** — login por senha ou **contas de usuário** (papéis admin/user) e **histórico de ações**.
- ⚙️ **Configuração** — editor visual do `server.properties`; **Console** RCON; **Logs** ao vivo; **Backups** do mundo.

![Painel — loja de mods](screenshots/painel-conteudo.jpg)
![Painel — gerenciar servidores](screenshots/painel-servidores.jpg)
![Painel — criar a partir de modpack](screenshots/painel-modpack.jpg)

---

## Por que existe

Nasceu de um caso real: um **Dell Inspiron 3442** (Intel i5-4210U, 4–8 GB DDR3, HD 5400 rpm) parado numa gaveta. Em vez de virar lixo eletrônico, virou servidor de Minecraft pros amigos. O `craftbox` é o resultado empacotado pra qualquer notebook antigo do mesmo porte.

## O que a ISO faz por você

Ao dar boot, o instalador (`mc-install`) abre **automaticamente** e cuida de tudo:

- 🌐 **Rede** — detecta cabo (DHCP) ou pergunta o Wi-Fi e **guarda a conexão** pro sistema instalado (via NetworkManager)
- 💽 **Instalação automática** — particiona (UEFI/GPT), formata e instala um Arch mínimo no disco
- 🎮 **Escolha do servidor na hora** — **Paper** (plugins, leve) ou **Fabric** (mods, com mods de performance já incluídos)
- ⚡ **Modo otimizado ou limpo** — na instalação você escolhe entre já vir com otimizações pra hardware fraco (mods de performance + `view/simulation-distance` menores) ou um servidor limpo pra configurar do seu jeito
- 🖥️ **Painel web já embarcado** — o `craftbox-panel` vem instalado e sobe no boot em `http://<ip>:8080` (você define a senha na instalação)
- 👋 **Tela de boas-vindas** no console após instalar, mostrando o **IP real** e o endereço do painel (`http://<ip>:8080`) — você já sabe onde acessar
- 🗂️ **Multi-servidor de fábrica** — cada instância fica em `/srv/minecraft/<nome>` com seu serviço `minecraft@<nome>`; crie/troque servidores pelo painel
- 🧠 **Heap da JVM auto-dimensionado** pela RAM detectada + **[flags do Aikar](https://docs.papermc.io/paper/aikars-flags)** aplicadas
- ⚙️ **Otimizado pra CPU fraca** — `view-distance` e `simulation-distance` calibrados
- 🔒 **SSH** pronto (com sua chave pública, se você colar uma) + teclado ABNT2 e timezone BR
- 💻 **Fechar a tampa não desliga** (`HandleLidSwitch=ignore`) — o notebook vira um "servidor de tampa fechada"
- 💾 **Backup diário** do mundo por instância (mantém os últimos 7)
- 🕹️ **RCON** habilitado (usado pelo painel e via SSH)
- ♻️ **systemd** cuida de tudo: servidor e painel sobem no boot e reiniciam sozinhos se caírem

> **Nota de licenciamento:** a ISO **não** redistribui os `.jar` da Mojang/Paper/Fabric. O instalador os baixa das fontes oficiais na hora da instalação, e pede que você aceite a [EULA da Mojang](https://aka.ms/MinecraftEULA).

## Hardware alvo

| | Mínimo | Confortável |
|---|---|---|
| CPU | 2 núcleos x86-64 | 4 núcleos |
| RAM | 4 GB | 8 GB+ |
| Disco | 16 GB | SSD |
| Boot | UEFI | UEFI |

Com 4 GB roda Paper com 3–6 amigos num mundo normal. Mods pesados (modpacks grandes) pedem 8 GB+ e uma CPU melhor — veja [as ressalvas abaixo](#limitações-honestas).

---

## Como usar

> 📖 **Guia completo de instalação (Ventoy + passo a passo com a interface): [`INSTALL.md`](INSTALL.md).** O resumo abaixo é o essencial.

### 1. Baixar a ISO

Pegue a última versão em **[Releases](https://github.com/Vascon11/craftbox/releases)** (ou compile localmente — veja abaixo). Confira o checksum:

```bash
sha256sum -c craftbox.sha256
```

### 2. Gravar num pendrive

> ⚠️ **`dd` no disco errado apaga tudo.** Confirme o device com `lsblk` antes.

```bash
lsblk -dpo NAME,SIZE,MODEL                    # ache seu pendrive (ex: /dev/sdb)
sudo dd if=craftbox-*.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

Ou use uma GUI: Fedora Media Writer, balenaEtcher, GNOME Disks. Também funciona **copiando a ISO pra dentro de um pendrive [Ventoy](https://www.ventoy.net)**.

### 3. Instalar

Dê boot pelo pendrive → o instalador abre sozinho (assistente em tela cheia, no estilo do Ubuntu Server: teclado, rede/Wi-Fi/IP fixo, disco, perfil, chaves SSH do GitHub, Minecraft e painel) → confirme no resumo → ele instala e reinicia já servindo Minecraft na porta **25565**, com o **painel em `http://<ip>:8080`**.

### 4. Administrar

O jeito principal é o **painel web** (`http://<ip-do-servidor>:8080`). Pela linha de comando (SSH), lembrando que cada instância é `minecraft@<nome>`:

```bash
ssh SEU_USUARIO@IP_DO_SERVIDOR
systemctl status minecraft@principal        # status da instância
journalctl -u minecraft@principal -f        # log ao vivo
sudo systemctl restart minecraft@principal  # reiniciar
systemctl status craftbox-panel             # o painel web
/srv/minecraft/principal/backup.sh          # backup manual do mundo
```

---

## Compilar a ISO localmente

Precisa de **Docker** (não Podman — o `mkarchiso` precisa de bind-mounts que o Podman rootless não faz):

```bash
git clone https://github.com/Vascon11/craftbox.git
cd craftbox
./build.sh                          # gera out/craftbox-*.iso (~10-25 min)
```

## Como o CI publica as ISOs

O workflow [`build-iso.yml`](.github/workflows/build-iso.yml) roda no GitHub Actions:

- **Em qualquer tag `vX.Y.Z`** → compila a ISO e publica um **Release** com a ISO + checksum anexados.
- **Manualmente** (aba Actions → *Build ISO* → *Run workflow*) → compila e sobe a ISO como *artifact*.

Pra soltar uma versão nova:

```bash
git tag v1.0.0
git push origin v1.0.0
```

---

## Rodar como container (Docker / Docker Compose)

Além da ISO, o craftbox pode rodar em **qualquer máquina com Docker** — alternativo à distro inteira. O container traz o painel web, **OpenJDK 8/17/21/25** (qualquer versão do Minecraft, inclusive as por calendário 26.x) e os CLIs de túnel (playit, cloudflared, tailscale).

Dentro do container o painel **gerencia os servidores por `child_process`** (runner `exec`, sem systemd e sem Docker socket): cada instância é um processo Java isolado com PID file/log próprios, e o `java` usado é escolhido **automaticamente pela versão do Minecraft** (`mcVersion` → JDK 8 até 1.16, 17 de 1.17 a 1.20.4, 21 de 1.20.5 a 1.21.x e 25 nas versões por calendário 26.1+).

```bash
# usa a senha do painel via CRAFTBOX_PASSWORD na primeira subida
docker compose up -d --build
# → painel em http://localhost:8080
# servidores na rede host: portas 25565-25575 (TCP/UDP)
```

### Como usar (primeiros passos)

```bash
# 1. Pré-requisitos: Docker Engine + Compose (https://docs.docker.com/engine/install/)
docker --version && docker compose version

# 2. Clonar e entrar no projeto
git clone https://github.com/Vascon11/craftbox.git && cd craftbox

# 3. Definir a senha inicial do painel e subir
CRAFTBOX_PASSWORD=sua-senha docker compose up -d --build
```

4. Abra **http://localhost:8080** e entre com a senha definida acima.
5. No painel, em **Servidores → Novo**, crie o primeiro servidor (escolha loader Paper/Fabric, versão e porta dentro de `25565-25575`). O aguarde baixar e montar a instância.
6. **Ligar** o servidor pelos cards/hero. Depois de no ar, entre no console (`RCON`), na aba **Logs** (tempo real) e gere **Backups** do mundo.
7. Para subir servidores automaticamente junto com o container, rode com `CRAFTBOX_AUTOSTART=zumbie,principal` (ou edite o `.env` do compose) uma segunda vez.
8. **Parar**: `docker stop craftbox` e `docker compose down` — o container entrega **SIGTERM** aos servidores, salvando os mundos antes de sair.
9. **Atualizar**: `git pull` + `docker compose up -d --build`.

Logs do painel/runner: `docker logs -f craftbox`.

> **Dica:** se a porta `8080` do host já estiver ocupada, mude só a primeira parte do mapeamento no compose (`"8081:8080"`). O `CRAFTBOX_PORT` (lado do container) continua `8080`.

### Variáveis de ambiente (`docker-compose.yml`)

| Variável | Padrão | Descrição |
|---|---|---|
| `TZ` | `UTC` | Fuso horário do container (ex: `America/Sao_Paulo`) |
| `CRAFTBOX_PORT` | `8080` | Porta do painel dentro do container |
| `CRAFTBOX_PASSWORD` | *(vazio)* | Senha inicial do painel (aplicada só se o `config.json` ainda não tiver senha) |
| `CRAFTBOX_AUTOSTART` | *(vazio)* | Servidores que sobem com o container, separados por vírgula (ex: `zumbie,principal`). Sem isso, ligue pelo painel. |
| `CRAFTBOX_MEM_LIMIT` | `4g` | Limite de memória do container |
| `PLAYIT_ARCH` / `CLOUDFLARED_ARCH` | *(auto)* | Forçar arquitetura dos binários de túnel (auto = `TARGETARCH` do buildkit) |

### Persistência e portas

- **Volume `/data`** guarda tudo: `config.json`, **instâncias** (`/data/servers/<id>/` — mundos, mods, configs, **backups**), integrações (`/data/craftbox-integrations`) e o estado do runner (`/data/craftbox-run`). Para editar no host, troque por um bind mount (`./data:/data`).
- **Portas `25565-25575` (TCP/UDP)** mapeadas para os servidores MC; `19132/UDP` (Geyser/Bedrock) fica comentada no compose.
- `docker stop` executa o **shutdown gracioso** (SIGTERM pros processos → mundos salvam antes de sair). O compose usa `init: true` (tini como PID 1) e `stop_grace_period: 90s`, porque o painel para as instâncias uma a uma; com `docker run` puro, passe `--init --stop-timeout 90`.

### Mesmo caminho em distro física e no container

O backend aceita os mesmos caminhos em ambos os ambientes, via `config.json` **ou** env vars (`CRAFTBOX_SERVERS_DIR`, `CRAFTBOX_INTEGRATIONS_DIR`, `CRAFTBOX_RUN_DIR`). A única diferença é o gerenciador de processos: **systemd** na ISO (comportamento original) vs **runner exec** no container — detectado automaticamente quando não há `/run/systemd/system` (ou forçando `CRAFTBOX_RUNNER=exec`).

### Problemas comuns

**"Authentication servers are down. Please try again later, sorry!"** (com `online-mode=true`) — quem valida o login é o **servidor**, não o jogador: ele precisa de saída HTTPS estável para `sessionserver.mojang.com` e `api.minecraftservices.com`. Se a Mojang está no ar e o erro só acontece no seu servidor, o problema costuma ser o **Docker travado ou sem recursos** no host — não a conta do jogador.

Pra checar se o container alcança a Mojang (`204` = alcança):

```bash
docker exec craftbox curl -sS -o /dev/null -w "%{http_code}\n" \
  "https://sessionserver.mojang.com/session/minecraft/hasJoined?username=x&serverId=y"
```

Ou use o botão **Testar conexão com a Mojang** no painel.

**No Windows (Docker Desktop / WSL2):**

- Confira o **espaço livre do `C:`** no Windows. O disco que o painel mostra é o disco virtual (VHDX) do WSL, que cresce sob demanda, então esse número **não** reflete o `C:` (o aviso de disco baixo do painel, abaixo de 2 GB, tem a mesma limitação). Com o `C:` quase cheio, o Docker Desktop trava.
- Se `docker ps` fica **pendurado**, o engine travou: libere espaço e **reinicie o Docker Desktop**. Caso real: com menos de 500 MB livres no `C:`, os jogadores caíam nesse erro, e liberar espaço + reiniciar resolveu na hora.

---

## Estrutura do projeto

```
craftbox/
├── airootfs/                       # arquivos sobrepostos ao ambiente live do Arch
│   ├── usr/local/bin/mc-install    # o instalador (coração do projeto)
│   ├── root/.zprofile              # abre o instalador automaticamente no boot
│   └── etc/motd
├── panel/                          # o painel web (embarcado na ISO pelo build)
├── docker/
│   ├── entrypoint.sh               # prepara /data e sobe o painel no container
│   ├── init.js                     # cria config.json + semeia binários de túnel
│   └── java-select                 # wrapper: escolhe JDK 8/17/21/25 pelo MC version
├── Dockerfile                      # imagem: Node 22 + JDK 8/17/21/25 + túneis
├── docker-compose.yml              # compose pronto (volumes, portas 25565-75, envs)
├── packages.extra                  # pacotes extras do ambiente live
├── scripts/build-in-container.sh   # monta a ISO (roda dentro do container Arch)
├── build.sh                        # wrapper de build local (Docker)
└── .github/workflows/build-iso.yml # CI: compila + publica Releases
```

---

## Limitações honestas

- **Mods pesados travam em hardware fraco.** O servidor de Minecraft é preso a *single-thread*; um i5-4210U aguenta Paper com poucos amigos, mas modpacks grandes derrubam o TPS. Prefira **Paper** quando puder.
- **Versionamento do Minecraft:** desde 2026 o Minecraft usa versionamento por calendário (ex: `26.2`). O instalador puxa a última estável automaticamente via API oficial.
- Testado no Dell Inspiron 3442. Deve funcionar em qualquer x86-64 UEFI, mas *YMMV*.

## Licença

[MIT](LICENSE). Arch Linux, Paper, Fabric e Minecraft pertencem aos seus respectivos donos.
