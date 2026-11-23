---
name: lumyx-ciclo
description: Engineering Loop da LUMYX. Usar para avançar UM passo do caminho crítico (ROADMAP PARTE VI) até PR em rascunho com evidência. Argumento opcional: o passo (ex. "M0 TD-021", "M3 3.1"). Defere a lumyx-cio-engineering.
---

# Engineering Loop — um passo, do início ao PR em rascunho

Argumento: $ARGUMENTS (se vazio, escolhe o próximo passo do caminho crítico).

## 1. Início (máx. 5 leituras)
- `git status`, branch, `git log -1`, `git fetch` e quanto está atrás/à frente da main.
- Ler SÓ: ROADMAP PARTE VI (marco ativo), a entrada do TD no ledger se houver, o ADR do passo.
- Procurar trabalho já feito: `git log --all --grep=<passo>`, PRs abertos, `docs/evidence/`.
  Se já existe, CONTINUAR a partir dele; não repetir.
- Declarar o ambiente (Mac/cloud) e o que não está ao alcance.

## 2. Classificar
- Risco: N1 (doc) · N2 (testes/CI/scripts) · N3 (concorrência, IPC, protocolo, segurança, player, hardware).
- Accept: N3 exige accept formal com sha. Se o código já está à frente do accept → PARAR e reportar.
- Escrever o CRITÉRIO DE CONCLUSÃO em uma frase verificável antes de tocar em código.

## 3. Construir (delegar)
- N1: fazer direto.
- N2/N3: agente `lumyx-implementador` com: objetivo, ficheiros permitidos, critério, testes a criar.
- Paralelo só em worktrees separadas e sem ficheiros em comum (ex.: M3 enquanto M0).

## 4. Provar
- `cargo test --workspace --locked` (exit lido sem pipe), `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `python3 scripts/audit_gate.py`; `./scripts/tsc_gate.sh` se tocou no console.
- Teste negativo obrigatório em N2/N3: a mutação que o teste tem de apanhar, executada.
- N2/N3 → correr `/lumyx-gauntlet` antes do PR.

## 5. Entregar
- Branch `tipo/passo-curto`, `git add <ficheiros>` (nunca -A), um commit por passo lógico.
- PR em RASCUNHO só se o operador autorizou push neste ciclo; senão, parar com o diff pronto.
- Atualizar no mesmo PR o que o passo torna falso (ledger, CLAUDE.md, ROADMAP) — sem documento novo.

## 6. Relatório (formato fixo, curto)
ESTADO · FACT · INFERENCE · FEITO · PENDENTE · BLOQUEIO · DECISÃO CIO · PRÓXIMA AÇÃO (uma, com critério).
Terminar com 3 linhas de handoff: commit, branch, evidência (caminho/run).
