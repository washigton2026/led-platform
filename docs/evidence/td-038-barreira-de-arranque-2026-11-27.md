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
