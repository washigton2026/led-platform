# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do ramo fix/td-037-td-id-duplicado: N_GATE do gate suite 34 → 36
git-hash: 9f08b32
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:a27cc177535f524336612f6fc5cc7f19d39fd71ce983b086c4d50583d9849808
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-11-26

# PORQUE EXISTE
#
# Regenera a evidência porque o ci.yml mudou (do ramo fix/td-037-td-id-duplicado: N_GATE do gate suite 34 → 36). O job `miri (led-triple)` e o passo «Miri — audio-core» NÃO
# mudaram, e o ring_buffer.rs também não (mesmo sha256 das evidências anteriores). A anterior
# (docs/evidence/td-027-miri-ring-buffer-2026-11-26.md) fica como está.
#
# Como nas regenerações de 2026-11-24 e 2026-11-26, a execução é LOCAL (macOS x86_64, toolchain fixado
# nightly-2026-06-02), sobre a árvore com o ci.yml acima, e NÃO um run da CI. É o mesmo comando do passo da CI, com o
# mesmo toolchain, o mesmo filtro e MIRIFLAGS="" (RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS unset).

## 1. Execução — local, ci.yml com o sha256 acima
# rustc 1.98.0-nightly (6bdf43094 2026-06-01) · uname -m x86_64
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::' -- --list  → 5 testes, exit 0
# cargo +nightly-2026-06-02 miri test -p audio-core --locked --lib 'ring_buffer::tests::'            → exit 0
# Logs completos: ~/lumyx-evidence/2026-11-26-td027-regen-td037/ (contexto.txt, list.out.txt, run.out.txt, *.exit.txt)

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
