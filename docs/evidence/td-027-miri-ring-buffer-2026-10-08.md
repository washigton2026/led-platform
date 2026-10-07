# TD-027 — o ring buffer SPSC do audio-core sob Miri na CI da main, na imagem FIXADA ubuntu-24.04 (depois de #27/#29/#31)
git-hash: bc7991f
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:8a97918f710c73398baa5df25bd88d8d6ea99bbb28875fb2ac72b32d7866580f
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-10-08

# PORQUE EXISTE
#
# Regenera a evidência (as de 2026-10-05 e 2026-10-07 ficam como estão): o #27, o #29 e o #31
# mudaram o ci.yml na main, e o ci.yml é vigiado. Fonte: a CI da main DEPOIS desses merges —
# run 37679587088, headSha bc7991f. O ring_buffer.rs e o ci.yml têm,
# nesse commit, exatamente os sha256 acima (o #24 só toca docs). O ci.yml é vigiado porque é
# ele que põe o passo do audio-core e o piso N sob Miri: mudá-lo muda o que esta prova mede.

## 1. Execução na main — run 37679587088, job 112991836048 (miri (led-triple)), headSha bc7991f
# Do cabeçalho do log: Image: ubuntu-24.04 · Version: 20261004.327.1
# Toolchain: nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Linhas do passo «Miri — audio-core», lidas via API (`/actions/jobs/112991836048/logs`), ANSI e
# carimbo temporal removidos (o carimbo de cada linha está no log, 2026-10-07T20:10:25Z):

--- prova ---
```
     Running unittests src/lib.rs (target/miri/x86_64-unknown-linux-gnu/debug/deps/audio_core-8c1b6e07d1ec0bcc)
test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
test ring_buffer::tests::push_slice_truncates_when_full ... ok
test ring_buffer::tests::push_then_pop_round_trip ... ok
test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.05s
miri audio-core: 5/5 testes (piso 5), exit 0
```
--- fim da prova ---

## 2. Controlo negativo — PR #22 (fechado sem merge), run 37236379756, job 111536190663, headSha a29451a
Mutação em `ring_buffer.rs`: `self.write.load(Ordering::Acquire)` → `Ordering::Relaxed`. Mesmo
toolchain pinado. O passo do audio-core reprova com `Undefined Behavior: Data race detected` no
`spsc_stress_no_loss_or_reorder_under_threads` (exit 1, 0 ocorrências de `error[E`). Linhas citadas
na evidência de 2026-10-05, §2. Esse run foi em `ubuntu-latest` (antes do #25), o que na data era
24.04: não foi repetido na imagem fixada.

## 3. O que isto NÃO mede
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio (só o do `write`).
- Nada em hardware: o TD-027 é sobre memória, não sobre o fio.
