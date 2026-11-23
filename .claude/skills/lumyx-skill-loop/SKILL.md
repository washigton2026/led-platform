---
name: lumyx-skill-loop
description: Skill Loop da LUMYX — no fim de um ciclo, transforma um erro ou passo manual REPETIDO (≥2 vezes) numa regra de skill, gate ou script. Não usar para casos únicos.
---

# Skill Loop

1. Listar o que neste ciclo custou retrabalho: erro repetido, comando manual repetido, pergunta repetida ao operador.
2. Filtrar: só entra o que aconteceu ≥2 vezes (citar as duas ocorrências: commit, run ou sessão).
3. Escolher o sítio mais barato que impede a repetição, por esta ordem:
   permissão em settings.json → gate/teste em CI → script em scripts/ → linha numa SKILL.md → KB no knowledge-base.md.
4. Propor o diff (máx. 10 linhas por regra). Não reescrever a skill inteira.
5. Pedir «sim» antes de gravar. Uma regra nova por ciclo, no máximo.
