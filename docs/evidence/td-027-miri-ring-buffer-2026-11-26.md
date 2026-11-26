# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do merge da main em fix/td-030-seqlock
git-hash: 7aeec49
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:b0eda2c81abd22606d57a4b21a11bbde6510fb201235282763bb300e6140efb3
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-11-26

# PORQUE EXISTE
#
# Regenera a evidência porque o ci.yml mudou: o merge de origin/main (6ce4ade) em fix/td-030-seqlock (7aeec49)
# junta o ci.yml da main (o da evidência de 2026-11-24, sha256 63a35fed…4509) com o job `loom (TD-030 seqlock)`
# do ramo. Em relação ao ci.yml da main, o diff é só o bloco desse job novo (31 linhas acrescentadas). O job
# `miri (led-triple)` e o passo «Miri — audio-core» NÃO mudaram, e o ring_buffer.rs também não (o sha256 é o
# mesmo das evidências de 2026-10-08b e 2026-11-24). As anteriores ficam como estão.
#
# Como em 2026-11-24, a execução é LOCAL (macOS x86_64, toolchain fixado nightly-2026-06-02), sobre a árvore do
# merge com o ci.yml acima, e NÃO um run da CI em ubuntu-latest. É o mesmo comando do passo da CI, com o mesmo
# toolchain e o mesmo filtro.

## 1. Execução — local, árvore do merge (7aeec49 + origin/main 6ce4ade), ci.yml com o sha256 acima
# rustc 1.98.0-nightly (6bdf43094 2026-06-01) · uname -m x86_64
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::' -- --list  → 5 testes, exit 0
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::'            → exit 0
# Logs completos: ~/lumyx-evidence/2026-11-26-td030-fecho/miri-td027/ (list2.out.txt, run.out.txt, *.exit.txt)

--- prova ---
```
test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
test ring_buffer::tests::push_slice_truncates_when_full ... ok
test ring_buffer::tests::push_then_pop_round_trip ... ok
test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.14s
```
--- fim da prova ---

## 2. Controlo negativo
Igual ao da evidência de 2026-10-05, §2: PR #22, run 37236379756. A mutação `write.load(Acquire)` →
`Relaxed` dá `Undefined Behavior: Data race detected` no `spsc_stress_no_loss_or_reorder_under_threads`. Esta
mudança não toca no passo Miri nem no ring_buffer.rs, só acrescenta o job loom. Não foi repetido.

## 3. O que isto NÃO mede
- Não é o runner ubuntu da CI: o primeiro run com este ci.yml confirma ou desmente.
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
- Nada em hardware.
