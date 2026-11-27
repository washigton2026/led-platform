#!/usr/bin/env bash
# TD-038 §2 (accept v2): N execuções de cada teste da classe, --exact --nocapture, no binário de testes do
# led-pixel-engine (--lib). Saída por iteração (KB-013); o exit de cada execução fica em <teste>/exits.txt.
# Um FAILED é DADO da linha de base; o script só sai ≠ 0 se a MEDIÇÃO for inválida.
# Accept: ~/lumyx-evidence/2026-11-26-td038/accept-v2.md (§2).
set -u
BIN="$1"; N="$2"; OUT="$3"
# id|teste|marca impressa num verde (vazio = o código não imprime o contador num verde → contagem NOT_MEASURED)
TESTES='T1|reactive::tests::frescura_pelo_scalars_e_monotonia_por_leitor|TD030_FRESCURA
T2|reactive::tests::fallbacks_medidos_com_escritor_continuo|TD030_FALLBACKS
T3|reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency|
T4|reactive::adversarial_tests::audioshare_isolamento_entre_instancias|
T5|seqlock::tests::frescura_concorrente_monotonica_por_leitor|SEQLOCK_FRESCURA'
mkdir -p "$OUT"; : > "$OUT/RESUMO.txt"; invalida=0
while IFS='|' read -r id T marca; do
  d="$OUT/$id"; mkdir -p "$d"
  "$BIN" "$T" --exact --list > "$d/list.out" 2>&1
  listado=$(grep -c ': test$' "$d/list.out")
  echo "== $id $T · N_listado=$listado (esperado 1)"
  [ "$listado" -eq 1 ] || { echo "::error::$id: --exact lista $listado testes, esperado 1 — medição inválida"; invalida=1; continue; }
  for i in $(seq 1 "$N"); do
    "$BIN" "$T" --exact --nocapture > "$d/it-$i.out" 2>&1; echo "$i $?" >> "$d/exits.txt"
  done
  its=$(wc -l < "$d/exits.txt" | tr -d ' ')
  um=$(grep -l '^running 1 test$' "$d"/it-*.out | wc -l | tr -d ' ')
  ok=$(grep -l "^test $T \.\.\. ok$" "$d"/it-*.out | wc -l | tr -d ' ')
  falhou=$(grep -l "^test $T \.\.\. FAILED$" "$d"/it-*.out | wc -l | tr -d ' ')
  if [ -n "$marca" ]; then
    echo "-- linhas $marca"; grep -ah "$marca" "$d"/it-*.out
    nlin=$(grep -ah "$marca" "$d"/it-*.out | wc -l | tr -d ' ')
    zeros=$(grep -ah "$marca" "$d"/it-*.out | grep -c ' leituras=0 ')
    stats=$(grep -ah "$marca" "$d"/it-*.out | grep -aoE 'leituras=[0-9]+' | cut -d= -f2 | sort -n | awk '{a[NR]=$1} END{if(NR) printf "min=%s mediana=%s max=%s", a[1], a[int((NR+1)/2)], a[NR]}')
    cont="linhas_${marca}=$nlin leituras_0=$zeros leituras[$stats]"
  else
    cont="contagem=NOT_MEASURED (o código não imprime o contador num verde)"
  fi
  echo "-- pânicos"; grep -ah 'panicked at\|assertion failed\|leitor não leu nada' "$d"/it-*.out || echo "(nenhum)"
  linha="$id N=$N iteracoes=$its com_1_teste=$um ok=$ok FAILED=$falhou $cont"
  echo "$linha" | tee -a "$OUT/RESUMO.txt"
  [ "$its" -eq "$N" ] && [ "$um" -eq "$N" ] && [ $((ok + falhou)) -eq "$N" ] || { echo "::error::$id: medição inválida (iterações, «running 1 test» ou ok+FAILED ≠ N)"; invalida=1; }
done <<< "$TESTES"
exit $invalida
