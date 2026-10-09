# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do #45 (gate suite + test_pg_check)
git-hash: (commit do F1b, ramo tools/parallel-gauntlet-v1)
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:7f6a650b0070ce989e713cc033c78ec2a1e95ced9582fc0af3ee347a2b4f14af
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-10-09

# PORQUE EXISTE
#
# O #45 (F1b, ordem do operador 2026-10-09) acrescenta ao job `gate suite` o passo tests/test_pg_check.py
# (N_PG=31). O ci.yml é vigiado pelo TD-027, logo a evidência tem de ser regenerada no mesmo PR.
#
# ESTADO DESTA VERSÃO: INTERINA — execução LOCAL, macOS x86_64. O pre-commit hook recusa commitar o
# ci.yml novo sem evidência que o fixe, e a CI só corre depois do commit. O toolchain pinado da CI
# (nightly-2026-06-02, rustc 6bdf43094) não tem o componente miri instalado nesta máquina. Usou-se o
# `nightly` local, rustc 1.98.0-nightly (d595fce01 2026-06-02), que tem um dia de diferença. Num commit
# seguinte, que só muda este texto (o ci.yml fica igual, o sha256 também), esta região passa a ter as
# linhas do job `miri (led-triple)` da CI do #45, na imagem ubuntu-24.04. As evidências anteriores ficam.

## 1. Execução local — `cargo +nightly miri test -p audio-core --locked --lib 'ring_buffer::tests::'`
# Exit lido do processo, sem pipe (KB-013): 0.

--- prova ---
```
     Running unittests src/lib.rs (target/miri/x86_64-apple-darwin/debug/deps/audio_core-153012d591ca0a69)
test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
test ring_buffer::tests::push_slice_truncates_when_full ... ok
test ring_buffer::tests::push_then_pop_round_trip ... ok
test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.15s
```
--- fim da prova ---

## 2. Controlo negativo
Igual ao da evidência de 2026-10-05, §2: PR #22, run 37236379756. A mutação `write.load(Acquire)` →
`Relaxed` dá `Undefined Behavior: Data race detected`. O #45 não toca no passo Miri nem no
ring_buffer.rs: só acrescenta um passo ao job `gate suite`.

## 3. O que isto NÃO mede
- A imagem ubuntu-24.04 da CI: é o que o commit seguinte vem trazer.
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
