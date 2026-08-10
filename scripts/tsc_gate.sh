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
DIR="$RAIZ/crates/led-console-bin/contract"

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

# `npx tsc` NAO serve: no registo npm, o pacote \`tsc\` e um stub antigo (2.0.4) que
# NAO e o TypeScript. Invocamos o binario instalado, para nao haver ambiguidade.
TSC="$DIR/node_modules/typescript/bin/tsc"
[ -x "$TSC" ] || [ -f "$TSC" ] || falhar "binario do tsc nao encontrado em $TSC"

echo "tsc --noEmit sobre o contrato gerado + verifica.ts"
if node "$TSC" --noEmit --project "$DIR/tsconfig.json"; then
  echo "PASS: o contrato TypeScript compila, e as assercoes de tipo do verifica.ts valem."
  exit 0
fi
falhar "o contrato TypeScript NAO compila."
