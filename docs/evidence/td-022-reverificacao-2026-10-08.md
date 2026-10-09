# TD-022 — Re-verificação no commit do reforço do TD-029 (R8.5: o override passa a token)
git-hash: c883875
watched: crates/led-console-bin/tests/ipc_contra_o_daemon.rs sha256:431cc9b0a64aca3f2fe442793b45a20e29a32402aee3917d483a4e48de8b3546
source_files: crates/led-console-bin/tests/ipc_contra_o_daemon.rs
required_test: o_daemon_recusa_a_linha_longa_por_si_proprio
data: 2026-10-08

# PORQUE EXISTE
#
# O reforço do TD-029 (R8.5) troca `Config.assume_no_wifi: bool` por `Option<AssumeNoWifi>`, um token que só
# o parser da CLI cria. O teste do TD-022 constrói um literal `Config {…}`, e por isso a linha
# `assume_no_wifi: false,` passou a `assume_no_wifi: None,`. É 1 linha em `subir()`, sem efeito no que o
# TD-022 prova. O conteúdo do ficheiro vigiado mudou, e o gate exige evidência regenerada. O sha256 acima é
# o do ficheiro neste commit. As evidências de 2026-09-28 e 2026-10-07 ficam como estão.

## 1. macOS local — C0 sobre o ficheiro com a linha nova

macOS 14.8.9 x86_64. Exit lido do processo, sem pipe (KB-013). Comando:
`cargo test -p led-console-bin --test ipc_contra_o_daemon --locked -- --exact o_daemon_recusa_a_linha_longa_por_si_proprio`

--- prova ---
```
test o_daemon_recusa_a_linha_longa_por_si_proprio ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
```
--- fim da prova ---

Exit `0`. Na mesma árvore, `cargo test --workspace --locked --no-fail-fast` deu exit `0`: 107 binários,
1186 passed; 0 failed; 9 ignored.

## 2. Controlo F1, re-executado a partir de uma cópia do commit c883875

Cópia por `git archive c883875` + touch, com CARGO_TARGET_DIR próprio. A mutação é a da evidência de 2026-10-07
§2: uma interrupção **forçada** (`MAX_BODY + 1` bytes sem `\n`, `flush`, 500 ms) seguida de `writeln!(…).unwrap()`
cru, no lugar do `if let Err(e)` que tolera só `BrokenPipe`.

| condição | exit | `error[E` | `panicked` |
|---|---|---|---|
| C0 | `0` | 0 | 0 |
| F1 ×3 | `101 101 101` | 0 | 1 cada |
| C0 depois (cópia limpa) | `0` | 0 | 0 |

Mensagem do F1: `called Result::unwrap() on an Err value: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }`.

## 3. O que isto NÃO re-mede

- Linux: aqui não. O job ubuntu da CI do PR corre o mesmo teste.
- F2 e F3: não re-executadas. Só mudou o literal de `Config`, fora do caminho que elas mutam.
