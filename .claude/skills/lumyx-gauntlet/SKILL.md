---
name: lumyx-gauntlet
description: Gauntlet Loop da LUMYX — verificação adversarial de um diff/PR antes de merge, com profundidade pelo risco. Argumento: PR, branch ou commit. Verifier primeiro, falsifier depois, nunca em paralelo.
---

# Gauntlet Loop

Alvo: $ARGUMENTS. Obter o diff do próprio alvo (`gh pr diff N` ou `git diff main...branch`), nunca da main atualizada.

## Profundidade
| Risco | Verifier | Falsifier | Mutações | Rondas máx. |
|---|---|---|---|---|
| N1 | leitura do diff | — | — | 1 |
| N2 | sim | só se o diff muda um gate/teste | 3 | 1 |
| N3 | sim | sim | todas as relevantes + controlo negativo | 2, depois decisão CIO |

## Passos
1. `lumyx-verifier`: critério do accept/TD vs. diff → PASS/FAIL por item, com ficheiro:linha.
2. Se FAIL do verifier: PARAR, reportar. Não chamar o falsifier sobre algo que já falhou.
3. `lumyx-falsifier` (contexto NOVO, sem o relatório do verifier): procurar falso-verde, oráculo neutralizável,
   `#[ignore]`/`cfg` que tira cobertura, timing frágil, e plataforma não coberta (x86 vs arm64).
4. Cada achado: CONFIRMADO (com mutação executada que o prova) ou HIPÓTESE. Hipótese não bloqueia sozinha.
5. Limite de rondas: ao fim de 2 rondas N3 sem PASS, o arquiteto apresenta 3 opções ao CIO
   (corrigir · simplificar/voltar atrás · aceitar com dívida registada). Não abrir ronda 3 sem «sim».

## Saída
Tabela: achado · severidade · CONFIRMADO/HIPÓTESE · prova (comando + resultado) · correção mínima.
Veredito final: PASS / FAIL / BLOCKED. Nunca PASS com NOT_MEASURED pendente no critério.
