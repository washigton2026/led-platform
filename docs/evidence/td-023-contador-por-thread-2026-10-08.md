# TD-023 — os gates no_alloc dos 4 crates contam POR THREAD: fecho
git-hash: bc7991f
watched: crates/led-hal/tests/no_alloc.rs sha256:05d53cb8d471f479e9a3e1c4500179c4b178c29e8916b8ad7f3f019f1923eb28
watched: crates/led-sequencer/tests/no_alloc.rs sha256:1ea4e380023600a4f84ab9a5714e8091a9960662582679826e81bcf8878cb466
watched: crates/audio-core/tests/no_alloc.rs sha256:149aec5fa7320ab722ef757eabaea1e5b3e5428fba67238c8c5578dff6367c47
watched: crates/led-pixel-engine/tests/no_alloc.rs sha256:c28d8f4c3533fd1a39d0498ca0a91876e9ebfa19bdb768f42ce10b300b6ece3b
source_files: crates/led-hal/tests/no_alloc.rs, crates/led-sequencer/tests/no_alloc.rs, crates/audio-core/tests/no_alloc.rs, crates/led-pixel-engine/tests/no_alloc.rs
required_test: o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste
data: 2026-10-08

# PORQUE EXISTE
#
# O #23 (697048a, mergeado 2026-10-06) passou os gates no_alloc do led-hal, led-sequencer, audio-core e
# led-pixel-engine a atribuir por thread (o molde da F7.2 no led-protocols). Esta evidência fecha o TD-023
# no formato do audit_gate R4.1: `watched:` dos 4 ficheiros (sha256 = main bc7991f) + região de prova com
# a execução na CI da main, nos dois SO, lida no log via API (carimbo temporal e ANSI removidos).
# N por binário fixado aqui (listagem do R3.4, `--list`): audio-core 3 · led-hal 4 · led-pixel-engine 4 ·
# led-sequencer 3. Os nomes completos estão abaixo; um rename/remoção muda estas linhas.

## 1. CI da main bc7991f — run 37679587088

--- prova ---
```
# macOS — test (macos-latest), job 112991835975 (Image macos-26-arm64)
# audio-core
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-325285c482e7c973)
running 3 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
# led-hal
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-ba0a179affa5259d)
running 4 tests
test zero_allocations_on_hot_path ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test zero_allocations_on_hot_path_with_calibration ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s
# led-pixel-engine
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-ef7e5845d5b11e15)
running 4 tests
test every_library_effect_renders_without_allocating ... ok
test negative_control_an_allocating_effect_is_caught ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.65s
# led-sequencer
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-a652ee70576c8b94)
running 3 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test timeline_render_is_allocation_free ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
# Ubuntu — test (ubuntu-24.04), job 112991835803
# audio-core
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-65ebcb2310d3f63a)
running 3 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s
# led-hal
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-a9317cd8a8f2b2d5)
running 4 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test zero_allocations_on_hot_path_with_calibration ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.46s
# led-pixel-engine
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-3095380767eabb9e)
running 4 tests
test every_library_effect_renders_without_allocating ... ok
test negative_control_an_allocating_effect_is_caught ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.67s
# led-sequencer
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-fca08a6360ffb698)
running 3 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test timeline_render_is_allocation_free ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
```
--- fim da prova ---

## 2. Controlo negativo (R3.4, ~/lumyx-evidence/2026-10-05/r3.4/, mutações em cópia)
- `registar` a devolver sempre true (= o contador global antigo), led-hal → `ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path`
  FAILED, `panicked at crates/led-hal/tests/no_alloc.rs:138:5` (mut-global-hal.log).
- `registar` a devolver sempre false (contador cego), led-hal → `o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste`
  FAILED, `panicked at …/no_alloc.rs:155:5` (mut-zero-hal.log).
- led-sequencer: alloc_zeroed cego, realloc cego e corpo vazio → o teste dono FAILED em cada (mut2-*.log).
- Falsificador R3.4 ronda 2: NOT_FALSIFIED contra o claim corrigido (falsifier-ronda2.md).

## 3. Suporte: 16 runs depois do #23 sem nenhuma falha TD-023
~/lumyx-evidence/flakes.log (R4.5): as 10 falhas conhecidas (todas macOS, todas antes do #23) não se repetiram em
16 runs pós-#23 (24+ jobs test, os 3 testes que falhavam «... ok» em cada job macOS). Não prova ausência — é suporte.

## 4. O que isto NÃO prova (LIMITE declarado no ledger)
O gate prova «zero alocações NA THREAD QUE EXECUTA o caminho quente». Uma alocação por frame delegada num worker
persistente deixa de ser vista (falsificador R3.4, ataque b2). Hoje nenhum caminho medido delega noutra thread.
