# TD-036 — o veredito de tempo do custo_do_fanout sai da CI; a CI corre só os asserts de alocação
git-hash: b4c3b7a
watched: crates/led-daemon-bin/tests/custo_do_fanout.rs sha256:56767d08dc88326c1791b2748aae71c1af390cf8cf76295436e32fef5de6063e
data: 2026-11-26

# PORQUE EXISTE
#
# O o_custo_do_fanout_e_medido_e_nao_cresce_superlinearmente assertava razao_sem <= 8.0 sobre tempos
# de parede no binário de testes da CI; deu 9.20× no test (macos-26) do run 45810176438/1 sem mudança
# de código, e passou na repetição. O PR #66 (671daa8, mergeado em b4c3b7a) separa-o em dois:
# o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas (CI, asserts exatos de alocação) e
# o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente (#[ignore], gate manual, molde do TD-006).
# O pending_gate pedia: nos jobs test (macos-26) e test (ubuntu-24.04), «... ok» no primeiro e
# «... ignored» no segundo. Abaixo, os 4 jobs que o provam: os dois do PR (run 45874287754, 671daa8)
# e os dois do push na main (run 45874977001 tentativa 1, b4c3b7a). O conteúdo de
# custo_do_fanout.rs é o mesmo nos três commits (sha acima, calculado em 671daa8, b4c3b7a e HEAD).
#
# Linhas copiadas de `gh run view <run> -R washigton2026/led-platform --job <id> --log`, sem os
# códigos ANSI. No run da main o gh escreve o passo como «UNKNOWN STEP» (há uma tentativa 2 desse
# run, só do job windows). Os números Lnnnn são as linhas desse output.
# Jobs: 140097324488 (ubuntu, PR) L1374–L1377 · 140097324368 (macos, PR) L1264–L1267 ·
#       140099654455 (ubuntu, main) L1360–L1363 · 140099654632 (macos, main) L1269–L1272.
# Logs completos e SHA256SUMS: ~/lumyx-evidence/2026-11-26-td037/td036-ci/ e
# ~/lumyx-evidence/2026-11-26-td-fanout/ci-671daa8/ , .../ci-main-b4c3b7a/.

## 1. CI — os 4 jobs, o teste da CI passa e o gate de tempo está ignored
--- prova ---
test (ubuntu-24.04)	Test	2026-11-26T18:15:44.3957121Z test o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente ... ignored, gate de performance manual (TD-036): razão de tempo de parede, fora da CI
test (ubuntu-24.04)	Test	2026-11-26T18:15:45.1341688Z test o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas ... ok
test (ubuntu-24.04)	Test	2026-11-26T18:15:45.1342267Z test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.74s
test (macos-26)	Test	2026-11-26T18:15:51.5133100Z test o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente ... ignored, gate de performance manual (TD-036): razão de tempo de parede, fora da CI
test (macos-26)	Test	2026-11-26T18:15:52.8584030Z test o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas ... ok
test (macos-26)	Test	2026-11-26T18:15:52.8585800Z test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.35s
test (ubuntu-24.04)	UNKNOWN STEP	2026-11-26T18:20:41.0078772Z test o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente ... ignored, gate de performance manual (TD-036): razão de tempo de parede, fora da CI
test (ubuntu-24.04)	UNKNOWN STEP	2026-11-26T18:20:42.5647692Z test o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas ... ok
test (ubuntu-24.04)	UNKNOWN STEP	2026-11-26T18:20:42.5649053Z test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.56s
test (macos-26)	UNKNOWN STEP	2026-11-26T18:20:42.7703170Z test o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente ... ignored, gate de performance manual (TD-036): razão de tempo de parede, fora da CI
test (macos-26)	UNKNOWN STEP	2026-11-26T18:20:44.1790040Z test o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas ... ok
test (macos-26)	UNKNOWN STEP	2026-11-26T18:20:44.1791140Z test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.41s
--- fim da prova ---

## 2. Controlos negativos (locais, macOS arm64, sobre cópia; ~/lumyx-evidence/2026-11-26-td-fanout/)
# A. gate de tempo com LIMITE plantado a 1.0 (04-neg-a-tempo, exit 101): ainda sabe reprovar.
NEG: test o_custo_do_fanout_em_tempo_nao_cresce_superlinearmente ... FAILED
NEG: 5 alvos custaram 4.04× um alvo (sem calibração), acima do limite de 1×.
# B. assert de alocação com 5.0 → 4.0 (05-neg-b-aloc, exit 101): o teste da CI continua exato.
NEG: test o_custo_do_fanout_e_medido_e_as_alocacoes_sao_exactas ... FAILED
NEG: assertion `left == right` failed: 5 alvos têm de custar exactamente 5 alocações por frame — left: 5.0, right: 4.0

## 3. Fora do âmbito, registado
# O push na main (run 45874977001) teve o job windows (allow-failure) vermelho na tentativa 1 por
# outro teste (led-pixel-engine::reactive::tests::fallbacks_medidos_com_escritor_continuo,
# «leituras=0») — TD-038; verde na tentativa 2 (job 140106749132). Os dois jobs que este TD exige
# (ubuntu-24.04 e macos-26) estão verdes na tentativa 1. O critério 8/8 da main foi substituído por
# decisão do proprietário (2026-11-26).
