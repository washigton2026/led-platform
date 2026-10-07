# TD-025 — O hook julga o índice: re-medição depois de o comentário D2 ter sido corrigido (R4.10)
git-hash: 24d5b91
watched: scripts/pre-commit-hook.sh sha256:2dadd96d1110053380e16324da1230cb824039d870c1da51f257c16bb73b99a7
source_files: scripts/pre-commit-hook.sh
required_test: test_pre_commit_hook
data: 2026-10-07

# PORQUE EXISTE
#
# O R4.10 (drift docs) passou ao passado o «Porquê D2» do hook: desde o R4.1 o stale compara o
# conteúdo e não usa `git log`. Só mudaram linhas de comentário (`git diff -U0` sem nenhuma
# linha não-comentário), mas o ficheiro é `watched:` do TD-025, por isso a evidência é
# regenerada. A de 2026-10-01 fica como está.

## 1. Hook atual — exit 0
macOS 14.8.9 x86_64, árvore = main 24d5b91 + o diff do R4.10. Exit lido do processo (KB-013).
`bash tests/test_pre_commit_hook.sh`

--- prova ---
```
PASS  S1 índice 19 TD / worktree 20: o gate viu 19 (exit 0)
PASS  S2 índice vermelho / worktree verde: recusado (exit 1)
PASS  S3 commit que torna a evidência stale: recusado (exit 1)
PASS  S4 commit limpo: aceite, commit contém o ficheiro, índice limpo, sem worktree órfão
cenarios: passou=4 falhou=0
test_pre_commit_hook: 4 passed; 0 failed
```
--- fim da prova ---

## 2. Controlo negativo — hook antigo `57cf21d`, exit 1
`bash tests/test_pre_commit_hook.sh <cópia do hook 57cf21d>`

```
FAIL  S1 índice 19 TD / worktree 20: o gate NÃO viu o índice (— 27 TD entries)
FAIL  S2 índice vermelho / worktree verde: deixou passar (exit 0)
PASS  S3 commit que torna a evidência stale: recusado (exit 1)
PASS  S4 commit limpo: aceite, commit contém o ficheiro, índice limpo, sem worktree órfão
cenarios: passou=2 falhou=2
test_pre_commit_hook: 2 passed; 2 failed
```

Mudança face a 2026-10-01: o S3 do hook antigo passou a PASS. Não é regressão do hook: com o
audit_gate R4.1 o stale compara o CONTEÚDO, e o hook antigo corre o gate sobre o worktree, que
já contém a alteração — o defeito D2 deixou de ser observável por este cenário. O S1 e o S2
(julgar o índice e não o worktree — o D1) continuam a reprovar o hook antigo: são eles que
discriminam hoje.
