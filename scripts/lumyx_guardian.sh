#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
# LUMYX-GUARDIAN — fast mechanical regression gate.
#
# The executable half of the LUMYX-GUARDIAN agent team: six guardians, each a
# cheap deterministic check, run on every change / PR / release. Designed to
# finish in well under a minute (the Haiku profile: fast, repetitive, low cost)
# — it is NOT the full e2e (lumyx-e2e.sh), it is the guard that runs first.
#
#   ./scripts/lumyx_guardian.sh              # all six guardians
#   ./scripts/lumyx_guardian.sh --update     # refresh the SemVer API baseline
#
# Exit 0 = no regression. Exit 1 = a guardian blocked. Exit code is the ONLY
# source of truth (KB-013).
# ══════════════════════════════════════════════════════════════════════════
set -u
cd "$(dirname "$0")/.."

RED='\033[0;31m'; GREEN='\033[0;32m'; YEL='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
FAIL=0
pass(){ echo -e "${GREEN}✅${NC} $1"; }
fail(){ echo -e "${RED}❌${NC} $1"; FAIL=$((FAIL+1)); }
hdr(){ echo -e "\n${BOLD}${YEL}$1${NC}"; }

STATE=".lumyx-guardian"
mkdir -p "$STATE"
UPDATE=0; [ "${1:-}" = "--update" ] && UPDATE=1

# ── 1. SemVer Guardian — breaking changes on seam types ───────────────────
# Snapshot the public API surface of the seam crate (led-core) and diff.
# A diff means the frozen/stable contracts changed — a human must confirm it
# is an intended SemVer bump, not an accidental break.
hdr "1 · SemVer Guardian — seam contract surface"
api_now=$(grep -rhoE '^\s*pub (fn|struct|enum|trait|const|type|mod) [A-Za-z_][A-Za-z0-9_]*' \
            crates/led-core/src/ | sed 's/^[[:space:]]*//' | sort -u)
baseline="$STATE/led-core-api.txt"
ver=$(sed -nE 's/.*LED_CORE_CONTRACT_VERSION[^"]*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' crates/led-core/src/contract_version.rs | head -1)
if [ "$UPDATE" = 1 ] || [ ! -f "$baseline" ]; then
    echo "$api_now" > "$baseline"
    echo "$ver" > "$STATE/led-core-version.txt"
    pass "SemVer: baseline gravado ($(echo "$api_now" | wc -l | tr -d ' ') itens públicos, v$ver)"
else
    if diff -q <(echo "$api_now") "$baseline" >/dev/null; then
        pass "SemVer: superfície de seam inalterada (v$ver, $(echo "$api_now" | wc -l | tr -d ' ') itens)"
    else
        old_ver=$(cat "$STATE/led-core-version.txt" 2>/dev/null || echo "?")
        if [ "$ver" != "$old_ver" ]; then
            pass "SemVer: superfície mudou COM bump de versão ($old_ver → $ver) — intencional"
            echo "$api_now" > "$baseline"; echo "$ver" > "$STATE/led-core-version.txt"
        else
            fail "SemVer: superfície de seam mudou SEM bump (v$ver) — breaking change não declarado:"
            diff <(echo "$api_now") "$baseline" | grep -E '^[<>]' | head -8
        fi
    fi
fi

# ── 2. Dependency Guardian — DAG, cycles, forbidden imports ───────────────
hdr "2 · Dependency Guardian — DAG acíclico + camadas"
# cargo build refuses dependency cycles; a metadata resolve proves acyclicity.
if cargo metadata --format-version 1 --no-deps >/dev/null 2>&1; then
    pass "Dependency: workspace resolve (sem ciclos — cargo recusaria)"
else
    fail "Dependency: cargo metadata falhou (possível ciclo)"
fi
# C1: led-protocols must not import led-pixel-engine (layer violation)
if grep -rqE 'led[_-]pixel[_-]engine' crates/led-protocols/src/ 2>/dev/null; then
    fail "Dependency: led-protocols importa led-pixel-engine (violação de camada C1)"
else
    pass "Dependency: led-protocols ⊄ led-pixel-engine (C1)"
fi
# led-core is the sink: it must import no sibling crate.
if grep -rqE 'use led_(hal|protocols|layout|pixel_engine|sequencer|audio|bridge|xlights|player|show_recorder)' crates/led-core/src/ 2>/dev/null; then
    fail "Dependency: led-core importa um crate irmão (deve ser o sink do DAG)"
else
    pass "Dependency: led-core não importa nenhum irmão (sink do DAG)"
fi

# ── 3. Replay Guardian — determinism + provenance hashes ──────────────────
hdr "3 · Replay Guardian — hashes determinísticos"
if cargo test -q -p integration-tests --test determinism_vector 2>&1 | grep -q "test result: ok"; then
    pass "Replay: vetores de determinismo batem com a plataforma de referência"
else
    fail "Replay: divergência nos vetores de determinismo"
fi
if cargo test -q -p led-show-recorder replay:: 2>&1 | grep -q "test result: ok"; then
    pass "Replay: ReplayManifest + cross-node hash íntegros"
else
    fail "Replay: testes de replay falharam"
fi

# ── 4. Performance Guardian — latency budget ──────────────────────────────
hdr "4 · Performance Guardian — orçamento de latência"
if cargo test -q -p led-hal --test bench_latency 2>&1 | grep -q "test result: ok"; then
    pass "Performance: HAL send_frame dentro do budget (p99 gated)"
else
    fail "Performance: benchmark de latência estourou o budget"
fi

# ── 5. Security Guardian — CVEs + signatures ──────────────────────────────
hdr "5 · Security Guardian — CVEs + assinaturas"
if cargo audit --version >/dev/null 2>&1; then
    audit_out=$(cargo audit 2>&1)
    if echo "$audit_out" | grep -qiE 'error\[|CRITICAL|HIGH'; then
        fail "Security: vulnerabilidade HIGH/CRITICAL encontrada"
        echo "$audit_out" | grep -iE 'CRITICAL|HIGH' | head -3
    else
        pass "Security: 0 CVEs HIGH/CRITICAL ($(echo "$audit_out" | grep -c 'Crate:' 2>/dev/null || echo 0) avisos)"
    fi
else
    echo -e "${YEL}⚠${NC}  Security: cargo-audit não instalado — pulando CVE scan"
fi
if cargo test -q -p led-show-recorder signing 2>&1 | grep -q "test result: ok"; then
    pass "Security: Ed25519 sign/verify + tamper-detection íntegros"
else
    fail "Security: testes de assinatura falharam"
fi

# ── 6. Governance Guardian — gates + debt ledger ──────────────────────────
hdr "6 · Governance Guardian — gates + ledger de débito"
if [ -f scripts/audit_gate.py ]; then
    if python3 scripts/audit_gate.py --workspace . >/dev/null 2>&1; then
        pass "Governance: debt ledger — todo TD fechado tem evidência + controle negativo"
    else
        fail "Governance: TD fechado sem substanciação (KB-012)"
    fi
else
    echo -e "${YEL}⚠${NC}  Governance: audit_gate.py ausente"
fi
# Warning-free build (C10) — the cheap structural half.
if cargo build -q --workspace 2>&1 | grep -qE '^warning:'; then
    fail "Governance: build tem warnings acionáveis (C10)"
else
    pass "Governance: build sem warnings (C10)"
fi

# ── Verdict ────────────────────────────────────────────────────────────────
echo ""
if [ "$FAIL" -eq 0 ]; then
    echo -e "${GREEN}${BOLD}✅ LUMYX-GUARDIAN: 0 regressões — liberado${NC}"
    exit 0
else
    echo -e "${RED}${BOLD}❌ LUMYX-GUARDIAN: $FAIL guardião(ões) bloquearam${NC}"
    exit 1
fi
