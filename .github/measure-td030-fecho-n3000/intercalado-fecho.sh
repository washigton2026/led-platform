#!/usr/bin/env bash
# Fecho do TD-030, N=3000 (D7, opção B): intercala N execuções de F (seqlock, 7aeec49) e C (pré-correção 28be05e =
# ArcSwap + teste de isolamento, controlo), filtro reactive::adversarial_tests — as condições intercaladas do #53.
# Saída por iteração (KB-013). Accept: ~/lumyx-evidence/2026-11-25-td030-medicao/accept.md (regra fixada antes).
set -u
F="$1"; C="$2"; N="$3"; OUT="$4"; mkdir -p "$OUT/F" "$OUT/C"
for i in $(seq 1 "$N"); do
  "$F" reactive::adversarial_tests > "$OUT/F/it-$i.out" 2>&1; echo "$i $?" >> "$OUT/F/exits.txt"
  "$C" reactive::adversarial_tests > "$OUT/C/it-$i.out" 2>&1; echo "$i $?" >> "$OUT/C/exits.txt"
done
for v in F C; do
  n=$(ls "$OUT/$v"/it-*.out | wc -l | tr -d ' ')
  com12=$(grep -l '^test result: .* 12 passed; 0 failed\|^test result: FAILED\. .* 12 filtered\|running 12 tests' "$OUT/$v"/it-*.out | wc -l | tr -d ' ')
  o=$(grep -l 'audioshare_scalars_beat_timestamp_coherent_under_concurrency ... FAILED' "$OUT/$v"/it-*.out 2>/dev/null | wc -l | tr -d ' ')
  iso=$(grep -l 'audioshare_isolamento_entre_instancias ... FAILED' "$OUT/$v"/it-*.out 2>/dev/null | wc -l | tr -d ' ')
  viol=$(grep -lE 'audioshare_(scalars_beat_timestamp_coherent_under_concurrency|isolamento_entre_instancias) \.\.\. FAILED' "$OUT/$v"/it-*.out 2>/dev/null | wc -l | tr -d ' ')
  outros=$(grep -ahE '^test .* FAILED$' "$OUT/$v"/it-*.out | grep -vE 'coherent_under_concurrency|isolamento_entre_instancias' | sort | uniq -c | tr '\n' ';')
  echo "$v N=$n iteracoes_com_12_testes=$com12 coerencia_original=$o isolamento=$iso iteracoes_com_violacao=$viol outros=[$outros] arch=$(uname -m)"
done | tee "$OUT/RESUMO.txt"
