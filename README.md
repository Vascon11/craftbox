# craftbox 🧱

**Uma mini-distro Linux que transforma um notebook velho num servidor de Minecraft dedicado.**

`craftbox` é um *spin* enxuto do [Arch Linux](https://archlinux.org): você grava a ISO num pendrive, dá boot, responde algumas perguntas, e no fim tem uma máquina headless que existe pra uma coisa só — rodar um servidor de Minecraft (Paper ou Fabric) que sobe sozinho no boot.

Sem ambiente gráfico, sem peso: toda a RAM sobra pro jogo.

![Build ISO](https://github.com/Vascon11/craftbox/actions/workflows/build-iso.yml/badge.svg)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)

---

## Interface

**Instalador** — abre sozinho no boot e cuida de tudo: escolha de Paper/Fabric, rede, EULA e dimensionamento automático de RAM.

![Instalador do craftbox](screenshots/instalador.png)

**Sistema instalado** — o appliance recebe você por SSH com um resumo dos comandos (MOTD):

![MOTD do sistema](screenshots/sistema-motd.png)

**Servidor no ar** — Paper rodando sob Java 26, com as portas 25565 (jogo) e 25575 (RCON) escutando:

![Servidor rodando](screenshots/servidor-rodando.png)

> Todas as telas acima são de uma instalação real do craftbox, validada numa máquina virtual UEFI (QEMU/KVM).

### Painel web — `craftbox-panel`

Um painel web opcional (**Node.js puro, zero dependências**) pra configurar e gerenciar o servidor pelo navegador: status + stats do sistema (CPU, RAM, disco, temperatura), ligar/reiniciar/desligar, editor visual do `server.properties`, console RCON, logs ao vivo, backups, **navegador de mods/plugins do Modrinth** (instala com 1 clique, estilo Prism) e **compatibilidade Bedrock (Geyser/Floodgate) + modo offline** — tudo com login por senha. Veja [`panel/`](panel/).

![Painel — dashboard](screenshots/painel-dashboard.jpg)
![Painel — configuração](screenshots/painel-config.jpg)

---

## Por que existe

Nasceu de um caso real: um **Dell Inspiron 3442** (Intel i5-4210U, 4–8 GB DDR3, HD 5400 rpm) parado numa gaveta. Em vez de virar lixo eletrônico, virou servidor de Minecraft pros amigos. O `craftbox` é o resultado empacotado pra qualquer notebook antigo do mesmo porte.

## O que a ISO faz por você

Ao dar boot, o instalador (`mc-install`) abre **automaticamente** e cuida de tudo:

- 🌐 **Rede** — detecta cabo (DHCP) ou pergunta o Wi-Fi e **guarda a conexão** pro sistema instalado (via NetworkManager)
- 💽 **Instalação automática** — particiona (UEFI/GPT), formata e instala um Arch mínimo no disco
- 🎮 **Escolha do servidor na hora** — **Paper** (plugins, leve) ou **Fabric** (mods, com mods de performance já incluídos)
- 🧠 **Heap da JVM auto-dimensionado** pela RAM detectada + **[flags do Aikar](https://docs.papermc.io/paper/aikars-flags)** aplicadas
- ⚙️ **Otimizado pra CPU fraca** — `view-distance` e `simulation-distance` calibrados
- 🔒 **SSH** pronto (com sua chave pública, se você colar uma) + teclado ABNT2 e timezone BR
- 💻 **Fechar a tampa não desliga** (`HandleLidSwitch=ignore`) — o notebook vira um "servidor de tampa fechada"
- 💾 **Backup diário** do mundo (mantém os últimos 7)
- 🕹️ **RCON** habilitado pra mandar comandos no servidor via SSH
- ♻️ **systemd** cuida do servidor: sobe no boot, reinicia sozinho se cair

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

Dê boot pelo pendrive → o instalador abre sozinho → responda as perguntas → ele instala e reinicia já servindo Minecraft na porta **25565**.

### 4. Administrar (via SSH)

```bash
ssh SEU_USUARIO@IP_DO_SERVIDOR
systemctl status minecraft          # status
journalctl -u minecraft -f          # log ao vivo
sudo systemctl restart minecraft    # reiniciar
/opt/minecraft/backup.sh            # backup manual do mundo
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

## Estrutura do projeto

```
craftbox/
├── airootfs/                       # arquivos sobrepostos ao ambiente live do Arch
│   ├── usr/local/bin/mc-install    # o instalador (coração do projeto)
│   ├── root/.zprofile              # abre o instalador automaticamente no boot
│   └── etc/motd
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
