# TD-034 — os testes UDP sobre loopback deixam de depender do buffer de receção do SO
git-hash: 2267dbd
watched: crates/led-protocols/src/artnet.rs sha256:c1a148d533f326394bbbb585c1a01e16b9c6a1821484a21aa91838e9e8e1ae94
watched: integration-tests/tests/udp_chaos.rs sha256:0ab7d828185557db508ab6cb8863166d5d1d2721525a9c391f5b9c1a3403c6d5
watched: integration-tests/src/lib.rs sha256:3414274e07c8d8b23492a810322cedaf74363bafa4fbcac6b902970b4392e63e
source_files: crates/led-protocols/src/artnet.rs, integration-tests/tests/udp_chaos.rs, integration-tests/src/lib.rs
required_test: artnet_sequence_is_per_universe_and_wraps_skipping_zero
data: 2026-11-21

# PORQUE EXISTE
#
# Fecha o TD-034. A correção é do PR #57 (fix/td-034-udp-loopback, head 2267dbd), mergeado na main em 4692ae2.
# Os três source_files são byte-idênticos entre 2267dbd e 4692ae2 (`git diff 2267dbd 4692ae2 -- <os três>` vazio)
# e têm exatamente os sha256 acima, medidos sobre 4692ae2.
# O ledger pede dois testes; o gate aceita um só required_test. Fica o do artnet (2 ocorrências no flakes.log,
# D-REP STOP). O same_seed_same_wire_drop_count está na mesma região de prova, com o resumo do seu binário.

## 1. Execução — run 44160888889 (PR #57, 2267dbd), job test (ubuntu-24.04), 5 tentativas
# Logs lidos por job, via API (`/actions/jobs/<id>/logs`), ANSI e carimbo temporal removidos.
# ATENÇÃO: `gh run view --log --job <id>` devolve o log da ÚLTIMA tentativa para qualquer id; não serve para
# distinguir tentativas. Os cinco logs abaixo vieram da API e têm carimbos distintos.
#
# | tentativa | job          | início (UTC)          | udp_chaos   | led_protocols lib |
# |-----------|--------------|-----------------------|-------------|-------------------|
# | 1         | 134314074716 | 2026-11-17T19:39:18Z  | 5 passed    | 82 passed         |
# | 2         | 134356324435 | 2026-11-17T20:46:47Z  | 5 passed    | 82 passed         |
# | 3         | 134358308070 | 2026-11-17T20:49:59Z  | 5 passed    | 82 passed         |
# | 4         | 135579832822 | 2026-11-19T13:04:17Z  | 5 passed    | 82 passed         |
# | 5         | 136867257680 | 2026-11-21T13:55:00Z  | 5 passed    | 82 passed         |
#
# Em todas: Image: ubuntu-24.04 · toolchain stable-x86_64-unknown-linux-gnu, rustc 1.100.0 (0ef1bc386 2026-11-09).
# Nas cinco, os dois testes do TD-034 aparecem como `... ok` e nenhum como FAILED.
# O job test (macos-26) passou nas tentativas 2–5 (134356324498, 134358308000, 135579832816, 136867257721);
# na 1 foi cancelado (134314074776). Esses logs não foram extraídos para aqui.
#
# A região de prova é a tentativa 5 (job 136867257680), na ordem em que o log a imprime:

--- prova ---
```
Image: ubuntu-24.04
stable-x86_64-unknown-linux-gnu unchanged - rustc 1.100.0 (0ef1bc386 2026-11-09)
     Running tests/udp_chaos.rs (target/debug/build/integration-tests/b161d5e25ee08b51/out/udp_chaos-b161d5e25ee08b51)
test same_seed_same_wire_drop_count ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
     Running unittests src/lib.rs (target/debug/build/led-protocols/66b5dee2b042d14d/out/led_protocols-66b5dee2b042d14d)
test artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero ... ok
test result: ok. 82 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
```
--- fim da prova ---

## 2. Controlos negativos (N1, N2, N3)
# Corridos localmente sobre o commit base 4692ae24aaa4490ad3d356a5a7e224abc4e7cc2a, numa worktree temporária.
# Máquina: uname -m x86_64 · Darwin 23.6.0 · rustc 1.96.0 (ac68faa20 2026-05-25).
# Artefactos completos (mutação em git diff, comando, saída, exit) em
# ~/lumyx-evidence/2026-11-21-td034-controlos-negativos/, com SHA256SUMS (`shasum -a 256 -c` OK nos 7 ficheiros).
# Cada caso: vermelho com a mutação, verde depois de reposta. Base sem mutação: 00-base-verde.txt (os dois testes ok).
#
# N1-299: o teste envia 299 em vez de 300 (artnet.rs, `enviar_em_lotes(&dev, &rx, 299)`).
#   $ cargo test -p led-protocols --lib artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero
#   test artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero ... FAILED
#   recebidos 299 de 300; última sequência vista Some(44); primeira em falta: índice 299 (esperada 45, vista None); erro de leitura: None
#   exit=101 · reposto: 1 passed; 0 failed, exit=0
#
# N2-salto: o emissor salta a sequência 101 (artnet.rs, `else if *seq == 100 { 102 }` em ArtNetDevice).
#   $ cargo test -p led-protocols --lib artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero
#   test artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero ... FAILED
#   recebidos 300 de 300; última sequência vista Some(46); primeira em falta: índice 100 (esperada 101, vista Some(102)); erro de leitura: None
#   exit=101 · reposto: 1 passed; 0 failed, exit=0
#
# N3-199: o proxy recebe 199 em vez de 200 (udp_chaos.rs, último lote com n - 1).
#   $ cargo test -p integration-tests --test udp_chaos same_seed_same_wire_drop_count
#   test same_seed_same_wire_drop_count ... FAILED
#   assertion `left == right` failed: pré-condição: o proxy leu 199 de 200 datagramas (seed 99); faltam antes do proxy (descarte do SO ou envio falhado), não por perda injetada — não é divergência de determinismo
#   exit=101 · reposto: 1 passed; 0 failed, exit=0
#
# Nos três, a falha é a mensagem de diagnóstico pedida pelo falsification_required, nunca um pânico de unwrap(),
# e nenhum é vermelho de compilação.

## 2b. Controlos negativos na CI — ubuntu-24.04 e macos-26
# Um PR [NEG-CTL] por mutação (fechados sem merge), cada um com um só commit sobre 4692ae2, só com a mutação
# (a mesma dos ficheiros *-vermelho.txt, comparada linha a linha) e sem tocar no ci.yml.
# Logs lidos job a job pela API (`/actions/jobs/<id>/logs`), ANSI e carimbo temporal removidos; cópias em
# ~/lumyx-evidence/2026-11-21-td034-controlos-negativos/ci-negctl/ (SHA256SUMS).
# Arquitetura: Runner Image e Operating System do «Set up job» + triplo do toolchain no passo de instalação do Rust.
# Os seis: rustc 1.100.0 (0ef1bc386 2026-11-09), imagem Version 20261111.635, 0 `error[E`.
#
# | caso     | PR  | commit  | run         | job          | imagem · SO                          | toolchain                 | resumo do binário      |
# |----------|-----|---------|-------------|--------------|--------------------------------------|---------------------------|------------------------|
# | N1-299   | #58 | e6b59e5 | 44958338412 | 137011339746 | ubuntu-24.04 · Ubuntu 24.04.5 LTS    | x86_64-unknown-linux-gnu  | 81 passed; 1 failed    |
# | N1-299   | #58 | e6b59e5 | 44958338412 | 137011339571 | macos-26-arm64 · macOS 26.7.2 25G332 | aarch64-apple-darwin      | 81 passed; 1 failed    |
# | N2-salto | #59 | 89c9397 | 44958356888 | 137011395815 | ubuntu-24.04 · Ubuntu 24.04.5 LTS    | x86_64-unknown-linux-gnu  | 81 passed; 1 failed    |
# | N2-salto | #59 | 89c9397 | 44958356888 | 137011395779 | macos-26-arm64 · macOS 26.7.2 25G332 | aarch64-apple-darwin      | 81 passed; 1 failed    |
# | N3-199   | #60 | badfcf9 | 44958376005 | 137011454533 | ubuntu-24.04 · Ubuntu 24.04.5 LTS    | x86_64-unknown-linux-gnu  | 4 passed; 1 failed     |
# | N3-199   | #60 | badfcf9 | 44958376005 | 137011454564 | macos-26-arm64 · macOS 26.7.2 25G332 | aarch64-apple-darwin      | 4 passed; 1 failed     |
#
# N1-299, nos dois jobs:
#   test artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero ... FAILED
#   panicked at crates/led-protocols/src/artnet.rs:611:13:
#   recebidos 299 de 300; última sequência vista Some(44); primeira em falta: índice 299 (esperada 45, vista None); erro de leitura: None
# N2-salto, nos dois jobs:
#   test artnet::tests::artnet_sequence_is_per_universe_and_wraps_skipping_zero ... FAILED
#   panicked at crates/led-protocols/src/artnet.rs:611:13:
#   recebidos 300 de 300; última sequência vista Some(46); primeira em falta: índice 100 (esperada 101, vista Some(102)); erro de leitura: None
# N3-199, nos dois jobs:
#   test same_seed_same_wire_drop_count ... FAILED
#   panicked at integration-tests/tests/udp_chaos.rs:145:9:
#   assertion `left == right` failed: pré-condição: o proxy leu 199 de 200 datagramas (seed 99); faltam antes do proxy (descarte do SO ou envio falhado), não por perda injetada — não é divergência de determinismo
#
# Nos cinco primeiros jobs, o teste esperado é o ÚNICO FAILED. No N3/macos-26 (job 137011454564) falhou também
# reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency (reactive.rs:488,
# «coherence violated 1 times», 71 passed; 1 failed). É o detetor do TD-030, aberto na base 4692ae2 (correção no
# #56); a mutação do N3 só toca em integration-tests/tests/udp_chaos.rs. Não conta para o TD-034 e não lhe é
# atribuível, mas fica registado: o vermelho desse job não é só do N3.

## 3. O que isto NÃO mede
- Não há `uname -m` nos logs (o ci.yml não foi alterado, por decisão). A arquitetura da secção 2b vem da Runner
  Image e do triplo do toolchain, lidos no log do mesmo job que falha.
- Os controlos da secção 2 (locais) e da 2b (CI) são as mesmas mutações; o verde depois de reposto vem da base
  4692ae2 (secção 1 na CI, ficheiros *-reposto-verde.txt localmente), não de um re-run dos branches NEG-CTL.
- Cinco verdes seguidos no ubuntu-24.04 não provam que a rajada nunca excede o buffer de receção. Provam que o
  teste deixou de depender dele: a leitura é feita em lotes, e uma perda passa a dar diagnóstico em vez de WouldBlock.
- A via (4) do required_fix (SO_RCVBUF com socket2) não foi precisa e não foi feita.
