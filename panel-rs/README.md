# panel-rs: backend do painel em Rust (fase 1)

Esta é a reescrita do `panel/server.js` como um binário único, com as **mesmas rotas**, então o frontend em `panel/public` continua o mesmo. O contrato completo e o status de cada rota estão em [`ROUTES.md`](ROUTES.md).

**Fase 1 (esta):** a fundação. Já estão migrados a carga de config, o login e a sessão, os arquivos estáticos, `/api/status`, `/api/diag/mojang`, as respostas 401/404, o shutdown gracioso e a CLI (`--hash`, `--init`, `--version`). As outras rotas respondem `501` até serem migradas.

```sh
cargo build --release                                   # target/release/craftbox-panel
CRAFTBOX_PANEL_CONFIG=/caminho/config.json \
CRAFTBOX_PUBLIC_DIR=../panel/public ./target/release/craftbox-panel
cargo test --release                                    # testes unitários
parity/run.sh                                           # paridade Node × Rust (ver abaixo)
```

## Decisões

| tema | escolha | por quê |
|---|---|---|
| HTTP (servidor) | **servidor HTTP/1.1 próprio sobre `std::net`, uma thread por conexão** (`src/http.rs`, ~400 linhas) | O painel atende 1–3 abas com polling de 3 s, e algumas rotas ficam **minutos** bloqueadas em disco, rede ou processo filho (modpack, backup, instalador `java`). Com uma thread por conexão isso sai de graça, sem runtime async. Também dá controle total do enquadramento, que o teste confere igual ao do Node: `Keep-Alive: timeout=5`, `Transfer-Encoding: chunked` nos estáticos, 400 com close. `axum`/`hyper` trariam `tokio` + `tower` e dezenas de crates a mais para compilar no Haswell, sem ganho para esta carga. |
| Runtime | **nenhum** (threads + `sigwait` para sinais) | Mesmo motivo. O shutdown usa uma thread dedicada com `sigwait(SIGTERM/SIGINT)`. |
| HTTPS de saída | **`ureq` 3 + `rustls` + `ring` + `webpki-roots`** | É um cliente bloqueante, então combina com o modelo de threads. Com TLS em Rust puro e as raízes da Mozilla embutidas, o binário só depende da `libc` (`ldd`: libc + libgcc_s). Assim o **binário pré-compilado de fallback** roda em qualquer distro ou imagem Docker, sem briga de versão de `libssl`, e dá para gerar um estático com musl. O `native-tls` (OpenSSL) compilaria ~25 s mais rápido com `-j2`, mas amarraria o binário à `libssl.so` do sistema. |
| Criptografia | **`ring`** (HMAC-SHA256, PBKDF2, RNG), já trazido pelo rustls; o scrypt é a composição da RFC 7914 em cima dele (~60 linhas) | Nenhuma crate de criptografia a mais. Validado com os vetores da RFC 7914 e com hashes gerados pelo próprio Node (`parity/fixtures`). |
| JSON | **módulo próprio com semântica de JS** (`src/json.rs`) | O contrato é "o mesmo JSON do Node": todo número é f64 formatado como `Number#toString` (`12`, não `12.0`), a ordem de inserção é preservada (com as chaves numéricas primeiro, como no V8) e o config é regravado com `JSON.stringify(cfg,null,2)`, byte a byte. Evita `serde` + `syn` na compilação. |
| Syscalls | `libc` | `statfs` (disco com `bavail`), `kill(pid,0)`, `sigwait`, `mktime`/`gmtime`. |

São 3 dependências diretas (`libc`, `ring`, `ureq`) e 25 crates no total.

## Métricas (medidas nesta máquina: Ryzen 7 3700U, 4C/8T)

| build | tempo (do zero, crates já baixadas) | binário |
|---|---|---|
| `cargo build --release` (8 threads) | 45 s | 2,9 MB |
| `RUSTFLAGS="-C target-cpu=native" cargo build --release -j2` com `taskset` em 2 núcleos (simula a instalação) | **60 s** (pico de ~400 MB de RAM) | 2,9 MB |
| `cargo build --profile dist` (LTO fat, `codegen-units=1`, 8 threads) | 60 s | 2,4 MB |

Estimativa para o i5-4210U (Haswell, 2C/4T, 1,7–2,7 GHz): **~1,5 a 2,5 min** para a build de instalação. Metade do tempo é o `ring` (C/assembly) + `rustls`.

Em execução (mesmo config, depois de 50 requisições de status e de estáticos): **Node ~77 MB de RSS, Rust ~4 MB**.

## Compilar na instalação (plano para o corte)

1. A ISO leva a toolchain e as crates **vendorizadas** (`cargo vendor`), porque a instalação pode estar offline.
2. `RUSTFLAGS="-C target-cpu=native" cargo build --release --locked --offline`
3. Se falhar (ou se não houver toolchain), usa o binário pré-compilado do perfil `dist`, feito para `x86-64` base (sem `native`, para rodar em qualquer CPU).

## Teste de paridade

`parity/run.sh` sobe o Node (`panel/server.js`) e o Rust **lado a lado**, em portas altas livres e só em `127.0.0.1`. Os dois usam o **mesmo config** (cópias num diretório em `/tmp`; nunca `panel/config.json`, `/mnt/dados` nem a porta 8080). O script faz login nos dois e compara status HTTP, headers de enquadramento, `Set-Cookie`, a ordem das chaves e os valores do JSON. Os valores voláteis (load, memória, ms, uptime) só têm o tipo conferido ou uma tolerância. Cenários cobertos:

- **A**: senha única, runner systemd: estáticos (bytes idênticos), login e as variações de erro, **cookie cruzado** (o cookie do Node vale no Rust e vice-versa), token adulterado ou expirado, `/api/status`, `/api/diag/mojang` e auditoria.
- **B**: multiusuário + multi-servidor, runner exec, com PID files de processos reais e um **servidor RCON falso** (`players` via `server.properties` e via meta da instância). Termina com o **shutdown gracioso** (SIGTERM): os dois param os processos gerenciados e saem com código 0.
- **C**: config sem `sessionSecret`: os dois geram o segredo e regravam o config.json **idêntico** (segredo mascarado).
- **Divergência documentada**: um cookie malformado derruba o processo Node (bug B2 no ROUTES.md); o Rust responde 401.

O número de verificações e o último resultado real estão no relatório da fase.
