# TD-027 — o ring buffer SPSC do audio-core sob Miri, com o ci.yml do #45 (gate suite + test_pg_check)
git-hash: dd0d1c2
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
# ESTADO DESTA VERSÃO: CI. A 1.ª versão (commit dd0d1c2) foi INTERINA, uma execução local em macOS com o nightly
# d595fce01, porque o pre-commit hook recusa o ci.yml novo sem evidência e a CI só corre depois do commit.
# Esta versão troca a região de prova pelas linhas do job `miri (led-triple)` da CI do próprio #45. O ci.yml
# e o ring_buffer.rs são os mesmos (sha256 acima inalterado). As evidências anteriores ficam.

## 1. Execução na CI do #45 — run 37904955664, job 113736077812 (miri (led-triple)), checkout b23782a
# («Merge dd0d1c2 into 29029b2»; a main 29029b2 não muda o ci.yml nem o ring_buffer.rs — `git diff` vazio)
# Do cabeçalho do log: Image: ubuntu-24.04 · Version: 20261004.327.1
# Toolchain: nightly-2026-06-02-x86_64-unknown-linux-gnu installed - rustc 1.98.0-nightly (6bdf43094 2026-06-01)
# Linhas do passo «Miri — audio-core», lidas com `gh api --allow-escape-sequences …/jobs/113736077812/logs`,
# carimbo temporal removido:

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

A execução local da 1.ª versão (nightly d595fce01, macOS x86_64) também deu 5/5, exit 0. Fica em
~/lumyx-evidence/2026-10-09/f1b/td027/.

## 2. Controlo negativo
Igual ao da evidência de 2026-10-05, §2: PR #22, run 37236379756. A mutação `write.load(Acquire)` →
`Relaxed` dá `Undefined Behavior: Data race detected`. O #45 não toca no passo Miri nem no
ring_buffer.rs.

## 3. O que isto NÃO mede
- O `read.load(Acquire)` (ring_buffer.rs:48) não tem controlo negativo próprio.
