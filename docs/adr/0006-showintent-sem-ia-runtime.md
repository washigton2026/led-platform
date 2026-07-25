# ADR-0006 — ShowIntent: IA só em design-time, nunca em runtime

- **Status:** aceito
- **Data original:** 2026-06-28 (`ShowIntentGenerator`); regra no GOSL desde a
  fundação
- **Fonte:** LUMYX_GOSL.md ("AI is deterministic / design-time only");
  CLAUDE.md changelog 2026-06-28

## Contexto e problema
Geração de show por IA é atraente, mas um LLM no caminho de runtime é
**não-determinístico** (mesmo áudio pode gerar shows diferentes), **imprevisível
em latência** e **impossível de reproduzir** — o que colide de frente com o
ADR-0001 (replay determinístico). Um show ao vivo não pode depender de uma
chamada de rede a um modelo que pode variar, atrasar ou falhar.

## Decisão
A IA opera **apenas em design-time**, e nunca emite pacote de protocolo:
- O LLM (quando usado) produz somente um `ShowIntent` — um schema estrito e
  validado (`energy[0,1]`, `bpm[20,300]`, `duration>0`, `pixels>0`) com
  `intent_hash` (FNV-1a).
- Em runtime, um gerador Rust **semeado e determinístico**
  (`ShowIntentGenerator::from_audio`, PRNG SplitMix64) transforma o intent +
  áudio no timeline. Mesmo áudio + mesmo intent ⇒ mesmo show, sempre.
- A IA **nunca** emite waypoints/pixels diretamente; o `DroneBridge` produz só
  *hints*, nunca trajetórias (invariante lumyx-ai-governor).

## Consequências
**Boas:** shows são reproduzíveis e auditáveis (o `intent_hash` entra na
Provenance); nenhuma dependência de rede/modelo no palco; latência previsível.
A fronteira "IA propõe intent, código executa" é testável e falsificável.
**Ruins/custos:** menos "mágica" em runtime — a criatividade da IA fica confinada
ao momento de composição, não à execução. O gerador determinístico é
rule-based/PRNG, não aprende em tempo real (por design).

## Alternativas rejeitadas
- **LLM gerando pixels/timeline em runtime** — não-determinístico, não
  reproduzível, latência imprevisível; incompatível com ADR-0001 e com show ao
  vivo.
- **IA emitindo pacotes de protocolo diretamente** — viola a fronteira de
  segurança (IA nunca toca a rede) e o determinismo.
- **Sem IA nenhuma** — desnecessariamente restritivo; a IA agrega valor real na
  composição de design-time, onde não-determinismo é aceitável.
