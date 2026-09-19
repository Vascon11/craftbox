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

# 3) identidade da ISO
sed -i 's/^iso_name=.*/iso_name="craftbox"/'                                   /tmp/profile/profiledef.sh
sed -i "s/^iso_label=.*/iso_label=\"CRAFTBOX_$(date +%Y%m)\"/"                 /tmp/profile/profiledef.sh
sed -i 's/^iso_publisher=.*/iso_publisher="craftbox (github.com\/Vascon11\/craftbox)"/' /tmp/profile/profiledef.sh
sed -i 's/^iso_application=.*/iso_application="craftbox - Minecraft Server Appliance"/'  /tmp/profile/profiledef.sh

# 4) permissao correta do instalador na imagem final
sed -i '/^file_permissions=(/a\  ["/usr/local/bin/mc-install"]="0:0:755"' /tmp/profile/profiledef.sh
sed -i '/^file_permissions=(/a\  ["/opt/craftbox-panel/craftbox-panel"]="0:0:755"' /tmp/profile/profiledef.sh

# constroi
mkdir -p /out
mkarchiso -v -w /tmp/work -o /out /tmp/profile

# checksum + ajuste de permissao
cd /out
sha256sum ./*.iso | tee craftbox.sha256
chmod 644 ./*.iso ./*.sha256 2>/dev/null || true
ls -lh /out
