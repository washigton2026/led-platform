# ADR-0017 — Blackout intencional × invariante do heartbeat (ADIADO)

- **Status:** 🔴 **proposto (adiado)** — problema registrado; **decisão pendente**. Nenhum
  botão, atalho ou API de blackout pode entrar antes de este ADR ser aceito.
- **Data original:** 2026-07-26
- **Fonte:** Revisão do plano de console do operador (regra: blackout requer decisão separada)

## Contexto e problema
O console do operador vai precisar, eventualmente, de um **blackout intencional** (apagar o
rig sob comando). Mas o Baseline 1.0 tem um invariante deliberado e testado: **o heartbeat
NUNCA envia um frame preto/zerado** — porque um frame zero apaga o palco por acidente e o
silêncio total dispara safe-mode nos controladores (`crates/led-hal/src/heartbeat.rs`; teste
`must not blast a blackout frame`). Existe, portanto, uma **tensão real**: "operador manda
apagar" precisa coexistir com "o sistema jamais apaga sozinho".

O problema central não resolvido: **quando o operador aciona blackout, o que o heartbeat
reenvia?** Se o heartbeat gravar o frame preto como "último frame válido", ele reenvia preto
(blackout persistente — correto para blackout). Se não gravar, ele reenvia o frame
pré-blackout (o rig "acende de volta" no próximo heartbeat — errado). Cada opção tem
implicações de segurança de palco.

## Decisão
**PENDENTE.** Este ADR existe para **impedir** que um blackout seja implementado ad-hoc antes
de a semântica ser decidida. Nenhuma decisão é tomada aqui ainda.

## Questões a resolver antes de aceitar
1. Blackout emite um **frame preto real** (não silêncio) — confirmar que isso NÃO é o
   mesmo caminho que o invariante proíbe (o invariante proíbe blackout *acidental/por
   silêncio*, não um preto *comandado*).
2. Interação com `heartbeat.record()`: o blackout comandado deve ou não virar o "último
   frame válido" reenviado?
3. Privilégio e confirmação: blackout é ação irreversível de palco → auth + confirmação +
   log auditável (ADR-0014).
4. Restauração: `restore` volta ao último frame não-preto? Como é rastreado?
5. Atalho de teclado (`B`?) só após confirmar não-conflito com foco de texto, atalhos de
   sistema e acessibilidade.

## Não-escopo
Este ADR **não** decide a UI, o IPC nem o preview (ADRs 0013–0016). É estritamente sobre a
semântica blackout × heartbeat.

## Consequências
Enquanto adiado: **nenhum blackout na UI**. O plano de PRs de console exclui explicitamente
botão/atalho de blackout até este ADR ser aceito.

## Critério de reversão
N/A (ainda não há decisão para reverter). Ao ser aceito, ganhará os campos padrão
(decisão, alternativas, consequências, critério de reversão).
