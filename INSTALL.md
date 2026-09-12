# Instalando o craftbox 🧱

Guia passo a passo pra transformar um notebook/PC antigo (UEFI) num servidor de
Minecraft com o craftbox. Do pendrive ao primeiro login no painel.

> ⚠️ **A instalação APAGA o disco inteiro da máquina de destino.** Faça backup do
> que importa antes. É pra ser um servidor dedicado.

---

## 1. O que você precisa

- A **ISO do craftbox** — baixe em [Releases](https://github.com/Vascon11/craftbox/releases) (ou compile com `./build.sh`).
- Um **pendrive** (≥ 2 GB) — que vira o instalador.
- A **máquina de destino** com **boot UEFI** e **internet** (cabo de rede é o mais tranquilo; Wi-Fi também dá).
- Alguns minutos. ☕

---

## 2. Gravar a ISO no pendrive

### Opção A — Ventoy (recomendada) 🟢

O [Ventoy](https://www.ventoy.net) transforma o pendrive num "menu de ISOs": você
copia a ISO como um arquivo normal e escolhe ela no boot. Melhor porque o pendrive
continua usável pra outras ISOs e pra guardar arquivos.

1. Baixe o Ventoy e instale no pendrive **uma vez** (isso formata o pendrive):
   - Linux: baixe o `ventoy-x.y.z-linux.tar.gz`, extraia e rode `sudo ./Ventoy2Disk.sh -i /dev/sdX` (troque `sdX` pelo seu pendrive — confira com `lsblk`).
   - Windows: abra o `Ventoy2Disk.exe`, selecione o pendrive e clique **Install**.
2. Depois de instalado, o pendrive tem uma partição grande. **Copie a ISO do craftbox pra dentro dela** (arrastar e soltar, como qualquer arquivo).
3. Pode copiar várias ISOs — o Ventoy mostra um menu na hora do boot.

### Opção B — Gravar direto (dd / Fedora Media Writer / balenaEtcher)

Se preferir um pendrive dedicado só pro craftbox:

```bash
lsblk -dpo NAME,SIZE,MODEL          # ache seu pendrive (ex: /dev/sdb)
sudo dd if=craftbox-*.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

> ⚠️ `dd` no disco errado apaga tudo. **Confirme o device com `lsblk` antes.**
> Alternativas gráficas: **Fedora Media Writer**, **balenaEtcher**, **GNOME Discos**.

---

## 3. Bootar a máquina pelo pendrive

1. Espete o pendrive na máquina de destino.
2. Ligue e abra o **menu de boot** (tecla varia por fabricante, aperte repetidamente ao ligar):
   - Dell: **F12** · HP: **F9** · Lenovo: **F12** · Acer: **F12** · ASUS: **Esc/F8**
3. Escolha o pendrive (algo como *"UEFI: <marca do pendrive>"*).
4. **Se for Ventoy:** aparece o menu do Ventoy → escolha a **ISO do craftbox** → dê Enter (pode escolher "Boot in normal mode").
5. O Arch live carrega e o **instalador do craftbox abre sozinho**.

> Se não aparecer o pendrive no menu de boot, entre na BIOS/UEFI e desative o
> **Secure Boot** (o Arch live não é assinado).

---

## 4. A interface do instalador (passo a passo)

O instalador (`mc-install`) é em texto, direto, e pergunta uma coisa de cada vez.
Valores entre `[colchetes]` são o padrão — é só apertar **Enter** pra aceitar.

<!-- SCREENSHOT: tela inicial do instalador -->

1. **Aviso + confirmação** — ele avisa que vai apagar um disco. Digite `s` pra continuar.
2. **Rede** — se tiver cabo, ele já pega internet sozinho. Sem cabo, ele pergunta o
   **nome da rede Wi-Fi (SSID)** e a **senha** (e guarda essa conexão pro sistema instalado).
3. **Disco de destino** — mostra os discos disponíveis e pergunta em qual instalar
   (padrão `/dev/sda`). **Tudo nele será apagado** — confirme com cuidado.
4. **Sistema:**
   - **Hostname** (nome da máquina na rede) — padrão `mc-server`.
   - **Usuário** do login por SSH — padrão `vascon1`.
   - **Senha** desse usuário (digita duas vezes).
   - **Chave SSH pública** (opcional) — cole se quiser entrar sem senha; ou só Enter pra pular.
5. **Minecraft:**
   - **Tipo de servidor** — `1` = **Paper** (plugins, leve, recomendado) · `2` = **Fabric** (mods).
   - **Versão** — Enter deixa a **última estável**.
   - **Nome do primeiro servidor** — padrão `Principal` (dá pra criar mais depois pelo painel).
   - **MOTD** (mensagem do servidor), **máximo de jogadores**, **dificuldade**.
6. **Painel web** — você **define a senha do painel** (digita duas vezes). É a senha
   que você vai usar em `http://<ip>:8080`.
7. **EULA da Mojang** — aceite (`s`) pra o servidor poder iniciar.
8. **Resumo** — ele mostra tudo (disco, hostname, servidor, etc.) e pede a confirmação
   final antes de mexer no disco. A partir daqui ele **particiona, instala o Arch,
   baixa o Minecraft e configura tudo** (uns minutos, mais lento em HD).
9. **Fim** — ele avisa que terminou e oferece **reiniciar**. Tire o pendrive quando a tela apagar.

<!-- SCREENSHOT: resumo antes de instalar -->
<!-- SCREENSHOT: instalação concluída -->

---

## 5. Depois de instalado

Ao reiniciar, sozinho no boot: o **servidor sobe** na porta **25565** e o **painel web** fica no ar.

### Acessar o painel (jeito principal)

No navegador de qualquer PC/celular na **mesma rede**:

```
http://<ip-do-servidor>:8080
```

Entre com a **senha do painel** que você definiu. Dali você liga/desliga o servidor,
instala mods/plugins, cria outros servidores, vê o console e os backups, etc.

> **Qual é o IP?** O painel/servidor mostra no boot, ou descubra pelo roteador. Por
> SSH: `ip -4 addr`. Dica: fixe um IP pra ele no roteador (DHCP reservation).

### Por SSH (linha de comando)

```bash
ssh SEU_USUARIO@IP_DO_SERVIDOR
systemctl status minecraft@principal        # status da instância
journalctl -u minecraft@principal -f        # log ao vivo
sudo systemctl restart minecraft@principal  # reiniciar
systemctl status craftbox-panel             # o painel web
```

Cada servidor é uma instância `minecraft@<nome>`, com os arquivos em
`/srv/minecraft/<nome>/`. O painel gerencia todas.

### Entrar no jogo

No Minecraft (Java), **Multijogador → Adicionar servidor**, endereço
`IP_DO_SERVIDOR:25565`. Pra jogar de fora de casa sem mexer no roteador, use a aba
**Integrações** do painel (playit.gg ou Tailscale).

---

## 6. Problemas comuns

- **Não bota pelo pendrive** — abra a BIOS e desative o **Secure Boot**; confira se o boot é **UEFI**.
- **"Sem internet" no instalador** — conecte o cabo de rede (mais simples) ou confira SSID/senha do Wi-Fi.
- **5 beeps ao ligar (Dell)** — bateria da BIOS (CR2032) fraca; troque a moeda. Não impede instalar.
- **Servidor não inicia** — no painel, aba **Console**, veja o log. Versões muito novas do MC exigem Java novo; o craftbox já instala o Java certo.
