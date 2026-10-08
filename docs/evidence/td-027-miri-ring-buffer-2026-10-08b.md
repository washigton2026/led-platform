# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do #30 (macOS fixado em macos-26)
git-hash: bca78c4
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:c51a7335212238d01993559216811398a647c7049d8d3d16c45b79540721e729
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-10-08

# PORQUE EXISTE
#
# Regenera a evidência no próprio #30 (R8.1). As de 2026-10-05, 2026-10-07 e 2026-10-08 ficam como estão.
# O #30 muda o ci.yml (a matriz do job test passa de macos-latest a macos-26), e o ci.yml é vigiado porque é ele
# que põe o passo do audio-core e o piso N sob Miri. Fonte: a CI do PR #30 sobre bca78c4 (merge da main edb4777
# no ramo), run 37813624105. O checkout desse run foi `refs/pull/30/merge` = 6a8c1af («Merge bca78c4 into
# c5b044c»). O ci.yml e o ring_buffer.rs desse checkout são os do bca78c4: a main c5b044c só mudou docs e testes
# fora destes dois ficheiros. Têm exatamente os sha256 acima.

## 1. Execução — run 37813624105, job 113436509744 (miri (led-triple)), checkout 6a8c1af
# Do cabeçalho do log: Image: ubuntu-24.04 · Version: 20261004.327.1
# Toolchain: nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Linhas do passo «Miri — audio-core», lidas via API (`/actions/jobs/113436509744/logs`), ANSI e carimbo
# temporal removidos:

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
#30 não toca no passo Miri nem no ring_buffer.rs, só na etiqueta da imagem macOS. Não foi repetido.

## 3. O que isto NÃO mede
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
- Nada em hardware.
