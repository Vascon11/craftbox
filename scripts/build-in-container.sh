#!/usr/bin/env bash
#
# Roda DENTRO de um container Arch privilegiado.
# Monta a ISO do craftbox a partir do perfil oficial 'releng' + customizacoes.
#
# Espera:
#   /build -> raiz do repositorio (montada pelo host/CI)
#   /out   -> diretorio de saida da ISO
#
set -euxo pipefail

# ferramentas de build
pacman -Sy --noconfirm archiso

# parte do perfil oficial 'releng' e customiza por cima
rm -rf /tmp/profile
cp -r /usr/share/archiso/configs/releng /tmp/profile

# 1) pacotes extras no ambiente live (o sistema alvo eh instalado pelo mc-install)
cat /build/packages.extra >> /tmp/profile/packages.x86_64

# 2) nossos arquivos por cima do airootfs (installer, auto-launch, motd)
cp -rT /build/airootfs /tmp/profile/airootfs
chmod 755 /tmp/profile/airootfs/usr/local/bin/mc-install

# 2b) embarca o painel web na ISO (o mc-install copia de /opt/craftbox-panel pro sistema)
mkdir -p /tmp/profile/airootfs/opt/craftbox-panel
cp -rT /build/panel /tmp/profile/airootfs/opt/craftbox-panel
rm -f /tmp/profile/airootfs/opt/craftbox-panel/config.json

# 2c) painel em Rust (panel-rs): binário único no lugar do server.js. O perfil
# `dist` (LTO) sai com x86-64 genérico, então roda em qualquer CPU. O mc-install
# usa o binário se existir e cai pro Node se não.
pacman -S --noconfirm --needed rust
CARGO_TARGET_DIR=/tmp/cargo-target cargo build --manifest-path /build/panel-rs/Cargo.toml --profile dist --locked
install -m755 /tmp/cargo-target/dist/craftbox-panel /tmp/profile/airootfs/opt/craftbox-panel/craftbox-panel
/tmp/profile/airootfs/opt/craftbox-panel/craftbox-panel --version

# 2d) versão + pacote de sistema pro craftbox-update. A tag vem do CI
# (CB_VERSION=vX.Y.Z); build local sai como "dev". O pacote tem o mesmo painel e
# scripts que a ISO instala, com os caminhos a partir da raiz do sistema.
CB_VERSION="${CB_VERSION:-dev}"
mkdir -p /tmp/profile/airootfs/etc
printf 'VERSION=%s\n' "$CB_VERSION" > /tmp/profile/airootfs/etc/craftbox-release
rm -f /tmp/profile/airootfs/opt/craftbox-panel/panel.log
SYS=/tmp/craftbox-system
rm -rf "$SYS" && mkdir -p "$SYS/opt" "$SYS/etc/systemd/system"
cp -a /tmp/profile/airootfs/opt/craftbox-panel "$SYS/opt/craftbox-panel"
install -Dm755 /build/airootfs/usr/local/bin/craftbox-update    "$SYS/usr/local/bin/craftbox-update"
install -Dm755 /build/airootfs/usr/local/bin/craftbox-bdprochot "$SYS/usr/local/bin/craftbox-bdprochot"
install -Dm755 /build/airootfs/usr/local/lib/craftbox/system.sh "$SYS/usr/local/lib/craftbox/system.sh"
install -Dm644 /build/airootfs/etc/systemd/system/craftbox-bdprochot.service "$SYS/etc/systemd/system/craftbox-bdprochot.service"
cp /tmp/profile/airootfs/etc/craftbox-release "$SYS/etc/craftbox-release"
mkdir -p /out
tar -C "$SYS" --owner=0 --group=0 -czf "/out/craftbox-system-$CB_VERSION.tar.gz" .
( cd /out && sha256sum "craftbox-system-$CB_VERSION.tar.gz" > "craftbox-system-$CB_VERSION.tar.gz.sha256" )

# 3) identidade da ISO
sed -i 's/^iso_name=.*/iso_name="craftbox"/'                                   /tmp/profile/profiledef.sh
sed -i "s/^iso_label=.*/iso_label=\"CRAFTBOX_$(date +%Y%m)\"/"                 /tmp/profile/profiledef.sh
sed -i 's/^iso_publisher=.*/iso_publisher="craftbox (github.com\/Vascon11\/craftbox)"/' /tmp/profile/profiledef.sh
sed -i 's/^iso_application=.*/iso_application="craftbox - Minecraft Server Appliance"/'  /tmp/profile/profiledef.sh

# 4) permissao correta do instalador na imagem final
sed -i '/^file_permissions=(/a\  ["/usr/local/bin/mc-install"]="0:0:755"' /tmp/profile/profiledef.sh
sed -i '/^file_permissions=(/a\  ["/usr/local/bin/craftbox-update"]="0:0:755"' /tmp/profile/profiledef.sh
sed -i '/^file_permissions=(/a\  ["/usr/local/lib/craftbox/system.sh"]="0:0:755"' /tmp/profile/profiledef.sh
sed -i '/^file_permissions=(/a\  ["/opt/craftbox-panel/craftbox-panel"]="0:0:755"' /tmp/profile/profiledef.sh

# constroi
mkdir -p /out
mkarchiso -v -w /tmp/work -o /out /tmp/profile

# checksum + ajuste de permissao
cd /out
sha256sum ./*.iso | tee craftbox.sha256
chmod 644 ./*.iso ./*.sha256 ./*.tar.gz 2>/dev/null || true
ls -lh /out
