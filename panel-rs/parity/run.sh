#!/usr/bin/env bash
# Teste de paridade: sobe o painel Node (panel/server.js) e o Rust (panel-rs)
# lado a lado, em portas altas livres, com o MESMO config temporário (cópias
# idênticas; só o caminho do log de auditoria muda), e compara as respostas das
# rotas migradas. Nada fora de um diretório em /tmp é tocado: não usa
# panel/config.json, /mnt/dados nem a porta 8080. Escuta só em 127.0.0.1.
#
# Uso: panel-rs/parity/run.sh        (KEEP=1 mantém o diretório de trabalho)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RS="$(dirname "$HERE")"
REPO="$(dirname "$RS")"
NODE_SERVER="$REPO/panel/server.js"
PUBLIC="$REPO/panel/public"
BIN="$RS/target/release/craftbox-panel"

command -v node >/dev/null || { echo "precisa do node pra rodar o lado Node"; exit 2; }
if [ ! -x "$BIN" ] || [ -n "$(find "$RS/src" "$RS/Cargo.toml" -newer "$BIN" 2>/dev/null | head -1)" ]; then
  echo "compilando o binário Rust..."
  (cd "$RS" && cargo build --release -q) || exit 2
fi

WORK="$(mktemp -d /tmp/craftbox-parity.XXXXXX)"
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  if [ "${KEEP:-0}" = 1 ]; then echo "diretório mantido: $WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT

freeport() {
  local p
  while :; do
    p="$(node -e "const s=require('net').createServer();s.listen(0,'127.0.0.1',()=>{console.log(s.address().port);s.close()})")"
    [ "$p" != 8080 ] && [ "$p" -gt 20000 ] && { echo "$p"; return; }
  done
}
wait_port() { # porta, pid
  for _ in $(seq 1 100); do
    curl -s -o /dev/null "http://127.0.0.1:$1/api/authcheck" && return 0
    kill -0 "$2" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}
# inicia "node"|"rust" com um config; demais args = env extra. PID em $LAST_PID
# (processo filho direto deste shell, pra o `wait` devolver o código de saída).
start() {
  local kind="$1" cfg="$2" port="$3" log="$4"; shift 4
  if [ "$kind" = node ]; then
    env CRAFTBOX_PANEL_CONFIG="$cfg" CRAFTBOX_PORT="$port" "$@" node "$NODE_SERVER" >"$log" 2>&1 &
  else
    env CRAFTBOX_PANEL_CONFIG="$cfg" CRAFTBOX_PORT="$port" CRAFTBOX_PUBLIC_DIR="$PUBLIC" "$@" "$BIN" >"$log" 2>&1 &
  fi
  LAST_PID=$!
  PIDS+=("$LAST_PID")
}

PASSWORD='senha-de-teste-123'
SALT='00112233445566778899aabbccddeeff'
HASH="$(node -e "console.log(require('crypto').scryptSync(process.argv[1], process.argv[2], 64).toString('hex'))" "$PASSWORD" "$SALT")"
SECRET="$(node -e "console.log(require('crypto').randomBytes(32).toString('hex'))")"
export SUMMARY="$WORK/summary.txt" PASSWORD SECRET
: >"$SUMMARY"
STATUS=0

echo "craftbox — paridade Node × Rust"
echo "  node $(node --version) · $("$BIN" --version) ($(du -h "$BIN" | cut -f1)) · trabalho em $WORK"

# ---------------------------------------------------------------------------
# Cenário A: senha única, modo legado (mcDir + service), runner systemd
# ---------------------------------------------------------------------------
mkdir -p "$WORK/a/mc"
printf 'motd=paridade\nonline-mode=false\nenable-rcon=true\nrcon.port=1\n' >"$WORK/a/mc/server.properties"
for k in node rust; do
  cat >"$WORK/a/config-$k.json" <<EOF
{
  "host": "127.0.0.1",
  "mcDir": "$WORK/a/mc",
  "service": "craftbox-parity-inexistente",
  "auth": { "salt": "$SALT", "hash": "$HASH" },
  "sessionSecret": "$SECRET",
  "auditLog": "$WORK/a/audit-$k.log",
  "chaveDesconhecida": { "preservar": [1, 2.5, "x"] }
}
EOF
done
PN="$(freeport)"; PR="$(freeport)"; while [ "$PR" = "$PN" ]; do PR="$(freeport)"; done
start node "$WORK/a/config-node.json" "$PN" "$WORK/a/node.log"; NPID=$LAST_PID
start rust "$WORK/a/config-rust.json" "$PR" "$WORK/a/rust.log"; RPID=$LAST_PID
wait_port "$PN" "$NPID" || { echo "Node não subiu"; cat "$WORK/a/node.log"; exit 2; }
wait_port "$PR" "$RPID" || { echo "Rust não subiu"; cat "$WORK/a/rust.log"; exit 2; }
echo "  A: Node em :$PN (pid $NPID), Rust em :$PR (pid $RPID)"
SCENARIO=A NODE_BASE="http://127.0.0.1:$PN" RUST_BASE="http://127.0.0.1:$PR" \
  AUDIT_NODE="$WORK/a/audit-node.log" AUDIT_RUST="$WORK/a/audit-rust.log" \
  node "$HERE/compare.mjs" || STATUS=1
SCENARIO=COOKIE NODE_BASE="http://127.0.0.1:$PN" RUST_BASE="http://127.0.0.1:$PR" NODE_PID="$NPID" \
  node "$HERE/compare.mjs" || STATUS=1
kill "$NPID" "$RPID" 2>/dev/null; wait "$NPID" "$RPID" 2>/dev/null

# ---------------------------------------------------------------------------
# Cenário B: multiusuário + multi-servidor, runner exec (sem systemd)
#   alpha: rodando (PID file de um `sleep`), RCON via server.properties
#   gamma: rodando, RCON via meta da instância (estilo Pumpkin), sem server.properties
#   beta : parado; activeServer = beta
# ---------------------------------------------------------------------------
RCONP="$(freeport)"
node "$HERE/fake-rcon.mjs" "$RCONP" segredo >"$WORK/rcon.log" 2>&1 & PIDS+=("$!")
S="$WORK/b/servers"; mkdir -p "$S/alpha" "$S/beta" "$S/gamma"
echo '{"name":"Alpha","loader":"paper","mcVersion":"1.21.1","port":25565}' >"$S/alpha/.craftbox-instance.json"
printf 'online-mode=true\nenable-rcon=true\nrcon.port=%s\nrcon.password=segredo\n' "$RCONP" >"$S/alpha/server.properties"
printf 'online-mode=false\n' >"$S/beta/server.properties"
echo "{\"name\":\"Gamma\",\"loader\":\"pumpkin\",\"rconPort\":$RCONP,\"rconPassword\":\"segredo\"}" >"$S/gamma/.craftbox-instance.json"
STARTED="$(node -e 'console.log(Date.now() - 42000)')"
declare -A SLEEPS
for k in node rust; do
  mkdir -p "$WORK/b/run-$k"
  for inst in alpha gamma; do
    sleep 600 & SP=$!; PIDS+=("$SP"); SLEEPS["$k-$inst"]=$SP
    printf '%s\n%s\n' "$SP" "$STARTED" >"$WORK/b/run-$k/minecraft@$inst.pid"
  done
  cat >"$WORK/b/config-$k.json" <<EOF
{
  "host": "127.0.0.1",
  "users": [
    { "user": "admin", "salt": "$SALT", "hash": "$HASH", "role": "admin" },
    { "user": "Joao", "salt": "$SALT", "hash": "$HASH", "role": "user" }
  ],
  "sessionSecret": "$SECRET",
  "activeServer": "beta",
  "auditLog": "$WORK/b/audit-$k.log"
}
EOF
done
PN="$(freeport)"; PR="$(freeport)"; while [ "$PR" = "$PN" ]; do PR="$(freeport)"; done
start node "$WORK/b/config-node.json" "$PN" "$WORK/b/node.log" CRAFTBOX_RUNNER=exec CRAFTBOX_SERVERS_DIR="$S" CRAFTBOX_RUN_DIR="$WORK/b/run-node"; NPID=$LAST_PID
start rust "$WORK/b/config-rust.json" "$PR" "$WORK/b/rust.log" CRAFTBOX_RUNNER=exec CRAFTBOX_SERVERS_DIR="$S" CRAFTBOX_RUN_DIR="$WORK/b/run-rust"; RPID=$LAST_PID
wait_port "$PN" "$NPID" || { echo "Node não subiu"; cat "$WORK/b/node.log"; exit 2; }
wait_port "$PR" "$RPID" || { echo "Rust não subiu"; cat "$WORK/b/rust.log"; exit 2; }
echo "  B: Node em :$PN, Rust em :$PR, RCON falso em :$RCONP"
SCENARIO=B NODE_BASE="http://127.0.0.1:$PN" RUST_BASE="http://127.0.0.1:$PR" \
  AUDIT_NODE="$WORK/b/audit-node.log" AUDIT_RUST="$WORK/b/audit-rust.log" \
  node "$HERE/compare.mjs" || STATUS=1

echo
echo "== Shutdown gracioso (SIGTERM, runner exec) =="
for k in node rust; do
  if [ $k = node ]; then P=$NPID; else P=$RPID; fi
  T0=$(date +%s%N); kill -TERM "$P"; wait "$P"; RC=$?; T1=$(date +%s%N)
  DEAD=0; for inst in alpha gamma; do kill -0 "${SLEEPS[$k-$inst]}" 2>/dev/null || DEAD=$((DEAD+1)); done
  printf '  %-4s saiu com código %s em %s ms; processos gerenciados parados: %s/2\n' "$k" "$RC" $(( (T1-T0)/1000000 )) "$DEAD"
  if [ $k = rust ] && { [ "$RC" != 0 ] || [ "$DEAD" != 2 ]; }; then STATUS=1; echo "  FALHA shutdown do Rust"; fi
done

# ---------------------------------------------------------------------------
# Cenário C: config sem sessionSecret → os dois geram e regravam o arquivo.
# Compara o texto gravado (com o segredo mascarado), incluindo o override de
# CRAFTBOX_PORT/CRAFTBOX_SERVERS_DIR persistido e a chave desconhecida.
# ---------------------------------------------------------------------------
echo
echo "== Cenário C: geração do sessionSecret e regravação do config.json =="
for k in node rust; do
  printf '{"host":"127.0.0.1","extra":{"n":[1,2.5,1e21,0.1]},"auth":{"hash":"h"},"rcon":{"port":25580}}' >"$WORK/c-$k.json"
done
PN="$(freeport)"; PR="$(freeport)"; while [ "$PR" = "$PN" ]; do PR="$(freeport)"; done
start node "$WORK/c-node.json" "$PN" "$WORK/c-node.log" CRAFTBOX_SERVERS_DIR="$WORK/c-servers"; NPID=$LAST_PID
start rust "$WORK/c-rust.json" "$PR" "$WORK/c-rust.log" CRAFTBOX_SERVERS_DIR="$WORK/c-servers"; RPID=$LAST_PID
wait_port "$PN" "$NPID"; wait_port "$PR" "$RPID"
kill "$NPID" "$RPID" 2>/dev/null; wait "$NPID" "$RPID" 2>/dev/null
mask() { sed -E 's/"sessionSecret": "[0-9a-f]{64}"/"sessionSecret": "<64 hex>"/; s/"port": '"$2"',/"port": <CRAFTBOX_PORT>,/' "$1"; }
if diff <(mask "$WORK/c-node.json" "$PN") <(mask "$WORK/c-rust.json" "$PR") >"$WORK/c.diff"; then
  echo "  OK   config regravado idêntico ($(wc -c <"$WORK/c-rust.json") bytes; segredo de 64 hex gerado nos dois)"
  echo "C 1 0 0" >>"$SUMMARY"
else
  echo "  FALHA config regravado difere:"; sed 's/^/    /' "$WORK/c.diff"; STATUS=1
  echo "C 0 1 0" >>"$SUMMARY"
fi

echo
awk '{o+=$2; f+=$3; n+=$4} END {printf "RESUMO: %d ok, %d falha(s), %d nota(s)\n", o, f, n}' "$SUMMARY"
exit $STATUS
