#!/bin/sh
# entrypoint do container craftbox: prepara /data e sobe o painel.
set -e
node /app/docker/init.js
exec node /app/server.js