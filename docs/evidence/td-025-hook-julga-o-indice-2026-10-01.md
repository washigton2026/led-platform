# TD-025 — O hook de pre-commit julga o índice: medição sobre `7ee2f89`
git-hash: 7ee2f89
watched: scripts/pre-commit-hook.sh sha256:f89f1fa9f17ddd305ac23726e44848b3b6a3ae8d98605f44dc058555126a3de9
source_files: scripts/pre-commit-hook.sh
required_test: test_pre_commit_hook
data: 2026-10-01

# PORQUE EXISTE
#
# A medição de 2026-09-29 (hook de origin/main: S1-S4 PASS) não estava num formato que o
# debt gate aceite: o teste só imprimia «cenarios: passou=N falhou=M». O commit `7ee2f89`
# acrescenta a linha «test_pre_commit_hook: N passed; M failed», calculada dos mesmos
# contadores. Este artefacto re-mede os dois hooks sobre esse commit.

## Ambiente e comandos

macOS 14.8.9 x86_64. HEAD `7ee2f894f435841815d3e15b1aad579d350a3579`. Exit lido do processo,
sem pipe (KB-013). O teste corre o hook através de um `git commit` real num clone temporário.

| ficheiro | sha256 |
|---|---|
| `scripts/pre-commit-hook.sh` (hook actual, inalterado desde `eb791fe`) | `f89f1fa9f17ddd305ac23726e44848b3b6a3ae8d98605f44dc058555126a3de9` |
| `scripts/pre-commit-hook.sh` em `57cf21d` (hook antigo) | `a4113c356282f2e07be5070af807cdf1efacbffccfef16bc31c7f5d4e39cf465` |

```
bash tests/test_pre_commit_hook.sh                          # hook actual
bash tests/test_pre_commit_hook.sh <cópia do hook 57cf21d>  # controlo negativo
```

## 1. Hook actual — exit 0

```
PASS  S1 índice 19 TD / worktree 20: o gate viu 19 (exit 0)
PASS  S2 índice vermelho / worktree verde: recusado (exit 1)
PASS  S3 commit que torna a evidência stale: recusado (exit 1)
PASS  S4 commit limpo: aceite, commit contém o ficheiro, índice limpo, sem worktree órfão
cenarios: passou=4 falhou=0
test_pre_commit_hook: 4 passed; 0 failed
```

## 2. Controlo negativo — hook antigo `57cf21d`, exit 1

```
FAIL  S1 índice 19 TD / worktree 20: o gate NÃO viu o índice (— 26 TD entries)
FAIL  S2 índice vermelho / worktree verde: deixou passar (exit 0)
FAIL  S3 commit que torna a evidência stale: deixou passar (exit 0)
PASS  S4 commit limpo: aceite, commit contém o ficheiro, índice limpo, sem worktree órfão
cenarios: passou=1 falhou=3
test_pre_commit_hook: 1 passed; 3 failed
```

O hook antigo lê o ledger do worktree (vê as 26 TD em vez das 19 do índice), deixa passar
um índice vermelho e não vê o commit que torna uma evidência stale. Só o S4 passa, e é o
cenário que um hook defeituoso também satisfaz.

## 3. O gate recusa a evidência do controlo negativo

Num clone temporário em `7ee2f89`, com o TD-025 marcado `closed` e um `evidence_ref` que
contém **só** a saída da secção 2 (`git-hash: 7ee2f89`, `required_test` presente):

```
python3 scripts/audit_gate.py   → exit 1
  🔴 [CRITICAL] TD-025: evidence_ref contains no 'N passed; 0 failed' line at all. Expected 'test result: ok. N passed' (N≥1). Re-run the verification and commit the output.
Result: 1 Critical, 0 Warning, 13 OK · 10 open · 2 wontfix
Gate FAILED — fix Critical findings before closing TDs.
```

A linha «1 passed; 3 failed» não casa com o regex do gate (`scripts/audit_gate.py:84`,
`N passed; 0 failed`): uma execução com falhas não fecha o TD.

Cópias brutas: `~/lumyx-evidence/2026-10-01/0d/` (`atual-7ee2f89.txt`, `antigo-7ee2f89.txt`,
`gate-recusa-neg.txt`, `sha256.txt`).
