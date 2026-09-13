#!/usr/bin/env bash
# cpu-diag.sh — diagnostico do clamp de CPU do craftbox (Dell 3442 / i5-4210U preso em ~800 MHz).
#
# USO (no proprio Dell, como root):
#   sudo bash cpu-diag.sh              # mede o estado real (driver, governor, freq sob carga, PROCHOT)
#   sudo bash cpu-diag.sh --apply-acpi # forca acpi-cpufreq (intel_pstate=disable no systemd-boot) e pede reboot
#   sudo bash cpu-diag.sh --revert-acpi# desfaz o acima
#
# A ideia: se sob acpi-cpufreq a CPU passar de ~2 GHz, o cap era do driver/SO.
# Se continuar em ~800 MHz, o cap e firmware/EC (CR2032 + BIOS).

set -u
ENTRY=/boot/loader/entries/arch.conf
LOAD_SECS=3

die() { echo "ERRO: $*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || die "rode como root:  sudo bash $0 $*"

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------- apply/revert
apply_acpi() {
  [ -f "$ENTRY" ] || die "nao achei $ENTRY (esse modo e so pra craftbox/systemd-boot)"
  if grep -q 'intel_pstate=disable' "$ENTRY"; then
    echo "Ja tem intel_pstate=disable na entrada. Nada a fazer. (reboot se ainda nao reiniciou)"; exit 0
  fi
  cp -a "$ENTRY" "$ENTRY.bak.$(date +%s)" || die "falha no backup"
  sed -i -E 's/^(options .*)$/\1 intel_pstate=disable/' "$ENTRY"
  echo "== nova linha de options =="; grep '^options' "$ENTRY"
  echo
  echo ">> Feito. REINICIE o servidor:  sudo reboot"
  echo ">> Depois do boot, rode de novo:  sudo bash $0"
  exit 0
}
revert_acpi() {
  [ -f "$ENTRY" ] || die "nao achei $ENTRY"
  cp -a "$ENTRY" "$ENTRY.bak.$(date +%s)"
  sed -i -E 's/ *intel_pstate=disable//g' "$ENTRY"
  echo "== options revertida =="; grep '^options' "$ENTRY"
  echo ">> REINICIE:  sudo reboot"
  exit 0
}
install_fix() {
  echo "Instalando o fix permanente de BD PROCHOT..."
  if command -v pacman >/dev/null;  then pacman -Sy --noconfirm msr-tools >/dev/null 2>&1
  elif command -v apt-get >/dev/null; then apt-get -qq update >/dev/null 2>&1; apt-get -qq install -y msr-tools >/dev/null 2>&1; fi
  command -v wrmsr >/dev/null || die "nao consegui instalar msr-tools (sem rede?)"
  cat > /usr/local/bin/craftbox-bdprochot <<'S'
#!/usr/bin/env bash
# craftbox: desliga o BD PROCHOT (bit0 do MSR 0x1FC) — a CPU ignora PROCHOT externo
# espurio (bateria/EC) que a prendia no minimo. Protecao termica INTERNA segue ativa.
modprobe msr 2>/dev/null || true
cur=$(rdmsr -p0 0x1fc 2>/dev/null) || exit 0
[ -n "$cur" ] || exit 0
new=$(( 0x$cur & ~1 ))
wrmsr -a 0x1fc "$new" 2>/dev/null || exit 0
echo "craftbox-bdprochot: MSR 0x1FC 0x$cur -> $(printf '0x%x' "$new") (BD PROCHOT desligado)"
S
  chmod +x /usr/local/bin/craftbox-bdprochot
  cat > /etc/systemd/system/craftbox-bdprochot.service <<'S'
[Unit]
Description=craftbox: desliga BD PROCHOT (destrava CPU presa no minimo)
ConditionVirtualization=no
[Service]
Type=oneshot
ExecStart=/usr/local/bin/craftbox-bdprochot
RemainAfterExit=yes
[Install]
WantedBy=multi-user.target
S
  echo msr > /etc/modules-load.d/msr.conf 2>/dev/null || true
  systemctl daemon-reload
  systemctl enable --now craftbox-bdprochot
  echo ">> Fix instalado e ativo AGORA (e sobe em todo boot)."
  echo ">> Confirme com:  sudo bash $0"
  exit 0
}
case "${1:-}" in
  --apply-acpi)  apply_acpi ;;
  --revert-acpi) revert_acpi ;;
  --install-fix) install_fix ;;
  "" ) : ;;
  * ) die "opcao desconhecida: $1" ;;
esac

# ---------------------------------------------------------------- ferramentas
modprobe msr 2>/dev/null
if ! have rdmsr; then
  echo "rdmsr ausente; tentando instalar msr-tools..."
  pacman -Sy --noconfirm msr-tools >/dev/null 2>&1 || echo "  (nao instalou; APERF/MPERF fica indisponivel)"
fi
CPUDIR=/sys/devices/system/cpu/cpu0/cpufreq
rd()  { rdmsr -p0 "$1" 2>/dev/null; }          # hex
rdd() { rdmsr -p0 -d "$1" 2>/dev/null; }       # decimal
rdf() { rdmsr -p0 -d -f "$1" "$2" 2>/dev/null; } # campo de bits, decimal

echo "=========================================================="
echo " craftbox cpu-diag — $(date '+%F %T')"
echo "=========================================================="
echo "CPU        : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"
echo "nucleos    : $(nproc)"
echo "driver     : $(cat $CPUDIR/scaling_driver 2>/dev/null)"
echo "governor   : $(cat $CPUDIR/scaling_governor 2>/dev/null)"
echo "scaling min/max: $(( $(cat $CPUDIR/scaling_min_freq)/1000 )) / $(( $(cat $CPUDIR/cpuinfo_max_freq)/1000 )) MHz"
if grep -q intel_pstate=disable /proc/cmdline; then echo "cmdline    : intel_pstate=disable ATIVO (deve estar em acpi-cpufreq)"; else echo "cmdline    : intel_pstate NAO desabilitado"; fi

# base ratio (PLATFORM_INFO 0xCE bits 15:8) -> base MHz p/ converter APERF/MPERF
BASE_RATIO=$(rdf 15:8 0xce); BASE_MHZ=""
[ -n "${BASE_RATIO:-}" ] && BASE_MHZ=$(( BASE_RATIO * 100 ))
[ -n "$BASE_MHZ" ] && echo "base(nominal): $BASE_MHZ MHz (ratio $BASE_RATIO)"

# ---------------------------------------------------------------- forcar performance
echo; echo "-- forcando governor=performance em todos os nucleos --"
for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance > "$g" 2>/dev/null; done
echo "governor agora: $(cat $CPUDIR/scaling_governor)"

# ---------------------------------------------------------------- carga + medicao
echo; echo "-- aplicando carga ($(nproc)x yes) por ${LOAD_SECS}s e medindo --"
PIDS=()
for _ in $(seq "$(nproc)"); do yes >/dev/null & PIDS+=($!); done

A1=$(rdd 0xe8); M1=$(rdd 0xe7)   # APERF/MPERF antes
sleep "$LOAD_SECS"
A2=$(rdd 0xe8); M2=$(rdd 0xe7)   # depois
CUR_SYSFS=$(( $(cat $CPUDIR/scaling_cur_freq)/1000 ))
# snapshot da freq de todos os nucleos via /proc/cpuinfo
CORES_MHZ=$(awk '/MHz/{printf "%d ",$4}' /proc/cpuinfo)
THERM=$(rd 0x19c); PERFCTL=$(rd 0x199); PERFSTAT_RATIO=$(rdf 15:8 0x198)

kill "${PIDS[@]}" 2>/dev/null; wait 2>/dev/null

echo
echo "== RESULTADO =="
echo "scaling_cur_freq (cpu0): ${CUR_SYSFS} MHz"
echo "freq por nucleo (/proc/cpuinfo): ${CORES_MHZ}MHz"
REAL=""
if [ -n "${A1:-}" ] && [ -n "${A2:-}" ] && [ -n "$BASE_MHZ" ] && [ "$M2" != "$M1" ]; then
  # freq real = (dAPERF/dMPERF) * base_MHz   (medicao por hardware, nao mente)
  REAL=$(awk -v a=$((A2-A1)) -v m=$((M2-M1)) -v b="$BASE_MHZ" 'BEGIN{printf "%d", (a/m)*b}')
  echo "APERF/MPERF (freq REAL sob carga): ${REAL} MHz   [dAPERF=$((A2-A1)) dMPERF=$((M2-M1))]"
else
  echo "APERF/MPERF: indisponivel (rdmsr ausente?)"
fi
if [ -n "${PERFSTAT_RATIO:-}" ]; then echo "PERF_STATUS ratio executado: $PERFSTAT_RATIO (~$((PERFSTAT_RATIO*100)) MHz)"; fi
if [ -n "${PERFCTL:-}" ]; then echo "PERF_CTL solicitado (0x199): 0x$PERFCTL"; fi
if [ -n "${THERM:-}" ]; then
  TH=$((16#$THERM))
  echo "THERM_STATUS (0x19C): 0x$THERM"
  echo "   PROCHOT/FORCEPR agora (bit2): $(( (TH>>2)&1 ))   |   PROCHOT log/latch (bit3): $(( (TH>>3)&1 ))"
  echo "   thermal agora (bit0): $(( TH&1 ))   |   power-limit log (bit11): $(( (TH>>11)&1 ))"
fi

# ---------------------------------------------------------------- veredito
echo
echo "== VEREDITO =="
if [ -n "$REAL" ]; then
  if [ "$REAL" -ge 1500 ]; then
    echo "CPU chegou a ${REAL} MHz sob carga -> DESTRAVOU."
    if systemctl is-active --quiet craftbox-bdprochot 2>/dev/null; then
      echo "   O servico craftbox-bdprochot desligou o BD PROCHOT (0x1FC bit0) -> fix ativo."
    elif [ "$(( 16#$(rdmsr -p0 0x1fc 2>/dev/null || echo 1) & 1 ))" = 0 ]; then
      echo "   BD PROCHOT (0x1FC bit0) esta desligado -> foi isso que destravou."
    else
      echo "   Destravou (BD PROCHOT ainda ligado; pode ter sido reset do EC)."
    fi
  else
    echo "CPU ainda presa em ~${REAL} MHz sob carga -> CAP CONTINUA."
    PLATCH=$(( (16#${THERM:-0} >> 3) & 1 ))
    [ "$PLATCH" = 1 ] && echo "   PROCHOT latch setado + termais frios = firmware/EC. Caminho: trocar CR2032, atualizar BIOS."
    grep -q intel_pstate=disable /proc/cmdline \
      && echo "   Ja testou com acpi-cpufreq e nao adiantou -> descarta driver, e firmware." \
      || echo "   Ainda em intel_cpufreq: teste  sudo bash $0 --apply-acpi  e reveja."
  fi
else
  echo "Sem APERF/MPERF nao da veredito por hardware; olhe scaling_cur_freq acima (mas ele pode mentir)."
fi
echo "=========================================================="
