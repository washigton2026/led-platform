#!/usr/bin/env bash
#
# Gate de COMPILAÇÃO do contrato TypeScript gerado (ADR-0027, F7/B).
#
# O gate Rust↔TS (`cargo test -p led-console-bin --test contract_gate`) prova que o
# `.ts` **corresponde** ao Rust. Não prova que o `.ts` **compila** — são duas
# propriedades independentes, e uma não substitui a outra. Este script fecha a segunda.
#
#   ./scripts/tsc_gate.sh
#
# Sai 0 se compilar, != 0 caso contrário. **Não salta**: se o toolchain faltar, isso é
# uma FALHA e não um "nada a verificar" — um gate que passa por não ter corrido é a
# forma mais barata do KB-012, e este repo já foi mordido por ela (Miri N=0).

set -u

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# DOIS projectos, propriedades diferentes (ADR-0028 D2):
#   contract   — os tipos GERADOS compilam, e as assercoes invertidas do verifica.ts valem
#   console-web — a APP usa esses tipos correctamente
# Nenhum substitui o outro. O gate so passa se AMBOS passarem.
DIR="$RAIZ/crates/led-console-bin/contract"
APP="$RAIZ/console-web"

falhar() {
  echo "FAIL: $*" >&2
  exit 1
}

command -v node >/dev/null 2>&1 || falhar "\`node\` nao esta no PATH. O gate exige um toolchain Node (ADR-0016 aceitou-o como custo do React+TS)."
command -v npm  >/dev/null 2>&1 || falhar "\`npm\` nao esta no PATH."

[ -f "$DIR/lumyx-contract.generated.ts" ] || falhar "contrato gerado ausente. Corra: cargo run -p led-console-bin --example gerar_contrato"
[ -f "$DIR/verifica.ts" ]                 || falhar "verifica.ts ausente — sem ele, o tsc compilaria tipos sem os usar, e provaria quase nada."

if [ ! -d "$DIR/node_modules/typescript" ]; then
  echo "typescript ausente; a instalar (dep unica, pinada no package.json)..."
  npm install --prefix "$DIR" --silent || falhar "npm install falhou."
fi

# O console-web so entra no gate se existir. Nao e "saltar": e a fatia anterior a esta
# nao o ter criado ainda. Se existir, e OBRIGATORIO — nao ha modo permissivo.
if [ -d "$APP" ]; then
  [ -f "$APP/tsconfig.json" ] || falhar "console-web existe mas nao tem tsconfig.json"
  [ -f "$APP/src/App.tsx" ]   || falhar "console-web existe mas nao tem src/App.tsx"
  if [ ! -d "$APP/node_modules/typescript" ]; then
    echo "typescript ausente em console-web; a instalar (lockfile exato)..."
    npm ci --prefix "$APP" --silent || falhar "npm ci falhou em console-web."
  fi
fi

# `npx tsc` NAO serve: no registo npm, o pacote \`tsc\` e um stub antigo (2.0.4) que
# NAO e o TypeScript. Invocamos o binario instalado, para nao haver ambiguidade.
TSC="$DIR/node_modules/typescript/bin/tsc"
[ -x "$TSC" ] || [ -f "$TSC" ] || falhar "binario do tsc nao encontrado em $TSC"

falhou=0

echo "── contract: tsc --noEmit sobre o contrato gerado + verifica.ts"
if node "$TSC" --noEmit --project "$DIR/tsconfig.json"; then
  echo "   OK"
else
  echo "   FALHOU" >&2
  falhou=1
fi

if [ -d "$APP" ]; then
  TSC_APP="$APP/node_modules/typescript/bin/tsc"
  [ -f "$TSC_APP" ] || falhar "binario do tsc nao encontrado em $TSC_APP"
  echo "── console-web: tsc --noEmit sobre a app"
  if node "$TSC_APP" --noEmit --project "$APP/tsconfig.json"; then
    echo "   OK"
  else
    echo "   FALHOU" >&2
    falhou=1
  fi

  # O typecheck prova que os tipos batem; NAO prova que a logica faz o que diz.
  # `descreveEvento` compila na mesma se devolver a string errada, e `interpretarErro`
  # compila na mesma se reescrever o codigo do daemon. Sao propriedades diferentes, e
  # por isso correm as duas.
  if [ -d "$APP/node_modules/vitest" ]; then
    echo "── console-web: testes"
    if (cd "$APP" && node node_modules/vitest/vitest.mjs run --reporter=dot); then
      echo "   OK"
    else
      echo "   FALHOU" >&2
      falhou=1
    fi
  else
    falhar "vitest ausente em console-web — o gate NAO salta testes."
  fi
fi

# Os dois correm SEMPRE, mesmo que o primeiro falhe: um gate que aborta no primeiro erro
# esconde o segundo, e quem o corre volta a correr as vezes que houver projectos.
[ "$falhou" -eq 0 ] || falhar "o gate do TypeScript reprovou (ver acima QUAL dos passos)."
echo "PASS: contrato e console-web compilam, os testes passam, e as assercoes de tipo valem."
exit 0
