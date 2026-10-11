#!/usr/bin/env bash
# Intercala N execuções: R = reprodução mínima (só arc-swap 1.9.1, 12 testes, todos), A = led-pixel-engine de
# produção (main 2cbbc8d) com o filtro reactive::adversarial_tests (controlo positivo nas mesmas condições).
# Saída por iteração em ficheiro (KB-013).
set -u
R="$1"; A="$2"; N="$3"; OUT="$4"; mkdir -p "$OUT/R" "$OUT/A"
for i in $(seq 1 "$N"); do
  "$R" > "$OUT/R/it-$i.out" 2>&1; echo "$i $?" >> "$OUT/R/exits.txt"
  "$A" reactive::adversarial_tests > "$OUT/A/it-$i.out" 2>&1; echo "$i $?" >> "$OUT/A/exits.txt"
done
r_falhas=$(grep -vc ' 0$' "$OUT/R/exits.txt")
r_viol=$(grep -l 'REPRO_VIOLATION' "$OUT/R"/it-*.out 2>/dev/null | wc -l | tr -d ' ')
r_12=$(grep -l '^test result: .* 12 passed\|^test result: FAILED' "$OUT/R"/it-*.out | wc -l | tr -d ' ')
r_outros=$(grep -ahE '^test .* FAILED$' "$OUT/R"/it-*.out | sort | uniq -c | tr '\n' ';')
a_coer=$(grep -l 'audioshare_scalars_beat_timestamp_coherent_under_concurrency ... FAILED' "$OUT/A"/it-*.out 2>/dev/null | wc -l | tr -d ' ')
a_outros=$(grep -ahE '^test .* FAILED$' "$OUT/A"/it-*.out | grep -v coherent_under_concurrency | sort | uniq -c | tr '\n' ';')
{
echo "R N=$N exit_nao_zero=$r_falhas iteracoes_com_REPRO_VIOLATION=$r_viol iteracoes_com_resultado=$r_12 falhados=[$r_outros] arch=$(uname -m)"
echo "A N=$N coerencia_original=$a_coer outros=[$a_outros] arch=$(uname -m)"
} | tee "$OUT/RESUMO.txt"
