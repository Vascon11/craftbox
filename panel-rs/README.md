# panel-rs: backend do painel em Rust

Esta é a reescrita do `panel/server.js` como um binário único, com as **mesmas rotas**, então o frontend em `panel/public` continua o mesmo. O contrato completo e o status de cada rota estão em [`ROUTES.md`](ROUTES.md).

**Migração completa:** todas as rotas do Node estão no Rust (fatias 1 a 7 do `ROUTES.md`). São energia e console, backups (com download em streaming), conteúdo do Modrinth, criação de servidores Paper/Fabric/Forge/NeoForge/Pumpkin, modpacks do Modrinth e do CurseForge, integrações (playit, Cloudflare Tunnel, Tailscale), rede, contas e histórico. O **corte** (fatia 8) também está feito: a ISO embarca o binário e o instalador o usa no lugar do `server.js`, e a imagem Docker não tem mais Node (o `docker/init.js` virou `craftbox-panel --docker-init`). O `panel/server.js` continua no repositório como referência do teste de paridade e como fallback do instalador.

```sh
cargo build --release                                   # target/release/craftbox-panel
CRAFTBOX_PANEL_CONFIG=/caminho/config.json \
CRAFTBOX_PUBLIC_DIR=../panel/public ./target/release/craftbox-panel
cargo test --release                                    # testes unitários (48)
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

`parity/run.sh` sobe o Node (`panel/server.js`) e o Rust **lado a lado**, em portas altas livres e só em `127.0.0.1`. Os dois usam o **mesmo config** (cópias num diretório em `/tmp`; nunca `panel/config.json`, `/mnt/dados` nem a porta 8080). O script faz login nos dois e compara status HTTP, headers de enquadramento, `Set-Cookie`, a ordem das chaves e os valores do JSON. Os valores voláteis (load, memória, ms, uptime, timestamps) só têm o tipo conferido ou uma tolerância. Cenários:

- **A**: senha única, runner systemd: estáticos (bytes idênticos), login e as variações de erro, **cookie cruzado** (o cookie do Node vale no Rust e vice-versa), token adulterado ou expirado, `/api/status`, `/api/diag/mojang` e auditoria.
- **B**: multiusuário + multi-servidor, runner exec, com PID files de processos reais e um **servidor RCON falso**. Termina com o **shutdown gracioso** (SIGTERM).
- **C**: config sem `sessionSecret`: os dois geram o segredo e regravam o config.json **idêntico**.
- **D**: as fatias 2 a 7, com uma cópia idêntica dos dados pra cada lado (instância com mundo, logs, backups com `mtime` fixo, plugins e manifesto). Cobre ligar/reiniciar/parar pelo runner, logs com `?lines=` estranhos, backups (listar, baixar em streaming, restaurar, tar inválido), loja de conteúdo (toggle, remover, busca e projeto no Modrinth de verdade), criar/clonar/apagar servidor (inclusive o B1), criação real de um servidor Fabric, modpacks e CurseForge sem chave, integrações, rede, extras, usuários (1º usuário vira admin e ganha cookie), troca de senha e histórico. Depois de cada escrita, compara também **os arquivos gravados** (`server.properties`, meta, manifesto, `config.json`), mascarando só o que é aleatório.
- **Divergências documentadas**: um cookie malformado derruba o processo Node (B2); o Rust responde 401. No B5 o Rust mantém ip/usuário na auditoria de ligar/desligar; o comparador trata isso como esperado.

Último resultado: **216 verificações iguais, 0 diferenças.** Fora do harness, um modpack real (Fabulously Optimized 6.5.0, 50 arquivos) instalado nos dois backends gerou respostas byte a byte iguais e a mesma árvore de 120 arquivos; o servidor subiu e parou pelo runner do Rust com o mundo salvo.
