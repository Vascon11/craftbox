#!/usr/bin/env bash
#
# Build local da ISO do craftbox usando Docker (precisa de daemon rootful).
# Uso:  ./build.sh
# Saida: ./out/craftbox-*.iso
#
# Obs: usa Docker (nao Podman) porque o mkarchiso precisa de bind-mounts de
# /dev, /proc etc. — Podman rootless nao consegue, mesmo com --privileged.
#
set -euo pipefail
REPO="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$REPO/out"

echo "==> Compilando ISO do craftbox (isso demora ~10-25 min)..."
docker run --rm --privileged \
  -v "$REPO":/build \
  -v "$REPO/out":/out \
  docker.io/library/archlinux:latest \
  bash /build/scripts/build-in-container.sh

echo
echo "==> Pronto! ISO em: $REPO/out/"
ls -lh "$REPO/out/"*.iso
