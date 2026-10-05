#!/usr/bin/env bash
# Auto-verificação dos scripts de gate (W1). Sem cargo: não compila nem corre os gates.
#
#   1. Sintaxe: `bash -n` em cada .sh e carga YAML de cada workflow — um gate que não
#      analisa sintaticamente não prova nada.
#   2. Tripwire KB-012: uma linha que olha para o resumo do libtest (`test result`/`result: ok`)
#      sem fixar o N NO PRÓPRIO PADRÃO (`… ok. 5 passed`, `… ok\. [0-9]+ passed`) aceita
#      «ok. 0 passed». As linhas que já existem estão listadas, literalmente, em
#      tests/fixtures/kb012_baseline.txt (cada uma é um TD-DRAFT). Uma linha nova reprova — mesmo
#      que outra tenha sido corrigida no mesmo diff — e uma linha da baseline que desapareceu
#      obriga a tirá-la da baseline no mesmo commit.
#
# O QUE ISTO NÃO PROVA (limite declarado, não lacuna escondida): que um N extraído é COMPARADO
# com o N esperado (`[0-9]+ passed` seguido de nada, ou de `-ge 0`), nem gates que só confiam no
# exit do cargo. Isso só se prova executando o gate com um filtro a 0 testes. Também não vê: texto
# partido por variáveis ou continuação de linha; uma linha corrigida e, no mesmo diff, outra com o
# MESMO texto acrescentada (a impressão digital é por texto, não por posição); bash inválido dentro
# de um `run:` de YAML válido. Tudo isto fica visível em review.
#
# Uso: tests/test_gate_scripts.sh [raiz]   (raiz por omissão: o repo)
set -u
ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
BASELINE="$ROOT/tests/fixtures/kb012_baseline.txt"
falhas=0

scripts=$(cd "$ROOT" && ls scripts/*.sh scripts/gates/*.sh .github/workflows/*.yml 2>/dev/null)
[ -n "$scripts" ] || { echo "FAIL: nenhum script de gate encontrado em $ROOT"; exit 1; }

for s in $scripts; do
  case "$s" in
    *.yml) ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$ROOT/$s" 2>/dev/null \
             || { echo "FAIL sintaxe YAML: $s"; falhas=$((falhas+1)); } ;;
    *)     bash -n "$ROOT/$s" 2>/dev/null \
             || { echo "FAIL sintaxe: $s"; falhas=$((falhas+1)); } ;;
  esac
done

# Linhas vulneráveis de um ficheiro, normalizadas (sem indentação), uma por linha: procuram
# `result: ok` (qualquer caixa) e o padrão não fixa o N. Diagnósticos de `FAILED` não entram.
vulneraveis() {
  grep -iE 'result:.{0,12}ok' "$ROOT/$1" \
    | grep -vE '^[[:space:]]*#' \
    | grep -viE 'ok(\\)?\.? ?\(?(\[0-9\]\+|[0-9]+)\)? passed' \
    | sed -E 's/^[[:space:]]+//'
}

[ -f "$BASELINE" ] || { echo "FAIL: baseline ausente ($BASELINE)"; exit 1; }
atual=$(for s in $scripts; do vulneraveis "$s" | sed "s#^#$s	#"; done | sort)
base=$(grep -v '^#' "$BASELINE" | grep -v '^$' | sort)

novas=$(comm -13 <(printf '%s\n' "$base") <(printf '%s\n' "$atual") | grep -v '^$' || true)
velhas=$(comm -23 <(printf '%s\n' "$base") <(printf '%s\n' "$atual") | grep -v '^$' || true)
if [ -n "$novas" ]; then
  echo "FAIL KB-012: linha(s) nova(s) que aceitam «ok. 0 passed» (fixar o N no padrão, ou N == N_esperado):"
  printf '  %s\n' "$novas"; falhas=$((falhas+1))
fi
if [ -n "$velhas" ]; then
  echo "FAIL baseline desatualizada: linha(s) que já não existem — tirá-las da baseline no mesmo commit:"
  printf '  %s\n' "$velhas"; falhas=$((falhas+1))
fi

n=$(printf '%s\n' "$scripts" | wc -l | tr -d ' ')
if [ "$falhas" -ne 0 ]; then echo "test_gate_scripts: $falhas falha(s) em $n ficheiros"; exit 1; fi
echo "test_gate_scripts: OK — $n ficheiros, sintaxe e tripwire KB-012 ($(printf '%s\n' "$base" | grep -c .) linhas conhecidas)"
