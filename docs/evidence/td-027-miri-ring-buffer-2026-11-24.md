# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do TD-021 (--no-fail-fast no job windows)
git-hash: 563b883
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:63a35fed0863c0a91cf74607310ce1185de20619780bcd8e230c6212cb8c4509
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-11-24

# PORQUE EXISTE
#
# Regenera a evidência porque o ci.yml mudou: no job `windows (allow-failure)` o `cargo test` passa a levar
# `--no-fail-fast` (TD-021, 3.a falha). O job `miri (led-triple)` e o passo «Miri — audio-core» NÃO mudaram, e o
# ring_buffer.rs também não (o sha256 é o mesmo da evidência de 2026-10-08b). A de 2026-10-08b fica como está.
#
# DIFERENÇA EM RELAÇÃO À ANTERIOR, e não é arredondada: esta execução é LOCAL (macOS x86_64, toolchain fixado
# nightly-2026-06-02), sobre a árvore de trabalho com o ci.yml acima, NÃO um run da CI em ubuntu-latest. Sem
# push não há run com este ci.yml. É o mesmo comando do passo da CI, com o mesmo toolchain e o mesmo filtro.

## 1. Execução — local, árvore 563b883 + diff do ci.yml (sha256 acima)
# rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::' -- --list  → 5 testes, exit 0
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::'            → exit 0
# Log completo: ~/lumyx-evidence/2026-11-22-td021/ciclo5/miri-audio-core.txt

--- prova ---
```
test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
test ring_buffer::tests::push_slice_truncates_when_full ... ok
test ring_buffer::tests::push_then_pop_round_trip ... ok
test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.14s
miri audio-core: 5/5 testes (piso 5), exit 0
```
--- fim da prova ---

## 2. Controlo negativo
Igual ao da evidência de 2026-10-05, §2: PR #22, run 37236379756. A mutação `write.load(Acquire)` →
`Relaxed` dá `Undefined Behavior: Data race detected` no `spsc_stress_no_loss_or_reorder_under_threads`. Esta
mudança não toca no passo Miri nem no ring_buffer.rs, só no job windows. Não foi repetido.

## 3. O que isto NÃO mede
- Não é o runner ubuntu da CI: o primeiro run com este ci.yml confirma ou desmente.
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
- Nada em hardware.
