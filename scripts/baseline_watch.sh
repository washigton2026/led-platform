#!/usr/bin/env bash
# LUMYX baseline watcher — apanha o vermelho intermitente do workspace.
#
# Porque existe: a 2026-09-22 o `cargo test --workspace` falhou UMA vez
# (`o_daemon_recusa_a_linha_longa_por_si_proprio`) e passou nas 54 execucoes
# seguintes. A mensagem de panic perdeu-se porque o comando era
# `cargo test --workspace 2>&1 | tail -5` — as ultimas 5 linhas nao chegam
# para diagnosticar, e o `| tail` mascara o exit do cargo (KB-013).
#
# Este script existe para nunca mais perder essa prova:
#   - o output vai INTEIRO para ficheiro, nunca por um pipe;
#   - o `$?` e lido na linha imediatamente a seguir ao cargo, antes de
#     qualquer outro comando lhe poder tocar;
#   - o grep corre sobre o ficheiro DEPOIS, nunca sobre o stream.
#
# Nao e producao e nao vive em crates/. Nao muta nada: le o workspace tal
# como esta.
#
#   bash scripts/baseline_watch.sh              # uma passagem
#   for i in $(seq 5); do bash scripts/baseline_watch.sh; done
#
# Um VERMELHO preserva o log como /tmp/baseline_RED_<ts>.log — e a prova.

set -uo pipefail

ts=$(date +%Y%m%d_%H%M%S); out="/tmp/baseline_$ts.log"
cargo test --workspace > "$out" 2>&1; ec=$?
echo "EXIT_CARGO=$ec  log=$out"
if [ "$ec" -ne 0 ]; then
  echo ">>> VERMELHO CAPTURADO"
  grep -nE "FAILED|panicked|test result:" "$out"
  echo ">>> panic completo:"; grep -n -A20 "panicked" "$out"
  cp "$out" "/tmp/baseline_RED_$ts.log"   # preserva a prova, nao a engole
else echo ">>> verde ($(grep -c 'test result: ok' "$out") suites ok)"; fi
