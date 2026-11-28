# TD-038 — barreira de arranque nos testes de concorrência do led-pixel-engine (classe «spawn sem barreira»)
git-hash: ef0f1f2
watched: crates/led-pixel-engine/src/reactive.rs sha256:3eec5bbc9288e684f5de80a32fc8d609b38d0f853ddddf358eeb9fd01b5d9149
watched: crates/led-pixel-engine/src/seqlock.rs sha256:f34e49904de830fd93910712801595f4351bf73304a4558debf980eae19b9f74
source_files: crates/led-pixel-engine/src/reactive.rs, crates/led-pixel-engine/src/seqlock.rs
required_test: audioshare_scalars_beat_timestamp_coherent_under_concurrency
data: 2026-11-27

# PORQUE EXISTE
#
# Accept v3, aprovado pelo proprietário a 2026-11-27: ~/lumyx-evidence/2026-11-26-td038/accept-v3.md
# (sha256 6b7f07e2739d2339d86c29fa06b7c76744463128a01d8dc08e69c78a861dfbe6).
# Classe: testes de concorrência do led-pixel-engine em que a leitora é lançada com `spawn` em `while !parar` e o
# escritor não espera por ela; a leitora pode não fazer nenhuma iteração.
#
# Os cinco «pode» do §3.2 do accept:
#   T1 reactive::tests::frescura_pelo_scalars_e_monotonia_por_leitor
#      - falhou no windows: run 46082339639, job 140783924395, `lidas > 0`.
#   T2 reactive::tests::fallbacks_medidos_com_escritor_continuo
#      - falhou no windows: run 45874977001, job 140099654308, `leituras > 0`.
#   T3 reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency
#      - dava VERDE com 0 leituras;
#      - é o required_test deste TD (Q3) e do TD-030 e do TD-002.
#   T4 reactive::adversarial_tests::audioshare_isolamento_entre_instancias
#      - dava VERDE com 0 leituras.
#   T5 seqlock::tests::frescura_concorrente_monotonica_por_leitor
#      - reprovaria por `lidas > 0`; nunca observado a falhar na CI.
#
# Correção (§3.3), só em código #[cfg(test)]:
#   - Barreira de arranque `Arranque`/`Leitora`, em reactive.rs `mod tests`, usada por T1–T4. Inline em T5, no
#     seqlock.rs, sem novas menções a miri e sem tocar nas linhas dos canários M01/M05.
#   - O escritor espera, com prazo de 5 s, que cada leitora tenha lido uma vez antes da 1.ª publicação, e outra vez
#     depois de metade das publicações.
#   - T3 e T4 ganham `assert!(… > 0)` e imprimem o contador (TD038_T3 / TD038_T4, o mesmo formato do patch de medição).
#
# Linha de base «antes» (§2), lida e aceite pelo CIO: ~/lumyx-evidence/2026-11-27-td038-linha-de-base/.
#   - 450/450 ok isolados, 0 com leituras=0.
#   - NOTA-CIO.md: o `--exact` subestima o risco (as falhas reais foram no job completo, com 92 testes em paralelo).
#   - A prova da correção é a mutação determinística da §4, não a frequência.

## 1. Execução da correção — local (macOS x86_64, rustc 1.96.0), `cargo test -p led-pixel-engine --locked`, exit 0
#    clippy --workspace --all-targets --locked -- -D warnings: exit 0.
#    Contadores, uma execução --exact --nocapture:
#      - T1 85 857;
#      - T2 108 848;
#      - T3 43 995;
#      - T4 52 295–68 301 por instância;
#      - T5 173 759–192 659 por leitor.

--- prova ---
```
     Running unittests src/lib.rs (target/debug/deps/led_pixel_engine-780217dfdee098b1)
test reactive::adversarial_tests::audioshare_isolamento_entre_instancias ... ok
test reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency ... ok
test reactive::tests::fallbacks_medidos_com_escritor_continuo ... ok
test reactive::tests::frescura_pelo_scalars_e_monotonia_por_leitor ... ok
test seqlock::tests::frescura_concorrente_monotonica_por_leitor ... ok
test result: ok. 92 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.71s
```
--- fim da prova ---

## 2. Controlo negativo (§4) — mutação por SINAL, 20/20 por teste e por variante
# Mutação (script ~/lumyx-evidence/2026-11-27-td038-fix/neg/mutar.py), aplicada a cópias `git archive`:
#   - antigo = ef0f1f2;
#   - novo = 0af2c1c.
# Cada cópia tem o seu `cargo clean -p led-pixel-engine` e `touch`. Os 5 binários têm sha256 distintos (bins.txt).
# Uma 1.ª tentativa com target partilhado deu o MESMO binário nas 5 variantes; foi descartada
# (bins-invalido-reaproveitado.txt).
# A thread em laço só arranca quando o escritor TERMINOU (TD038_TERMINOU, gravado depois de `parar`) OU está
# bloqueado à espera dela (a_espera, que só existe na versão nova). Nunca sleep.
# Variante «novo-sem-espera-mut»: o escritor deixa de esperar E de sinalizar «à espera»; a leitora continua a
# sinalizar. Leitura do «remover a espera, manter o sinal» do §4.
#
# | variante              | T1              | T2                        | T3                      | T4                       | T5              |
# |-----------------------|-----------------|---------------------------|-------------------------|--------------------------|-----------------|
# | antigo + mutação      | FAILED 20/20    | FAILED 20/20, leituras=0  | VERDE 20/20, leituras=0 | VERDE 20/20, 160×0       | FAILED 20/20    |
# | novo                  | ok 20/20, >0    | ok 20/20, >0              | ok 20/20, >0            | ok 20/20, >0             | ok 20/20, >0    |
# | novo + mutação        | ok 20/20, >0    | ok 20/20, >0              | ok 20/20, >0            | ok 20/20, >0             | ok 20/20, >0    |
# | novo + mut. sem `> 0` | ok 20/20, >0    | ok 20/20, >0              | ok 20/20, >0            | ok 20/20, >0             | ok 20/20, >0    |
# | novo sem espera + mut.| FAILED 20/20    | FAILED 20/20, leituras=0  | FAILED 20/20, 0         | FAILED 20/20, 160×0      | FAILED 20/20    |
#
# Motivos, sempre o mesmo nas 20 execuções:
#   - antigo + mutação:
#       T1 `assertion failed: lidas > 0` (reactive.rs:544);
#       T2 `assertion failed: leituras > 0` (reactive.rs:693);
#       T5 `o leitor não leu nada` (seqlock.rs:217).
#   - novo sem espera + mutação:
#       T1 reactive.rs:618 e T2 reactive.rs:772, as asserções de sempre;
#       T3 `TD-038: a leitora não leu nada` (reactive.rs:1059);
#       T4 `TD-038: leitoras sem nenhuma leitura` (reactive.rs:1180);
#       T5 `o leitor não leu nada` (seqlock.rs:245).
#   - Os números de linha são das cópias mutadas, por isso são maiores do que no ficheiro real.
# Leitura:
#   - O antigo reprova (T1, T2, T5) ou passa em VAZIO (T3, T4, com contador 0 impresso pela mutação).
#   - O novo passa com contador > 0, mesmo sem as asserções `> 0`.
#   - Sem a espera, o novo `> 0` de T3/T4 sabe reprovar.
# Revertida a mutação («novo»): verde 20/20.
# Resultado completo: ~/lumyx-evidence/2026-11-27-td038-fix/neg/resultado.tsv e runs/.
# Execução LOCAL, macOS x86_64.

## 3. O que isto NÃO prova
- Não prova a correção do seqlock nem do AudioShare; isso continua a ser o TD-030.
- A barreira garante leituras concorrentes (≥1 no arranque, ≥1 depois de metade), não um número de intercalações.
- Não é a CI. O fecho exige:
  - a CI do PR lida job a job, incluindo o windows (allow-failure);
  - a medição «depois» por SO (§5.5).
- Não é arm64: o §4 correu em x86_64 local; a arm64 só entra pela CI (macos-26) e pela medição «depois».

## 4. Fecho (critério 7 do accept v3), 2026-11-28 — CI e medição lidas no log, job a job
# A região de prova e os watched acima não mudaram: reactive.rs e seqlock.rs da main 1975984 têm os mesmos sha256
# (o código é o de c3b9dd0). Leituras completas em ~/lumyx-evidence/, cada pasta com SHA256SUMS (verificados a 2026-11-28, 0 falhas).
#
# 4.1 CI do PR #70 (fix/td-038-barreira-de-arranque @ c3b9dd0), run 46137292750, 9/9 jobs success.
#   ~/lumyx-evidence/2026-11-27-td038-fix/ci-c3b9dd0/TABELA.txt
#     sha256 1001eb478cae237d42898ae3df4b761052454f327bd3a436092ab0bd3a0430dc
#   - test (ubuntu-24.04), test (macos-26), windows (allow-failure): T1–T5 «... ok» nos 3; 109 linhas «test result: ok.»
#     em cada um; 1213 / 1217 / 1105 passed, 0 failed, 10 ignored; nenhum «... FAILED».
#   - miri (seqlock): «miri seqlock: 3/3 testes (piso 3), exit 0»; canários M01 (seqlock.rs:206:21) e M05 (:207:21)
#     mortos; «árvore reposta».
#   - loom: «TD-030 gate: OK — std 14 oráculos, loom 2/2, 7 canários mortos.»
#   - debt gate: «Result: 0 Critical, 0 Warning, 24 OK · 10 open · 2 wontfix»; gate suite: «esperados 36», 36 passed.
#
# 4.2 CI da main sobre o merge do #70 (1975984), run 46138069440 (push), 9/9 jobs success.
#   ~/lumyx-evidence/2026-11-27-td038-fix/ci-main-19759840/TABELA.txt
#     sha256 b827328385da6803e4b112855aa3400f6be6cb3b1c054d3227f692153ff28a29
#   - Os mesmos números de 4.1 nos 3 jobs de teste, no miri (seqlock), no loom, no debt gate e na gate suite.
#
# 4.3 Controlo negativo da §4 (secção 2 acima), resultado por teste e por variante:
#   ~/lumyx-evidence/2026-11-27-td038-fix/neg/resultado.tsv
#     sha256 e4af777759e27ec701e3afdd9ac52419ed70bea372d94e810994ea5560704e8b
#
# 4.4 Antes / depois por SO (§2 e §5.5): N=30 por teste, --exact --nocapture, o mesmo medir.sh.
#   antes  = PR #69 @ cd5809d (fechado sem merge), runs 46084441257 e 46084441204
#     ~/lumyx-evidence/2026-11-27-td038-linha-de-base/ci-cd5809d/TABELA.txt
#       sha256 4879e118107c80ba6c0cf1cbe498cf763aebb382891cb39d52520d995c0ab9ee
#     linha-de-base.tsv sha256 e265a1db2b533ad5aee55cc1192e8d887e713f6529d1c9b59916020ebd3d253a
#   depois = PR #71 @ 28df81c (sobre 1975984; fechado sem merge), runs 46138787130 e 46138787137
#     patch de instrumentação de T3/T4 dispensado (accept v3 §2; decisão do proprietário, 2026-11-28): a correção já
#     imprime TD038_T3/TD038_T4; o passo verifica árvore limpa e as 2 marcas exatas (marcas=2 t3=1 t4=1 nos 3 SO).
#     ~/lumyx-evidence/2026-11-28-td038-depois/ci-28df81c/TABELA.txt
#       sha256 5ef0fddd53b6a9eb96af2ce3763ee51b70e92f50cf9e52c2c3af5d14e6c26b11
#     linha-de-base.tsv sha256 3d99ba89276c3a2450c0b5e0252e120ea1a7f8dfae73d38d2358ba9f246ce4ec
#   Nos dois, ubuntu-24.04 x86_64, macos-26 arm64 e windows-latest x86_64, T1–T5: ok 30/30, FAILED 0, ok_sem_marca 0,
#   leituras=0 em 0. Mín/mediana/máx por SO e por teste nas duas TABELAS (aceite pelo CIO, 2026-11-28).
#
# 4.5 O que o fecho NÃO acrescenta à secção 3: a frequência por --exact continua a não medir o job completo; a prova
#   da correção continua a ser a mutação determinística da §4. Não reabre o TD-030.
