# craftbox-panel

Painel web para **configurar e gerenciar** o servidor Minecraft do craftbox.
Escrito em **Node.js puro, sem nenhuma dependência** (só módulos nativos) — leve
o suficiente pra rodar ao lado do servidor num notebook antigo (~30–40 MB de RAM).

## O que faz

- **Multi-servidor** — cria, clona e apaga vários servidores; cada instância é uma subpasta em `serversDir` com loader, versão e porta próprios, e um serviço systemd `minecraft@<id>`. Um **seletor** no topo troca o servidor ativo. Pensado pra rodar **um de cada vez** em hardware fraco.
- **Painel** — status (no ar/desligado), jogadores online (via RCON), e stats do sistema em tempo real: **frequência da CPU**, RAM, disco, temperatura e load.
- **Ligar / Reiniciar / Desligar** o servidor via systemd — no sistema (`sudo systemctl`) ou **rootless** (`systemctl --user`, sem sudo), conforme `systemctlUser`.
- **Configuração** — editor visual do `server.properties` (MOTD, dificuldade, PvP, view-distance, etc.).
- **Console** — envia comandos ao servidor via **RCON**.
- **Logs** — últimas linhas do `latest.log`, com auto-atualização.
- **Backups** — lista e dispara o backup do mundo.
- **Conteúdo** — loja de **mods/plugins do [Modrinth](https://modrinth.com)** estilo Prism/Modrinth: busca com ícones/categorias/ordenação, **escolha de versão**, **instala as dependências obrigatórias junto**, e gerencia os instalados (ativar/desativar, atualizar, remover). Detecta o loader (Fabric→`mods/`, Paper→`plugins/`).
- **Modpacks** — cria um servidor a partir de um modpack do Modrinth (`.mrpack`) em **Fabric/Quilt/Forge/NeoForge**: baixa os arquivos do lado servidor, aplica os `overrides`, instala o loader e mostra o **link do pack** pros jogadores instalarem o mesmo no cliente.
- **Compatibilidade** — **modo offline** ("pirata", `online-mode=false`) e **Bedrock** (Geyser + Floodgate), cada um em 1 clique.
- **Login por senha** (sessão em cookie assinado).

![Dashboard](../screenshots/painel-dashboard.jpg)
![Conteúdo — mods/plugins](../screenshots/painel-conteudo.jpg)
![Gerenciar servidores](../screenshots/painel-servidores.jpg)
![Criar a partir de modpack](../screenshots/painel-modpack.jpg)

> Precisa do comando `unzip` disponível pra instalar modpacks (extrair o `.mrpack`).

> **Mods (Fabric) x plugins (Paper):** plugins rodam só no servidor (os amigos usam o Minecraft normal). Mods de conteúdo Fabric precisam ser instalados também no cliente de cada jogador — o painel instala o lado do servidor.

## Instalar no servidor craftbox

No servidor (que já tem o serviço `minecraft` em `/opt/minecraft`):

```bash
sudo pacman -S --needed nodejs        # se ainda não tiver
git clone https://github.com/Vascon11/craftbox.git
cd craftbox/panel
sudo ./setup.sh                       # pede a senha do painel e instala tudo
```

O `setup.sh` copia pra `/opt/craftbox-panel`, cria a senha, configura o `sudoers`
(pra ligar/desligar o serviço), instala o serviço systemd e sobe o painel em
`http://<ip-do-servidor>:8080`.

> **Segurança:** o painel controla o servidor — exponha só na **LAN** ou via
> **Tailscale**, nunca direto na internet. O login é protegido por senha, mas
> mantenha-o fora do alcance público.

## Rodar manualmente (dev/teste)

```bash
node server.js --hash MINHA_SENHA     # gera o hash pra colar no config.json
cp config.example.json config.json    # ajuste port/mcDir/service e cole o "auth"
node server.js                        # sobe em http://localhost:8080
```

## Configuração (`config.json`)

| Campo | Descrição |
|---|---|
| `port` / `host` | Onde o painel escuta (padrão `8080` / `0.0.0.0`). |
| `mcDir` | Pasta do servidor no **modo 1 servidor** (padrão `/opt/minecraft`). |
| `service` | Nome do serviço systemd no modo 1 servidor (padrão `minecraft`). |
| `serversDir` | **Ativa o multi-servidor**: pasta que guarda as instâncias (cada subpasta = 1 servidor). Vazio = modo 1 servidor. |
| `serviceTemplate` | Template do serviço systemd no multi (padrão `minecraft@`) — o serviço vira `minecraft@<id>`. |
| `activeServer` | Instância selecionada no painel (o painel mantém sozinho). |
| `systemctlUser` | `true` = controla via `systemctl --user` (**rootless**, sem sudo). `false` = `sudo systemctl` (padrão). |
| `rcon` | Host/porta/senha do RCON. Vazio = lê do `server.properties` de cada instância. |
| `auth` | `salt` + `hash` da senha (gere com `--hash`). |
| `sessionSecret` | Segredo pra assinar a sessão (gerado sozinho se vazio). |

O `config.json` guarda o hash da senha e o segredo de sessão — ele **não vai pro
git** (está no `.gitignore`) e fica com permissão `600`.
