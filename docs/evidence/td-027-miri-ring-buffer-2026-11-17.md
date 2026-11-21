# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do #56 (job loom-td030 acrescentado)
git-hash: 9947477
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:101d7e6f052e35d025126cd60070fb22146b59c9b86cb5fcc8c2a058880984a1
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-11-17

# PORQUE EXISTE
#
# Regenera a evidência no próprio #56. A de 2026-10-08b fica como está.
# O #56 muda o ci.yml (9947477 acrescenta o job `loom (TD-030 seqlock)` a partir da linha 389), e o ci.yml é
# vigiado porque é ele que põe o passo do audio-core e o piso N sob Miri. O diff 807949c..9947477 do ci.yml é só
# esse acréscimo: o job `miri (led-triple)` e o passo «Miri — audio-core» não mudam.
# Fonte: a CI do PR #56 sobre 9947477, run 44154600623 (tentativa 1). O checkout desse run foi
# `refs/pull/56/merge` = 5f8c66a («Merge 9947477 into 0078e73»). Como 0078e73 já é ancestral de 9947477, o ci.yml
# e o ring_buffer.rs desse checkout são os do 9947477, com os sha256 acima.

## 1. Execução — run 44154600623, job 134291822887 (miri (led-triple)), checkout 5f8c66a
# Do cabeçalho do log: Image: ubuntu-24.04 · Version: 20261111.370.1
# Toolchain: nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Linhas do passo «Miri — audio-core (ring buffer SPSC, TD-027)», lidas com `gh run view --job 134291822887 --log`,
# ANSI e carimbo temporal removidos (log completo: ~/lumyx-evidence/2026-11-17-pr56-miri-job134291822887.log):

--- prova ---
```
test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
test ring_buffer::tests::push_slice_truncates_when_full ... ok
test ring_buffer::tests::push_then_pop_round_trip ... ok
test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.05s
miri audio-core: 5/5 testes (piso 5), exit 0
```
--- fim da prova ---

## 2. Controlo negativo
Igual ao da evidência de 2026-10-05, §2: PR #22, run 37236379756. A mutação `write.load(Acquire)` →
`Relaxed` dá `Undefined Behavior: Data race detected` no `spsc_stress_no_loss_or_reorder_under_threads`. O
#56 não toca no passo Miri nem no ring_buffer.rs, só acrescenta um job ao ci.yml. Não foi repetido.

## 3. O que isto NÃO mede
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
- Nada em hardware.
