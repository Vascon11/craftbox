# Diagnóstico — CPU preso em 800 MHz (servidor `axos-pudding`)

## Contexto

Máquina-alvo: **Dell Inspiron 3442**, i5-4210U (Haswell, base 1.7 GHz / turbo 2.7 GHz), rodando craftbox (Arch Linux).
Sintoma relatado: "o CPU não passa de 800 MHz".

## Linha do tempo

| Quando | O que foi feito | Resultado |
|---|---|---|
| 16:05–16:24 | Instalação + Tailscale | — |
| 16:24 | Primeira sondagem via SSH | Servidor Minecraft `minecraft@axos-pudding` **ativo**; CPU em `scaling_cur_freq = 798 MHz`; governador `schedutil`; driver `intel_cpufreq` (passivo). |
| 16:24 | Teste de carga (4× `yes`, load 1.29) | Todos os núcleos **colados em 798 MHz**, mesmo sob carga. |
| 16:38 | Checagem térmica/energia | Temperaturas 42 °C (frias), AC `online=1`, **bateria ausente**. Sem throttling térmico real. |
| 16:45 | Teste governador `performance` + carga | **Continua em 798 MHz** → descartada hipótese de bug do `schedutil`. |
| 16:45 | Sudo travado por faillock | Causado por comando meu (password errada 3× no `sudo -S`). Resolvido com reboot. |
| 17:0x | Leitura de MSRs sob carga + `performance` | Ver "Leitura de MSRs" abaixo. |

> Observação: entre os testes, o acesso SSH foi perdido por lockout do `pam_faillock`
> (minha tentativa de `sudo -S` passou o script inteiro como senha). Voltou após reboot.

## Leitura de MSRs (sob carga, governador `performance`)

Ferramentas: `msr-tools`, `linux-cpupower` (na verdade `cpupower`), `dmidecode` (BSD-3 da Dell são vendor tools).

| MSR | Valor | Decodificação |
|---|---|---|
| 0x199 PERF_CTL | `1b00` | **Solicitado:** ratio 0x1B = 27 → **2700 MHz (turbo)** |
| 0x198 PERF_STATUS | `159800000800` | **Executado:** ratio 0x15 = 21 → **2100 MHz** (hardware se reporta acima do mínimo, mas sysfs mostra 798) |
| 0x19C IA32_THERM_STATUS | `8836080c` | bit 2 (PROCHOT latch) = **SET**; bit 0/1 = 0 (sem calor real) |
| 0x610 PKG_POWER_LIMIT | `804280c800dd80c8` | PL1 habilitado = **25 W** (sem cap estranho de potência) |
| 0x614 PKG_POWER_STATUS | `78` | ~ 0x78 unidades (potência atual baixa, não disparou cap de 25 W) |

### Interpretação

- O **SO pede turbo (2700 MHz)** mas o hardware não sobe → o cap **não é do Linux**.
- O **latch de PROCHOT setado** aponta para throttling vindo do **Embedded Controller (EC)** da Dell,
  um sintoma clássico de:
  1. **Fonte AC fraca/errada** (o 3442 pede adaptador de **45W**; com fonte menor o EC capa em 800 MHz);
  2. **Configuração de energia no BIOS** ("Max Performance" vs. economizador);
  3. **EC com estado preso** (corrigido com *discharge reset*, descrito abaixo);
  4. BIOS antiga (A12, 2016).

## Conclusão (definitiva — 17:0x)

Travamento **100% por hardware/firmware (EC da Dell)** — não é o governador do kernel.

Prova final — **teste `userspace` com frequência forçada** (`cpupower frequency-set -f 2700000`):
- governador `userspace`, pedido explícito de **2700 MHz** → núcleos continuam em **798 MHz**;
- sanity check com `800000` → 798 MHz.
- Ou seja: o CPU **ignora qualquer pedido de frequência do SO**. Só pode ser capada no EC/firmware.

`turbostat` não está disponível (pacote não instalado) e o teste `intel_pstate=disable` **deixa de ser útil**:
pedir frequência explicitamente já prova o cap. Não recomendo reboot do servidor pra isso.

## Re-medicação pós-reset do EC (17:3x)

Após o reset de descarga do EC (AC fora, power 30 s), com a bateria principal já fora:

- Núcleos seguem em **798 MHz** sob carga (4× `yes`).
- **APERF/MPERF** (medição real por hardware, via `rdmsr` 0xE8/0xE7): deltas
  `ΔAPERF=2600872898 / ΔMPERF=5534856303 → ratio 0.4699 → 799 MHz` **reais** sob carga.
  → o sysfs não estava mentindo: o CPU está de fato em ~800 MHz.
- **MSR 0x19C `8835080c`** → **latch de PROCHOT continua setado** mesmo após reset do EC.
- **Fan**: `dell_smm fan1_input = 0 rpm` / `pwm1 = 0` (normal a 45 °C; consequência e não causa).
- Termais ~44–47 °C (frias).

### Interpretação final

Cap **ativo e contínuo vindo do EC** (PROCHOT real), com termais normais e fonte 65 W boa.
Reset de EC não resolveu. Sobra a faixa de: sensor/VRM com leitura errada, EC corrompido,
ou configuração de fábrica do firmware (CMOS/BIOS). Próximo teste decisivo: **boot num live USB e estressar lá**.

## Ações no lado físico (pendentes — precisam de você)

1. **Conferir o carregador** — ~~o Dell Inspiron 3442 pede o adaptador de **45W**~~ → usuário confirmou: **fonte de 65W** (suficiente, descarta hipótese de fonte fraca).
2. **Reset do EC (discharge)**: desligar → tirar o cabo AC (e a bateria, se houver) →
   segurar o botão de power por 30 s → religar. *(usuário já tirou a bateria; só falta tirar o AC e segurar o botão)*.
3. **BIOS → Power Management** — usuário não encontrou opção de "Max Performance"; ok, substituído pelos itens abaixo.
4. **Atualização de BIOS** — a instalada é a **A12 (05/2016)**; conferir se a Dell tem versão mais nova pro Inspiron 3442.

## Novas evidências (relato do usuário, 17:2x)

- Fonte de **65W** (descarta cap por adaptador fraco).
- Bateria principal **já estava removida** durante todos os testes (probe de 16:38 já mostrava só `AC`).
- O notebook emite **4 bipes** no boot; usuário acredita ser **bateria CMOS/BIOS** descarregada.
- Nota: o cap persistiu o tempo todo com AC 65W sem bateria → o EC não estava "confuso por bateria", o cap é outra coisa (provável estado preso do EC / CMOS).

### Hipótese com CMOS/4 bipes (a investigar)

Nos Dells, além do beep=RAM, há relatos de **bateria CMOS morta** causando:
- beeps no boot (estado de configuração corrompido) e
- **EC segurando o CPU em frequência mínima** até que se faça o ciclo de drenagem da lógica (discharge reset).

Ação recomendada:
1. **Agora:** desligar → **tirar o AC** → segurar power 30 s → religar → eu re-meço a frequência.
2. **Trocar a bateria CMOS (CR2032)** — custo baixo; elimina os bipes e o reset de data.

## Teste acpi-cpufreq vs intel_cpufreq (12/09/2026, 18:4x) — DRIVER DESCARTADO

Rodado o `cpu-diag.sh` no próprio Dell, antes e depois de `intel_pstate=disable`:

| | `intel_cpufreq` (passivo) | `acpi-cpufreq` (intel_pstate=disable) |
|---|---|---|
| Freq REAL (APERF/MPERF, sob carga) | **796 MHz** | **795 MHz** |
| scaling min/max | 800 / 2700 | 782 / **1701** (perdeu o turbo até da visão do driver) |
| PERF_CTL solicitado | `0x1b00` (pede 2700) | `0x1100` (pede 1700) |
| THERM_STATUS 0x19C | `8836080c` | `8834080c` |
| PROCHOT agora (bit2) / latch (bit3) | **1 / 1** | **1 / 1** |

**Conclusão:** trocar o driver **não muda nada** (795–796 MHz nas duas). A hipótese
"era o intel_pstate passivo" está **descartada**. Pior: sob `acpi-cpufreq` o teto
cai pra 1701 MHz — então convém **reverter** (`--revert-acpi` + reboot) pra voltar
ao `intel_cpufreq`, que pelo menos enxerga o turbo pra quando destravar.

### Achado novo importante: PROCHOT **bit2 = 1 (ativo AGORA)**, não só latch

O `bit2` (PROCHOT/FORCEPR *no momento*) está setado nas duas medições, com termais
frios (`bit0 = 0`). Ou seja: **algo está afirmando PROCHOT continuamente**, não é um
latch velho preso. PROCHOT externo assertado + CPU fria = fonte externa (EC/VRM/sensor).

### Hipótese principal agora: clamp por AUSÊNCIA de bateria (comportamento Dell)

Todos os testes rodaram com a **bateria removida**. Vários Dell/Inspiron **capam a CPU
no mínimo quando estão só no AC sem bateria (ou com bateria morta)** — o EC usa a
bateria como *buffer* pros picos de corrente do turbo; sem ela, ele assere PROCHOT
pra evitar brownout do adaptador, mesmo com fonte boa. Isso casa com tudo aqui
(fonte 65W ok, termais frios, PROCHOT contínuo, bateria fora o tempo todo).

**Próximo teste (o mais provável e barato):** colocar uma **bateria carregada e sã**
de volta e rodar `cpu-diag.sh` de novo. Se destravar → era isso.
Depois, em ordem: **CR2032** (4 bipes/RTC = CMOS morta) e **BIOS A12 → atual**.

## Artefatos

- Scripts usados: `/tmp/opencode/cpu_test.sh`, `/tmp/opencode/perf_test.sh`, `/tmp/opencode/hw_probe.sh`, `/tmp/opencode/msr_probe.sh`, `/tmp/opencode/userspace_test.sh` (máquina local de onde os testes rodaram).
- Pacotes instalados no servidor durante o diagnóstico: `msr-tools`, `cpupower`, `dmidecode`.