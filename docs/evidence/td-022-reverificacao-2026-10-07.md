# TD-022 — Re-verificação no commit do TD-029 (o ficheiro vigiado ganhou 1 linha)
git-hash: 24d5b91
watched: crates/led-console-bin/tests/ipc_contra_o_daemon.rs sha256:40397a4f5fc891a80193ebbb3142f70fb515c0a11593dfb2bd5523a7b7e82415
source_files: crates/led-console-bin/tests/ipc_contra_o_daemon.rs
required_test: o_daemon_recusa_a_linha_longa_por_si_proprio
data: 2026-10-07

# PORQUE EXISTE
#
# O TD-029 acrescenta `Config.assume_no_wifi`. O teste do TD-022 constrói um literal `Config {…}`
# e por isso ganhou `assume_no_wifi: false,` (1 linha em `subir()`, sem efeito no que o TD-022
# prova). O conteúdo do ficheiro vigiado mudou → o gate exige evidência regenerada. Medido sobre
# `24d5b91` (main com o #26) + o diff do TD-029 na árvore de trabalho — o mesmo conteúdo que este
# commit fixa no sha256 acima. A evidência de 2026-09-28 fica como está.

## 1. macOS local — C0 sobre o ficheiro com a linha nova

macOS 14.8.9 x86_64. Exit lido do processo, sem pipe (KB-013). Comando:
`cargo test -p led-console-bin --test ipc_contra_o_daemon --locked -- --exact o_daemon_recusa_a_linha_longa_por_si_proprio`

--- prova ---
```
test o_daemon_recusa_a_linha_longa_por_si_proprio ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
```
--- fim da prova ---

Exit `0`. Na mesma árvore, `cargo test --workspace --locked`: exit `0`, 107 binários,
1161 passed; 0 failed; 9 ignored.

## 2. Controlo F1, re-executado sobre o mesmo ficheiro

Mutação: interrupção **forçada** (`MAX_BODY + 1` bytes sem `\n`, `flush`, 500 ms) e depois
`writeln!(…).unwrap()` cru, no lugar do `if let Err(e)` que tolera só `BrokenPipe`.

| condição | exit | `error[E` | `panicked` |
|---|---|---|---|
| C0 | `0` | 0 | 0 |
| F1 ×3 | `101 101 101` | 0 | 1 cada |
| C0 após reverter | `0` | 0 | 0 |

F1: `called Result::unwrap() on an Err value: Os { code: 32, kind: BrokenPipe, … }` (a linha
indicada pelo pânico é a do ficheiro **mutado**). Reversão por cópia do original;
`shasum -a 256 -c` → OK antes do C0 final.

## 3. O que isto NÃO re-mede

- Linux: não re-medido aqui; o job ubuntu da CI do PR do TD-029 corre o mesmo teste.
- F2 e F3 não re-executadas (o que mudou foi só o literal de `Config`, fora do caminho que elas mutam).
- Nada em hardware — o TD-022 é de teste, não de fio.
