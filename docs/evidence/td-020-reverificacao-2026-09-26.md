# TD-020 — Re-verificação contra `320ff94` (a evidência de `13d2f41` ficou stale)
git-hash: 320ff94
source_files: crates/led-hal/src/shared_clock.rs
required_test: concurrent_readers_never_see_rewind_during_correction
data: 2026-09-26
evidencia_original: docs/evidence/td-020-relogio-monotonico-2026-09-17.md

# PORQUE ESTE ARTEFACTO EXISTE
#
# O `audit_gate.py` passou a reprovar o TD-020 com
#   «evidence is stale — source files changed after evidence was generated (hash 13d2f41)»
# porque o commit `320ff94` (C4 — remoção dos `unsafe impl Send/Sync` redundantes do
# `led-hal`) tocou no `shared_clock.rs`. O hook de pre-commit NÃO o apanhou quando o C4 foi
# commitado: a verificação é `git log <hash>..HEAD -- <ficheiro>`, e nesse momento o HEAD
# ainda não continha a alteração (é a classe do candidato a TD 0.4 do ROADMAP). A CI também
# não o apanhou, porque o `ci.yml` não corre o `audit_gate`.
#
# Tudo abaixo foi RE-EXECUTADO contra o código actual. Nada foi transcrito do artefacto
# original, que fica intacto como registo da correcção.

## O que mudou no ficheiro desde `13d2f41`

```
git log --oneline 13d2f41..320ff94 -- crates/led-hal/src/shared_clock.rs
  320ff94 refactor(led-hal): remove unsafe impl Send/Sync redundantes (...)

linhas de CÓDIGO alteradas (descontados comentários):
  -unsafe impl Send for SharedClock {}
  -unsafe impl Sync for SharedClock {}
```

`now_ms()` (o `fetch_max(AcqRel)`, `shared_clock.rs:90-91`) e o detector estão **intocados**.
A pergunta desta re-verificação não é «a correcção ainda está lá» — o diff já responde — mas
«o detector continua a distinguir o defeito da correcção sobre o código actual».

## Resultados — três condições, 10 execuções cada

Comando por execução (exit lido do processo, sem pipe — KB-013):
`cargo test -p led-hal --lib -- --exact shared_clock::tests::concurrent_readers_never_see_rewind_during_correction`

| condição | código | vermelho / 10 | verde / 10 | exit do processo (10 execuções) | `negative_control` do ledger |
|---|---|---|---|---|---|
| **A** — actual | `fetch_max` | **0** | **10** | `0` ×10 | — (é o `required_test` a passar) |
| **B** — defeito reposto | `load` / `max` / `store` (3 operações) | **10** | **0** | `101` ×10 | controlo **A)** |
| **C** — controlo negativo | 3 operações **e** `AMPLITUDE_MS = 0` | **0** | **10** | `0` ×10 | controlo **B)** |

`N` por execução: **1** teste corrido em todas as 30 (`1 passed` em A e C, `1 failed` em B;
`101 filtered out` — o `--exact` seleccionou o `required_test` e só ele).

As mutações B e C foram aplicadas a uma cópia de trabalho e **restauradas** no fim
(`git diff --quiet -- crates/led-hal/src/shared_clock.rs` → 0).

Condição A, execução 1:

```
running 1 test
test shared_clock::tests::concurrent_readers_never_see_rewind_during_correction ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 0.36s
```

Condição B, execução 1 (as 10 reprovam com a mesma mensagem; recuo **1 ms** nas 10, na
ronda 0 em 7 e na ronda 1 em 3):

```
assertion `left == right` failed: ronda 0: um leitor viu o relógio RECUAR 1 ms sob correcção
concorrente (8 leitoras × 20000 leituras). A guarda de monotonia não é atómica — ...
  left: 1
 right: 0
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 101 filtered out; finished in 0.02s
```

Condição C, execução 1:

```
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 0.30s
```

**Leitura:** o mesmo padrão de `13d2f41` (10/0, 0/10, controlo 0/10). O detector continua a
reprovar o defeito sempre e a passar a correcção sempre, e o controlo prova que o vermelho vem
da alternância do offset, não do volume de iterações.

## Gates (2026-09-26, HEAD `320ff94`)

```
cargo test -p led-hal --lib
  running 102 tests
  test shared_clock::tests::concurrent_readers_never_see_rewind_during_correction ... ok
  test result: ok. 102 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
  EXIT_HAL_LIB=0

cargo clippy -p led-hal --all-targets --locked -- -D warnings
  EXIT_CLIPPY=0 · 0 warnings

cargo tree -p led-hal --depth 1
  led-hal v0.1.0
  └── led-core v0.1.0
```

Workspace no mesmo código: CI run `36226221020` (PR #6, head `320ff94`), **lido no log**:
ubuntu 105 suítes · 1131 passed · 0 failed · 9 ignored; macOS 105 · 1135/0/9. Não
re-executado localmente neste artefacto.

## NÃO MEDIDO — declarado em vez de arredondado

- **Alocação em `now_ms`**: continua sem gate (`tests/no_alloc.rs` não exercita o
  `SharedClock`). Argumento estrutural, como no artefacto original.
- **Miri sobre o `led-hal`**: não corrido. Depois do `320ff94` o crate tem **0** construções
  `unsafe`, logo o Miri já não é exigido por essa razão.
- **Hardware**: nada validado; o rig continua offline.
- **Tempos**: os `finished in` acima são informativos; load 1-min ~5,7–6,6 durante a corrida.
