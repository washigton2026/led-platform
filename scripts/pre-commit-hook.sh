#!/bin/bash
# LUMYX pre-commit hook — KB-012 debt gate (FALHA FECHADO)
#
# Runs scripts/audit_gate.py before every commit.
# Exit != 0 aborts the commit.
# FAIL-CLOSED: if the gate itself is missing or crashes → abort.
# Never lets a commit through due to infrastructure failure.
#
# O gate corre sobre o COMMIT CANDIDATO (o índice), nunca sobre o worktree.
#
# Porquê (2026-09-26, dois defeitos medidos):
#   D1 — o gate lia o ledger e a evidência do WORKTREE. Um índice com 19 TD e um worktree
#        com 20 dava «20 OK»: validava um estado que não ia ser commitado.
#   D2 — o detector de evidência stale corre `git log <hash>..HEAD`, e o HEAD ainda NÃO
#        contém as alterações em stage. O commit que torna uma evidência stale passava
#        sempre (foi assim que o C4, 320ff94, deixou o TD-020 stale na main).
#
# Correcção: o índice vira um commit candidato (`write-tree` + `commit-tree`, filho do HEAD),
# que é materializado num worktree temporário; o gate corre lá, com HEAD = candidato. O
# commit candidato é um objecto solto, nunca referenciado — o `gc` apaga-o.
#
# Dentro de um hook o git exporta GIT_INDEX_FILE (e pode exportar GIT_DIR/GIT_WORK_TREE).
# Herdados, fariam o `worktree add` escrever no índice do commit em curso e o gate ler o
# HEAD errado. São limpos antes de tocar no worktree temporário.

set -euo pipefail

WORKSPACE="$(git rev-parse --show-toplevel)"

echo "⚙️  [pre-commit] Running LUMYX debt gate (KB-012) sobre o índice..."

# ── O commit candidato: exactamente o que vai ser commitado ──────────────────
TREE="$(git write-tree)"
if git rev-parse -q --verify HEAD >/dev/null; then
    CANDIDATO="$(git commit-tree "$TREE" -p HEAD -m 'lumyx pre-commit: candidato')"
else
    CANDIDATO="$(git commit-tree "$TREE" -m 'lumyx pre-commit: candidato')"
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/lumyx-precommit.XXXXXX")"
limpa() {
    env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE \
        git -C "$WORKSPACE" worktree remove --force "$TMP/wt" >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap limpa EXIT

env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE \
    git -C "$WORKSPACE" -c core.hooksPath=/dev/null worktree add --detach --quiet "$TMP/wt" "$CANDIDATO"

GATE="$TMP/wt/scripts/audit_gate.py"

# ── FAIL CLOSED: gate must exist (no commit candidato) ───────────────────────
if [ ! -f "$GATE" ]; then
    echo "❌ [pre-commit] ABORT: scripts/audit_gate.py not found."
    echo "   Gate missing = fail closed. Restore the file before committing."
    exit 1
fi

# ── Run the gate ──────────────────────────────────────────────────────────────
gate_exit=0
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE PYTHONDONTWRITEBYTECODE=1 \
    python3 "$GATE" --workspace "$TMP/wt" || gate_exit=$?

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
