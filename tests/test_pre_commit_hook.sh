#!/usr/bin/env bash
# Teste do hook de pre-commit: o gate tem de julgar o ÍNDICE (o que vai ser commitado),
# não o worktree. Corre o hook através de um `git commit` REAL num clone temporário — é o
# único modo de o hook receber o GIT_INDEX_FILE que o git exporta em produção.
#
#   bash tests/test_pre_commit_hook.sh                      # testa scripts/pre-commit-hook.sh
#   bash tests/test_pre_commit_hook.sh <caminho-de-um-hook> # testa outro (controlo negativo)
#
# Exit 0 só se os 4 cenários passarem. Não mexe no repositório: tudo corre num clone em
# diretório temporário, apagado no fim.

set -uo pipefail

WS="$(git rev-parse --show-toplevel)"
HOOK="$(cd "$(dirname "${1:-$WS/scripts/pre-commit-hook.sh}")" && pwd)/$(basename "${1:-$WS/scripts/pre-commit-hook.sh}")"
BASE="$(git -C "$WS" rev-parse HEAD)"
T="$(mktemp -d "${TMPDIR:-/tmp}/lumyx-hooktest.XXXXXX")"
trap 'rm -rf "$T"' EXIT

git clone -q "$WS" "$T/repo"
cd "$T/repo"
git checkout -q --detach "$BASE"
git checkout -q -b teste-hook
git config user.email hook@test.local
git config user.name hook-test
mkdir -p "$T/hooks"; cp "$HOOK" "$T/hooks/pre-commit"; chmod +x "$T/hooks/pre-commit"
git config core.hooksPath "$T/hooks"

L=docs/technical-debt-ledger.md
passou=0; falhou=0
ok()   { echo "PASS  $1"; passou=$((passou+1)); }
nok()  { echo "FAIL  $1"; falhou=$((falhou+1)); }
# Cada cenário parte do BASE, não do HEAD: com um hook defeituoso o cenário anterior pode
# ter sido COMMITADO, e partir do HEAD contaminaria os seguintes.
limpa() { git reset -q --hard "$BASE"; git clean -qfd; }

tenta_commit() {  # grava a saída do hook em $T/out e devolve o exit do git commit
    git commit -q -m "cenario" > "$T/out" 2>&1
}

# ── S1: índice com 19 TD, worktree com 20 → o hook tem de ver 19 ──────────────
python3 - "$L" <<'PY'
import sys
p=sys.argv[1]; s=open(p).read()
i=s.index("## TD-022")
open(p,"w").write(s[:i].rstrip("-\n ")+"\n")
PY
git add "$L"
git show HEAD:"$L" > "$L"                            # worktree = 20 TD, índice = 19 TD
tenta_commit; ec=$?
if grep -aq -- '— 19 TD entries' "$T/out"; then ok "S1 índice 19 TD / worktree 20: o gate viu 19 (exit $ec)"
else nok "S1 índice 19 TD / worktree 20: o gate NÃO viu o índice ($(grep -ao -- '— [0-9]* TD entries' "$T/out" | head -1))"; fi
limpa

# ── S2: índice VERMELHO, worktree verde → commit recusado ─────────────────────
sed -i.bak 's#^evidence_ref: docs/evidence/td-020-reverificacao-2026-09-26.md#evidence_ref: docs/evidence/nao-existe.md#' "$L"; rm -f "$L.bak"
git add "$L"
git show HEAD:"$L" > "$L"                            # worktree verde, índice vermelho
tenta_commit; ec=$?
if [ $ec -ne 0 ] && grep -aq 'CRITICAL' "$T/out"; then ok "S2 índice vermelho / worktree verde: recusado (exit $ec)"
else nok "S2 índice vermelho / worktree verde: deixou passar (exit $ec)"; fi
limpa

# ── S3: o commit que torna uma evidência stale → recusado ─────────────────────
F=crates/led-hal/src/shared_clock.rs
printf '\n// teste do hook: alteração que torna a evidência do TD-020 stale\n' >> "$F"
git add "$F"
tenta_commit; ec=$?
if [ $ec -ne 0 ] && grep -aq 'stale' "$T/out"; then ok "S3 commit que torna a evidência stale: recusado (exit $ec)"
else nok "S3 commit que torna a evidência stale: deixou passar (exit $ec)"; fi
limpa

# ── S4: commit limpo → aceite, e o índice/commit ficam intactos ───────────────
echo "teste do hook" > docs/_teste_hook.md
git add docs/_teste_hook.md
tenta_commit; ec=$?
if [ $ec -eq 0 ] && git show --name-only --format= HEAD | grep -qx 'docs/_teste_hook.md' \
   && [ -z "$(git status --porcelain)" ] && [ -z "$(git worktree list --porcelain | grep -c lumyx-precommit | grep -v '^0$')" ]; then
    ok "S4 commit limpo: aceite, commit contém o ficheiro, índice limpo, sem worktree órfão"
else nok "S4 commit limpo: exit $ec / conteúdo ou índice errado"; cat "$T/out"; fi

echo "cenarios: passou=$passou falhou=$falhou"
[ $falhou -eq 0 ] && [ $passou -eq 4 ]
