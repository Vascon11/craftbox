# shellcheck shell=bash
# =============================================================================
# craftbox — arquivos de sistema (units do systemd, sudoers, pacotes).
#
# Fonte única usada por dois caminhos:
#   - mc-install      → na instalação, com CB_ROOT=/mnt (sistema novo montado)
#   - craftbox-update → numa máquina já instalada, com CB_ROOT="" (o próprio /)
# Tudo aqui é idempotente: rodar de novo só regrava os arquivos.
#
# Uso:  . /usr/local/lib/craftbox/system.sh
#       CB_ROOT=/mnt cb_write_system_files
# =============================================================================

CB_ROOT="${CB_ROOT:-}"

# pacotes que o sistema instalado precisa (o updater garante que estejam lá;
# assim uma versão nova que passe a depender de algo chega às máquinas antigas).
# JDKs: o painel escolhe por versão do MC (8 ≤1.16, 17 até 1.20.4, 21 até 1.21.x,
# a mais nova — jre-openjdk — pras versões por calendário 26.x).
CB_PACKAGES=(
  networkmanager openssh sudo
  jre-openjdk-headless jre21-openjdk-headless jre17-openjdk-headless jre8-openjdk-headless
  git tmux nano vim htop jq curl wget which unzip
  msr-tools
)

# o `java` padrão é sempre a JDK mais nova instalada (as outras o painel chama pelo caminho)
cb_java_default_newest() {
  local newest
  newest=$(archlinux-java status 2>/dev/null | grep -o 'java-[0-9]*-openjdk' | sort -t- -k2 -n | tail -1)
  [ -n "$newest" ] && archlinux-java set "$newest" >/dev/null 2>&1
  return 0
}

cb_panel_exec() {
  if [ -x "${CB_ROOT:-}/opt/craftbox-panel/craftbox-panel" ]; then
    echo /opt/craftbox-panel/craftbox-panel
  else
    echo "/usr/bin/node /opt/craftbox-panel/server.js"
  fi
}

cb_write_units() {
  local d="${CB_ROOT:-}/etc/systemd/system"
  mkdir -p "$d"
  # serviço do minecraft — TEMPLATE (uma unidade por instância: minecraft@<id>)
  cat > "$d/minecraft@.service" <<'EOF'
[Unit]
Description=Servidor Minecraft (%i)
After=network-online.target
Wants=network-online.target
# se cair 3x em 10 min (ex.: OOM), para de tentar em vez de travar a máquina em loop
StartLimitIntervalSec=600
StartLimitBurst=3
[Service]
Type=simple
User=minecraft
Group=minecraft
WorkingDirectory=/srv/minecraft/%i
ExecStart=/srv/minecraft/%i/start.sh
Restart=on-failure
RestartSec=15
# Paper/vanilla salvam o mundo ao receber SIGTERM
KillSignal=SIGTERM
# salvar o mundo num HD leva minutos (Cursed Walking: 183 s; um Fabric 1.20.1 recém-
# gerado: 220 s). Matar antes disso corrompe o mundo, então a folga é grande.
TimeoutStopSec=600
# protege o SO: o servidor não pode levar a máquina inteira pro swap (com 4 GB o
# Dell travava até o SSH). Passou do teto, o OOM mata só o Minecraft.
MemoryMax=90%
MemorySwapMax=1G
# parar o servidor (SIGTERM) faz a JVM sair com 143 — isso é parada limpa, não falha
SuccessExitStatus=143 SIGTERM
[Install]
WantedBy=multi-user.target
EOF
  cat > "$d/mc-backup@.service" <<'EOF'
[Unit]
Description=Backup do mundo (%i)
[Service]
Type=oneshot
User=minecraft
ExecStart=/srv/minecraft/%i/backup.sh
EOF
  cat > "$d/mc-backup@.timer" <<'EOF'
[Unit]
Description=Backup diario do mundo (%i)
[Timer]
OnCalendar=*-*-* 05:00:00
Persistent=true
[Install]
WantedBy=timers.target
EOF
  cat > "$d/craftbox-panel.service" <<EOF
[Unit]
Description=craftbox-panel (painel web)
After=network-online.target
[Service]
Type=simple
User=minecraft
Group=minecraft
WorkingDirectory=/opt/craftbox-panel
ExecStart=$(cb_panel_exec)
Restart=on-failure
RestartSec=5
[Install]
WantedBy=multi-user.target
EOF
}

# regras do sudo pro usuário do painel. Escreve num temporário e só troca se o
# visudo aprovar: sudoers quebrado tranca o sudo da máquina inteira.
cb_write_sudoers() {
  local f="${CB_ROOT:-}/etc/sudoers.d/craftbox-panel" tmp
  mkdir -p "${CB_ROOT:-}/etc/sudoers.d"
  tmp=$(mktemp)
  cat > "$tmp" <<'SUDO'
minecraft ALL=(root) NOPASSWD: /usr/bin/systemctl start minecraft@*, /usr/bin/systemctl stop minecraft@*, /usr/bin/systemctl restart minecraft@*
minecraft ALL=(root) NOPASSWD: /usr/bin/systemctl kill --signal=SIGKILL minecraft@*
minecraft ALL=(root) NOPASSWD: /usr/bin/tailscale up *, /usr/bin/tailscale down
minecraft ALL=(root) NOPASSWD: /usr/bin/nmcli
# atualização pelo painel: só checar, e disparar o updater fora do cgroup do painel
# (ele reinicia o painel no fim). Linhas exatas, sem curinga.
minecraft ALL=(root) NOPASSWD: /usr/local/bin/craftbox-update --check --json
minecraft ALL=(root) NOPASSWD: /usr/bin/systemd-run --unit=craftbox-update --collect --no-block /usr/local/bin/craftbox-update --yes
SUDO
  if command -v visudo >/dev/null && ! visudo -cqf "$tmp"; then
    rm -f "$tmp"; echo "sudoers gerado é inválido; mantive o anterior" >&2; return 1
  fi
  install -m440 "$tmp" "$f"
  rm -f "$tmp"
}

cb_write_system_files() {
  cb_write_units
  cb_write_sudoers
}
