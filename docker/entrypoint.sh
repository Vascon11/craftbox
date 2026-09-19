#!/bin/sh
# entrypoint do container craftbox: prepara /data e sobe o painel (binário Rust).
set -e
/app/craftbox-panel --docker-init
exec /app/craftbox-panel
