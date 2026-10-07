# TD-022 — Re-verificação sobre `e46151b` (o #7 depois de `gh pr update-branch`)
git-hash: e46151b
watched: crates/led-console-bin/tests/ipc_contra_o_daemon.rs sha256:162ee0828a0f16c2ef5ec3fc6ed386b54855d78e906f9ed5e03a2dafbb327626
source_files: crates/led-console-bin/tests/ipc_contra_o_daemon.rs
required_test: o_daemon_recusa_a_linha_longa_por_si_proprio
data: 2026-09-28

# PORQUE EXISTE
#
# O debt gate do PR #7 (run 36475926021, job 109109510154) reprovou o TD-022 como «stale»
# contra `git-hash: 57cf21d` do artefacto de 2026-09-26. O ficheiro-fonte NÃO mudou:
#   git diff --quiet 57cf21d e46151b -- crates/led-console-bin/tests/ipc_contra_o_daemon.rs → 0
# Quem o `git log 57cf21d..HEAD -- <ficheiro>` da CI vê é o merge `b86464b` (PR #6): no merge
# ref do PR (`a16600c`, 1.º pai = `main`) a simplificação de história segue o lado da `main`,
# e `b86464b` difere do SEU 1.º pai. No head da branch (`e46151b`, 1.º pai = `b4e0777`) o mesmo
# comando sai vazio. É topologia, não conteúdo. Este artefacto re-mede no head actual; o de
# 2026-09-26 (`td-022-brokenpipe-linux-2026-09-26.md`) fica como está.

## 1. Linux — CI do PR #7 no head actual

Run `36475926021`, job `test (ubuntu-latest)` `109109510175`, checkout do merge ref
`a16600c` (= `e46151b` sobre `82e5ba1`), lido **no log**:

--- prova ---
```
test o_daemon_recusa_a_linha_longa_por_si_proprio ... ok
```
Soma das linhas `test result` do job: 105 binários, 1131 passed; 0 failed; 9 ignored.
--- fim da prova ---

## 2. macOS local — C0 e controlo negativo F1 sobre `e46151b`

macOS 14.8.9 x86_64, rustc 1.96.0. Exit lido do processo, sem pipe (KB-013). Comando:
`cargo test -p led-console-bin --test ipc_contra_o_daemon --locked -- --exact o_daemon_recusa_a_linha_longa_por_si_proprio`

| condição | mutação | exit | `error[E` | `panicked` |
|---|---|---|---|---|
| **C0** | nenhuma | `0` | 0 | 0 |
| **F1** ×3 | interrupção **forçada** (`MAX_BODY + 1` bytes sem `\n` + 500 ms) **+ `unwrap()` cru** no `writeln!` | `101 101 101` | 0 | 1 cada |
| **C0 após reverter** | nenhuma | `0` | 0 | 0 |

```
C0  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s

F1  panicked at crates/led-console-bin/tests/ipc_contra_o_daemon.rs:191:82:
    called `Result::unwrap()` on an `Err` value: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.53s

C0  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
```

Reversão: `git checkout -- <ficheiro>`; `shasum -a 256 -c` → OK (byte a byte igual ao de antes da
mutação); `git diff --quiet` → 0. A linha `:191` é do ficheiro **mutado**.

## 3. O que isto NÃO re-mede

- **A sonda Linux do errno** (run `36245228354`, `BrokenPipe` errno 32, 3/3) **não** foi
  re-executada: continua a ser a de `57cf21d`. Aplica-se a `e46151b` por o ficheiro-fonte ser
  byte a byte igual (`git diff --quiet` → 0) — inferência sobre conteúdo, não medição nova.
- F2 e F3 não foram re-executadas nesta passagem.
- Nada em hardware — o TD-022 é de teste, não de fio.
