#!/usr/bin/env bash
#
# Instala o craftbox-panel num servidor craftbox (ou qualquer Arch com o
# servico 'minecraft' em /opt/minecraft). Rode como root:  sudo ./setup.sh
#
set -euo pipefail
DEST=/opt/craftbox-panel
SRC="$(cd "$(dirname "$0")" && pwd)"
SERVICE=minecraft
MCDIR=/opt/minecraft

[ "$(id -u)" -eq 0 ] || { echo "rode como root: sudo ./setup.sh"; exit 1; }
command -v node >/dev/null || { echo "node nao encontrado. Instale: pacman -S nodejs"; exit 1; }
id minecraft &>/dev/null || { echo "usuario 'minecraft' nao existe (o servidor craftbox nao esta instalado?)"; exit 1; }

echo "==> Copiando arquivos para $DEST"
mkdir -p "$DEST"
cp -r "$SRC/server.js" "$SRC/public" "$DEST/"
chown -R minecraft:minecraft "$DEST"

echo "==> Senha de acesso ao painel"
read -rsp "  defina a senha do painel: " PW; echo
HASH_JSON=$(sudo -u minecraft node "$DEST/server.js" --hash "$PW" | tail -n +2)

echo "==> Gerando config.json"
SECRET=$(node -e 'console.log(require("crypto").randomBytes(32).toString("hex"))')
if [ -d /srv/minecraft ]; then
  # modo multi-servidor (appliance): instancias em /srv/minecraft
  FIRST=$(ls -1 /srv/minecraft 2>/dev/null | head -1)
  cat > "$DEST/config.json" <<EOF
{
  "port": 8080,
  "host": "0.0.0.0",
  "serversDir": "/srv/minecraft",
  "serviceTemplate": "minecraft@",
  "activeServer": "${FIRST:-principal}",
  "systemctlUser": false,
  "rcon": { "host": "127.0.0.1", "port": 25575, "password": "" },
  "auth": $HASH_JSON,
  "sessionSecret": "$SECRET"
}
EOF
else
  # modo 1 servidor (legado)
  cat > "$DEST/config.json" <<EOF
{
  "port": 8080,
  "host": "0.0.0.0",
  "mcDir": "$MCDIR",
  "service": "$SERVICE",
  "rcon": { "host": "127.0.0.1", "port": 25575, "password": "" },
  "auth": $HASH_JSON,
  "sessionSecret": "$SECRET"
}
EOF
fi
chown minecraft:minecraft "$DEST/config.json"
chmod 600 "$DEST/config.json"

echo "==> Permissao pro painel controlar o servico (sudoers)"
cat > /etc/sudoers.d/craftbox-panel <<EOF
minecraft ALL=(root) NOPASSWD: /usr/bin/systemctl start $SERVICE, /usr/bin/systemctl stop $SERVICE, /usr/bin/systemctl restart $SERVICE
minecraft ALL=(root) NOPASSWD: /usr/bin/systemctl start minecraft@*, /usr/bin/systemctl stop minecraft@*, /usr/bin/systemctl restart minecraft@*
minecraft ALL=(root) NOPASSWD: /usr/bin/tailscale up *, /usr/bin/tailscale down
EOF
chmod 440 /etc/sudoers.d/craftbox-panel

# Se o tailscale estiver instalado, deixa o painel controlar sem root (plug-and-play)
if command -v tailscale >/dev/null 2>&1; then
  tailscale set --operator=minecraft 2>/dev/null || true
fi

echo "==> Instalando servico systemd"
cp "$SRC/craftbox-panel.service" /etc/systemd/system/craftbox-panel.service
systemctl daemon-reload
systemctl enable --now craftbox-panel

IP=$(ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -1)
echo
echo "==> Pronto! Painel em:  http://${IP:-<ip-do-servidor>}:8080"
echo "    Ver status:  systemctl status craftbox-panel"
