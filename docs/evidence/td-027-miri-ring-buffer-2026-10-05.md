# TD-027 — o ring buffer SPSC do audio-core sob Miri na CI da main (fecho)
git-hash: 7f1ec01
source_files: crates/audio-core/src/ring_buffer.rs
required_test: spsc_stress_no_loss_or_reorder_under_threads
data: 2026-10-05

# PORQUE EXISTE
#
# O PR #17 (4a43167 + 035159f) acrescentou ao job `miri (led-triple)` um passo que corre
# `ring_buffer::tests::` do audio-core sob Miri, com o nightly pinado e um piso N == nº de
# #[test] do módulo. Este artefacto extrai, dos logs da CI, a execução na main logo após o
# merge (7f1ec01) e o controlo negativo do PR #22, e é o evidence_ref do fecho do TD-027.

## 1. Execução na main — run 37268736600, job 111631004086 (miri (led-triple)), headSha 7f1ec01
# Corroborada pela run seguinte da main (37268800503, 79e52e2): mesmo resultado, 5/5.
# Toolchain (linha do log):
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:39:28.5850186Z   nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Saída do passo audio-core — linhas tal como `gh run view 37268736600 --log --job 111631004086` as devolve:
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5654501Z      Running unittests src/lib.rs (target/miri/x86_64-unknown-linux-gnu/debug/deps/audio_core-8c1b6e07d1ec0bcc)
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5655090Z running 5 tests
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5655576Z test ring_buffer::tests::pop_exact_returns_false_without_consuming_when_short ... ok
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5655850Z test ring_buffer::tests::push_slice_truncates_when_full ... ok
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5656087Z test ring_buffer::tests::push_then_pop_round_trip ... ok
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5656310Z test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... ok
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5656559Z test ring_buffer::tests::wraps_around_correctly_over_many_cycles ... ok
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5656846Z test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 6.05s
miri (led-triple)	UNKNOWN STEP	2026-10-05T05:41:30.5687829Z miri audio-core: 5/5 testes (piso 5), exit 0

## 2. Controlo negativo — PR #22 (fechado sem merge), run 37236379756, job 111536190663, headSha a29451a
# Mutação (gh pr diff 22), em crates/audio-core/src/ring_buffer.rs:
#   -        let w = self.write.load(Ordering::Acquire);
#   +        let w = self.write.load(Ordering::Relaxed);
# Mesmo toolchain pinado; o passo do audio-core reprova com UB do Miri (não com erro de compilação):
miri (led-triple)	Rust nightly PINADO + miri	2026-10-04T21:30:49.2775228Z   nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
NEG: miri (led-triple)	Miri — audio-core (ring buffer SPSC, TD-027)	2026-10-04T21:34:12.4475786Z test ring_buffer::tests::spsc_stress_no_loss_or_reorder_under_threads ... error: Undefined Behavior: Data race detected between (1) non-atomic write on thread `unnamed-5` and (2) non-atomic read on thread `ring_buffer::te` at alloc112227
NEG: miri (led-triple)	Miri — audio-core (ring buffer SPSC, TD-027)	2026-10-04T21:34:12.4494820Z   process didn't exit successfully: `/home/runner/.rustup/toolchains/nightly-2026-06-02-x86_64-unknown-linux-gnu/bin/cargo-miri runner /home/runner/work/led-platform/led-platform/target/miri/x86_64-unknown-linux-gnu/debug/deps/audio_core-8c1b6e07d1ec0bcc 'ring_buffer::tests::'` (exit status: 1)
NEG: miri (led-triple)	Miri — audio-core (ring buffer SPSC, TD-027)	2026-10-04T21:34:12.4514742Z ##[error]Undefined Behavior: Data race detected betwee
NEG: miri (led-triple)	Miri — audio-core (ring buffer SPSC, TD-027)	2026-10-04T21:34:12.4523779Z ##[error]Process completed with exit code 1.
# error[E no log do NEG-CTL: 0 ocorrências.
