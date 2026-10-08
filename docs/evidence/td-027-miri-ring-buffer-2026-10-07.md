# TD-027 — o ring buffer SPSC do audio-core sob Miri na CI da main, na imagem FIXADA ubuntu-24.04
git-hash: 24d5b91
watched: crates/audio-core/src/ring_buffer.rs sha256:66ff27a115ad43f57a8d5a856395a25e8ad45fd251e40d6a22c1ddb62abe682d
watched: .github/workflows/ci.yml sha256:c44c2a16420c44da541732bd7845c37f86b30c1bd731f42eccfc6e3030e49c40
source_files: crates/audio-core/src/ring_buffer.rs, .github/workflows/ci.yml
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-10-07

# PORQUE EXISTE
#
# Regenera a evidência de 2026-10-05 (que fica como está) no formato do audit_gate R4.1:
# `watched:` por conteúdo + região de prova. Fonte: a CI da main DEPOIS do #25 (imagem fixada
# ubuntu-24.04) e do #26 — run 37576929449, headSha 24d5b91. O ring_buffer.rs e o ci.yml têm,
# nesse commit, exatamente os sha256 acima (o #24 só toca docs). O ci.yml é vigiado porque é
# ele que põe o passo do audio-core e o piso N sob Miri: mudá-lo muda o que esta prova mede.

## 1. Execução na main — run 37576929449, job 112647806152 (miri (led-triple)), headSha 24d5b91
# Do cabeçalho do log: Image: ubuntu-24.04 · Version: 20261004.327.1
# Toolchain: nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Linhas do passo «Miri — audio-core», lidas via API (`/actions/jobs/112647806152/logs`), ANSI e
# carimbo temporal removidos (o carimbo de cada linha está no log, 2026-10-07T05:36:54Z):

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
