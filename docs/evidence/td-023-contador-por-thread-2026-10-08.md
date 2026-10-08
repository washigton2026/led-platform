# TD-023 — os 5 gates no_alloc contam SÓ a thread do teste (thread_local): fecho
git-hash: edb4777
watched: crates/audio-core/tests/no_alloc.rs sha256:b3250c4756257cd0752f20fc70cacf429de6564df584ff6555972ae6a5d5b890
watched: crates/led-hal/tests/no_alloc.rs sha256:b715113386a1c57f33cc5db2d2c0016e064fe3691471fc7f369f8375ac09e742
watched: crates/led-pixel-engine/tests/no_alloc.rs sha256:b6430530b4d270af5954cec3666d8643b0f8097b22454152781ac871de1a2b5a
watched: crates/led-sequencer/tests/no_alloc.rs sha256:61de8b8fe0c392bba7cdc7764471a39190b16743a5e69ddc5f3226a1b2039e29
watched: crates/led-protocols/tests/no_alloc.rs sha256:3634a481b9fd42f5e2768423ecba9082472e044a5f9efe7033ee6a88fe47cc19
source_files: crates/audio-core/tests/no_alloc.rs, crates/led-hal/tests/no_alloc.rs, crates/led-pixel-engine/tests/no_alloc.rs, crates/led-sequencer/tests/no_alloc.rs, crates/led-protocols/tests/no_alloc.rs
required_test: outra_thread_marcada_nao_entra_na_contagem_desta
data: 2026-10-08

# PORQUE EXISTE
#
# O #23 (contador «por thread» via marca + AtomicUsize global) NÃO fechou o TD-023: o R6.1 (run 37735453730)
# mediu 10/500 falhas em macos-26 e o diagnóstico mostrou que em 10/10 a alocação contada vinha de OUTRA thread
# marcada (a marca nunca era desligada e o contador era global) — hipótese H3, defeito do arnês. O #42
# (a98ff98, mergeado na main em edb4777, 2026-10-08) troca o contador por um `thread_local` por thread
# (ALLOCS_DESTA_THREAD), desliga a marca ao fechar a janela, e acrescenta a cada um dos 5 gates o teste
# determinístico `outra_thread_marcada_nao_entra_na_contagem_desta` (uma thread marcada aloca 1000× dentro da
# janela; a contagem do teste tem de ficar a 0 e a da outra ≥ 1000). Os sha256 acima são os da main edb4777 e
# são iguais aos de a98ff98, o commit medido em §2.
# N por binário fixado aqui: audio-core 4 · led-hal 5 · led-pixel-engine 5 · led-sequencer 4 · led-protocols 3.
# Um rename/remoção muda estas linhas.

## 1. CI da main edb4777 — run 37813416605, lida no log via API (carimbo temporal e ANSI removidos)

--- prova ---
```
# macOS — test (macos-latest), job 113435792225
# audio-core
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-325285c482e7c973)
running 4 tests
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s
# led-hal
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-ba0a179affa5259d)
running 5 tests
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test zero_allocations_on_hot_path ... ok
test zero_allocations_on_hot_path_with_calibration ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.41s
# led-pixel-engine
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-ef7e5845d5b11e15)
running 5 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test every_library_effect_renders_without_allocating ... ok
test negative_control_an_allocating_effect_is_caught ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.80s
# led-protocols
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-d9d9351b89427be6)
running 3 tests
test ddp_backend_send_path_is_alloc_free ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
# led-sequencer
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-a652ee70576c8b94)
running 4 tests
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test timeline_render_is_allocation_free ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
# Ubuntu — test (ubuntu-24.04), job 113435792332
# audio-core
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-65ebcb2310d3f63a)
running 4 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
# led-hal
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-a9317cd8a8f2b2d5)
running 5 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_hot_path ... ok
test zero_allocations_on_hot_path ... ok
test zero_allocations_on_hot_path_with_calibration ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
# led-pixel-engine
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-3095380767eabb9e)
running 5 tests
test negative_control_an_allocating_effect_is_caught ... ok
test every_library_effect_renders_without_allocating ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.70s
# led-protocols
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-9b0aab5d540a5f2e)
running 3 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test ddp_backend_send_path_is_alloc_free ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
# led-sequencer
     Running tests/no_alloc.rs (target/debug/deps/no_alloc-fca08a6360ffb698)
running 4 tests
test o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste ... ok
test outra_thread_marcada_nao_entra_na_contagem_desta ... ok
test ruido_de_fundo_noutra_thread_nao_reprova_o_render ... ok
test timeline_render_is_allocation_free ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
```
--- fim da prova ---

## 2. Recorrência sob carga — 5000 execuções por SO (PR #43 [MEDIÇÃO], fechado sem merge)
# Run 37757231893, ramo measure/td-023-r7 sobre a98ff98 (os 5 ficheiros = os sha256 acima). N=500 iterações ×
# 5 binários no_alloc por job, com `cargo test --workspace` em ciclo noutro processo; carga amostrada por
# iteração (binários de teste da carga vivos), voltas completas contadas, carga viva no fim. 0 vácuas.
#
#   runner                 variante   falhas     sob carga  voltas  viva
#   macos-26 arm64         corrigido  0/2500     485/500    14      sim
#   ubuntu-24.04 x86_64    corrigido  0/2500     496/500    18      sim
#   macos-26 arm64         regressão  25/2500    487/500    155     sim   (led-hal 18 · pixel-engine 5 · audio-core 1 · sequencer 1)
#   ubuntu-24.04 x86_64    regressão  1/2500     464/500    168     sim   (led-hal 1)
#
# A linha RESULTADO do macOS imprimiu `=25` em cada binário (bash 3.2 sem `declare -A`); o total está certo e a
# repartição acima foi recontada a partir das linhas `##[warning]` do log.

## 3. Controlo negativo — o contador antigo reposto
# (a) Local (macOS x86_64), cópia de `git archive a98ff98`, CARGO_TARGET_DIR próprio: `contar_nesta_thread`
#     volta a um AtomicUsize global → `outra_thread_marcada_nao_entra_na_contagem_desta` FAILED nos 5 binários,
#     «1000 alocação(ões) de OUTRA thread marcada entraram na contagem desta thread (TD-023)».
# (b) Sob carga (§2, variante `regressao`: contagem global + marca nunca desligada, o teste novo saltado): as
#     falhas do R6.1 voltam — 25/2500 em macOS e 1/2500 em Ubuntu, «calibrated hot path allocated 1 time(s) over
#     10000 frames (other threads, ignored: 4)».

## 4. O que isto NÃO mede
# - O LIMITE aceite do TD-023 continua: o gate prova «zero alocações NA THREAD QUE EXECUTA o caminho quente»;
#   uma alocação delegada num worker persistente não é vista.
# - led-daemon-bin/tests/custo_do_fanout.rs e no_alloc_canal.rs (contador global, fora deste TD por ordem do operador).
