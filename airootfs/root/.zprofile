# Auto-inicia o instalador do servidor Minecraft ao ligar (apenas no console fisico)
if [[ "$(tty)" == /dev/tty1 ]]; then
    /usr/local/bin/mc-install
fi
