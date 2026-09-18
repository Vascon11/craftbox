# syntax=docker/dockerfile:1
# =============================================================================
# craftbox — painel web + runtime Minecraft em um único container.
# Multi-estágio: JDKs (8/17/21/25) do Adoptium são copiadas para o container final.
#
# Modo container (sem systemd): o painel gerencia os servidores e túneis
# diretamente via child_process (CRAFTBOX_RUNNER=exec) — sem Docker socket.
# =============================================================================
FROM eclipse-temurin:8-jre AS jdk8
FROM eclipse-temurin:17-jre AS jdk17
FROM eclipse-temurin:21-jre AS jdk21
FROM eclipse-temurin:25-jre AS jdk25

# Node 22 (LTS em manutenção; o 20 saiu de suporte em abr/2026)
FROM node:22-bookworm-slim AS base

ARG TARGETARCH=amd64

LABEL org.opencontainers.image.title="craftbox" \
      org.opencontainers.image.description="Painel web + runtime Minecraft (JDK 8/17/21/25, playit, cloudflared, tailscale)" \
      org.opencontainers.image.source="https://github.com/Vascon11/craftbox" \
      org.opencontainers.image.licenses="MIT"

ENV DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

# Dependências do sistema: ferramentas básicas.
RUN apt-get update && apt-get install -y --no-install-recommends \
      bash coreutils curl ca-certificates file tar gzip xz-utils unzip procps \
    && rm -rf /var/lib/apt/lists/*

# Tailscale (CLI p/ o painel comandar o Tailscale do host; no container o uso é
# opcional — só com NET_ADMIN+/dev/net/tun). Falha não derruba o build.
RUN curl -fsSL https://tailscale.com/install.sh | sh || true

# Binários de túnel pré-baixados (playit + cloudflared) — o entrypoint semeia
# no integrationsDir na primeira subida, onde o painel já os detecta.
# a arquitetura é detectada pelo buildkit (TARGETARCH); override opcional via args.
ARG PLAYIT_ARCH=
ARG CLOUDFLARED_ARCH=
RUN mkdir -p /usr/local/lib/craftbox/tunnels \
    && if [ -z "$PLAYIT_ARCH" ]; then case "$TARGETARCH" in arm64) PLAYIT_ARCH=aarch64; CLOUDFLARED_ARCH=arm64 ;; *) PLAYIT_ARCH=amd64; CLOUDFLARED_ARCH=amd64 ;; esac; fi \
    && curl -fsSL -o /usr/local/lib/craftbox/tunnels/playit \
       https://github.com/playit-cloud/playit-agent/releases/latest/download/playit-linux-$PLAYIT_ARCH \
    && curl -fsSL -o /usr/local/lib/craftbox/tunnels/cloudflared \
       https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-$CLOUDFLARED_ARCH \
    && chmod 0755 /usr/local/lib/craftbox/tunnels/playit /usr/local/lib/craftbox/tunnels/cloudflared

# JDKs copiadas dos estágios (8 = MC <=1.16, 17 = 1.17–1.20.4, 21 = 1.20.5+/1.21.x,
# 25 = versões por calendário 26.1+)
COPY --from=jdk8  /opt/java/openjdk /usr/local/lib/jvm/jdk8
COPY --from=jdk17 /opt/java/openjdk /usr/local/lib/jvm/jdk17
COPY --from=jdk21 /opt/java/openjdk /usr/local/lib/jvm/jdk21
COPY --from=jdk25 /opt/java/openjdk /usr/local/lib/jvm/jdk25

# wrapper 'java': /usr/bin/java e o start.sh das instâncias chamam este script,
# que escolhe a JDK certa por instância via CRAFTBOX_JAVA_MAJOR (setado pelo painel).
COPY docker/java-select /usr/local/bin/java
RUN chmod +x /usr/local/bin/java \
    && ln -sf /usr/local/bin/java /usr/bin/java \
    && for v in 8 17 21 25; do \
         out=$(CRAFTBOX_JAVA_MAJOR=$v java -version 2>&1) || { echo "JDK $v quebrada"; exit 1; }; \
         echo "JDK $v: $(echo "$out" | head -1)"; \
       done

# Aplicação
WORKDIR /app
COPY --chmod=0755 docker/entrypoint.sh /usr/local/bin/entrypoint
COPY docker/init.js /app/docker/init.js
COPY panel/ /app/

# Configuração padrão do container (sobrescrevável por env no compose)
ENV CRAFTBOX_RUNNER=exec \
    CRAFTBOX_PANEL_CONFIG=/data/config.json \
    CRAFTBOX_SERVERS_DIR=/data/servers \
    CRAFTBOX_INTEGRATIONS_DIR=/data/craftbox-integrations \
    CRAFTBOX_RUN_DIR=/data/craftbox-run \
    CRAFTBOX_AUTOSTART=

# volume persistente: instâncias, config global, túneis e PID/logs do runner
VOLUME /data

EXPOSE 8080 25565-25575/tcp 25565-25575/udp

# saúde = painel respondendo HTTP (o compose pode sobrescrever). start-period
# cobre o init.js; a porta segue CRAFTBOX_PORT (default 8080).
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:'+(process.env.CRAFTBOX_PORT||8080)+'/').then(r=>process.exit(r.status<500?0:1)).catch(()=>process.exit(1))"

ENTRYPOINT ["/usr/local/bin/entrypoint"]