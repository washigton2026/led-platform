#!/usr/bin/env bash
# Corre o teste TD-030 N vezes, isolado (--exact), saída por iteração em ficheiro (KB-013).
# Uso: corre_n.sh <binario> <N> <pasta_saida> [nome_do_teste]
set -u
BIN="$1"; N="$2"; OUT="$3"; T="${4:-reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency}"
mkdir -p "$OUT"
falhas=0
for i in $(seq 1 "$N"); do
  "$BIN" "$T" --exact --test-threads=1 > "$OUT/it-$i.out" 2>&1
  rc=$?
  echo "$i $rc" >> "$OUT/exits.txt"
  [ "$rc" -ne 0 ] && falhas=$((falhas+1))
done
passou=$(grep -c ' 0$' "$OUT/exits.txt")
executados=$(grep -lE '^test result: (ok|FAILED)\. 1 passed|^test result: FAILED\. 0 passed; 1 failed' "$OUT"/it-*.out | wc -l | tr -d ' ')
echo "N=$N falhas=$falhas passou=$passou iteracoes_com_1_teste=$executados arch=$(uname -m)" | tee "$OUT/RESUMO.txt"
