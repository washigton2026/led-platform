#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
# LUMYX-BUILDER — feature-construction workflow harness.
#
# The executable half of the LUMYX-BUILDER agent team: every feature carries
# the mandatory output (Motivação · Design · Implementação · Testes · Rollback
# · Evidência) as a *feature workbook*, and a feature is only "done" when the
# workbook is complete AND LUMYX-GUARDIAN clears the diff.
#
#   ./scripts/lumyx_builder.sh new <slug>     # scaffold docs/features/<slug>.md
#   ./scripts/lumyx_builder.sh check <slug>   # validate workbook + run guardian
#
# Exit 0 = feature workbook complete + 0 regressions. Exit 1 = incomplete/blocked.
# ══════════════════════════════════════════════════════════════════════════
set -u
cd "$(dirname "$0")/.."

RED='\033[0;31m'; GREEN='\033[0;32m'; BOLD='\033[1m'; NC='\033[0m'
CMD="${1:-}"; SLUG="${2:-}"
[ -z "$CMD" ] || [ -z "$SLUG" ] && {
    echo "usage: lumyx_builder.sh new|check <feature-slug>"; exit 2; }
WB="docs/features/$SLUG.md"

case "$CMD" in
new)
    mkdir -p docs/features
    if [ -f "$WB" ]; then echo "workbook already exists: $WB"; exit 1; fi
    cat > "$WB" <<'TEMPLATE'
# Feature: <título>

Subagente responsável: <rust|dsp|network|realtime|product|drone|security>-architect

## Motivação
<!-- o problema concreto, amarrado a uma necessidade real (ex.: o rig de 5 robôs) -->

## Design
<!-- arquitetura, seam tocado, contrato de dados -->

## Implementação
<!-- o plano e os arquivos tocados -->

## Testes
<!-- testes adicionados; OBRIGATÓRIO incluir a linha "Teste negativo:" com a
     rodada descrita que FALHA se a propriedade regredir (anti KB-012) -->
Teste negativo:

## Rollback
<!-- como reverter (arquivo inteiro, nunca patch inline em violação) -->

## Evidência
<!-- comando + saída provando que funciona -->
```
$ <comando>
<saída>
```
TEMPLATE
    echo "workbook criado: $WB — preencha as 6 seções"
    ;;
check)
    [ -f "$WB" ] || { echo -e "${RED}❌${NC} workbook não existe: $WB"; exit 1; }
    FAIL=0
    # Every mandatory section must exist AND have content beyond the template.
    for sec in "Motivação" "Design" "Implementação" "Testes" "Rollback" "Evidência"; do
        if ! grep -q "^## $sec" "$WB"; then
            echo -e "${RED}❌${NC} seção ausente: $sec"; FAIL=1; continue
        fi
        body=$(awk "/^## $sec/{f=1;next} /^## /{f=0} f" "$WB" \
               | grep -vE '^\s*$|^<!--|^-->' | grep -cv '^\s*$')
        if [ "$body" -lt 1 ]; then
            echo -e "${RED}❌${NC} seção vazia: $sec"; FAIL=1
        else
            echo -e "${GREEN}✅${NC} $sec"
        fi
    done
    # Negative test is non-negotiable (KB-012).
    if grep -q "Teste negativo:" "$WB" && \
       [ -n "$(sed -n 's/.*Teste negativo:\(.*\)/\1/p' "$WB" | tr -d '[:space:]')" ] || \
       grep -A2 "Teste negativo:" "$WB" | tail -2 | grep -qE '[a-zA-Z]'; then
        echo -e "${GREEN}✅${NC} teste negativo descrito"
    else
        echo -e "${RED}❌${NC} 'Teste negativo:' ausente ou vazio (KB-012)"; FAIL=1
    fi
    # Evidence must contain an actual command block.
    if awk '/^## Evidência/{f=1} f' "$WB" | grep -qE '^\$ |^```'; then
        echo -e "${GREEN}✅${NC} evidência contém comando"
    else
        echo -e "${RED}❌${NC} evidência sem bloco de comando"; FAIL=1
    fi
    [ "$FAIL" -ne 0 ] && { echo -e "${RED}${BOLD}❌ workbook incompleto${NC}"; exit 1; }

    # Workbook complete → hand the tree to LUMYX-GUARDIAN.
    echo -e "\n${BOLD}→ entregando ao LUMYX-GUARDIAN…${NC}"
    if ./scripts/lumyx_guardian.sh; then
        echo -e "${GREEN}${BOLD}✅ FEATURE '$SLUG' APROVADA — workbook completo + 0 regressões${NC}"
        exit 0
    else
        echo -e "${RED}${BOLD}❌ guardião bloqueou — feature não está pronta${NC}"
        exit 1
    fi
    ;;
*)
    echo "usage: lumyx_builder.sh new|check <feature-slug>"; exit 2 ;;
esac
