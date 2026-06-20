#!/bin/bash
# LUMYX pre-commit hook — KB-012 debt gate (FALHA FECHADO)
#
# Runs scripts/audit_gate.py before every commit.
# Exit != 0 aborts the commit.
# FAIL-CLOSED: if the gate itself is missing or crashes → abort.
# Never lets a commit through due to infrastructure failure.

set -euo pipefail

WORKSPACE="$(git rev-parse --show-toplevel)"
GATE="$WORKSPACE/scripts/audit_gate.py"

# ── FAIL CLOSED: gate must exist ─────────────────────────────────────────────
if [ ! -f "$GATE" ]; then
    echo "❌ [pre-commit] ABORT: scripts/audit_gate.py not found."
    echo "   Gate missing = fail closed. Restore the file before committing."
    exit 1
fi

# ── Run the gate ──────────────────────────────────────────────────────────────
echo "⚙️  [pre-commit] Running LUMYX debt gate (KB-012)..."
gate_exit=0
python3 "$GATE" --workspace "$WORKSPACE" || gate_exit=$?

# ── FAIL CLOSED: any non-zero exit (including crashes) aborts ─────────────────
if [ $gate_exit -ne 0 ]; then
    echo ""
    echo "❌ [pre-commit] COMMIT ABORTED — debt gate failed (exit $gate_exit)."
    echo "   Fix Critical findings in docs/technical-debt-ledger.md before committing."
    echo "   To bypass (emergency only): git commit --no-verify"
    exit 1
fi

echo "✅ [pre-commit] Debt gate passed — commit allowed."
exit 0
