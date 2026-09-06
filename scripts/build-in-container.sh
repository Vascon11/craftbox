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

# 3) identidade da ISO
sed -i 's/^iso_name=.*/iso_name="craftbox"/'                                   /tmp/profile/profiledef.sh
sed -i "s/^iso_label=.*/iso_label=\"CRAFTBOX_$(date +%Y%m)\"/"                 /tmp/profile/profiledef.sh
sed -i 's/^iso_publisher=.*/iso_publisher="craftbox (github.com\/Vascon11\/craftbox)"/' /tmp/profile/profiledef.sh
sed -i 's/^iso_application=.*/iso_application="craftbox - Minecraft Server Appliance"/'  /tmp/profile/profiledef.sh

# 4) permissao correta do instalador na imagem final
sed -i '/^file_permissions=(/a\  ["/usr/local/bin/mc-install"]="0:0:755"' /tmp/profile/profiledef.sh

# constroi
mkdir -p /out
mkarchiso -v -w /tmp/work -o /out /tmp/profile

# checksum + ajuste de permissao
cd /out
sha256sum ./*.iso | tee craftbox.sha256
chmod 644 ./*.iso ./*.sha256 2>/dev/null || true
ls -lh /out
