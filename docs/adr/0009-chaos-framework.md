# ADR-0009 — Chaos framework determinístico com baseline

- **Status:** aceito
- **Data original:** 2026-06-28 (`ChaosHarness` in-process); estendido
  2026-07-09 (`UdpChaosProxy` no fio)
- **Fonte:** CLAUDE.md changelog 2026-06-28 e 2026-07-09

## Contexto e problema
Resiliência não pode ser alegada — tem de ser exercida. Mas injeção de falha
"aleatória" produz testes flaky e não-reproduzíveis: um teste que às vezes passa
e às vezes falha não prova nada e erode a confiança na suíte. Precisamos
injetar perda de pacote, latência e crash de forma que seja **reproduzível** e
que sempre tenha um **estado de comparação** (baseline).

## Decisão
Um chaos framework com duas propriedades inegociáveis:
1. **Determinístico por seed** — PRNG SplitMix64 (o mesmo do
   `ShowIntentGenerator`, ADR-0006): mesmo seed ⇒ mesmo padrão de drops. Um
   experimento é 100% reproduzível.
2. **Todo experimento tem um baseline** — o estado antes da injeção de falha.
Dois níveis: `ChaosHarness<P: ProtocolOutput>` (in-process, intercepta
`send_frame`) e `UdpChaosProxy` (no fio, dropa datagramas reais entre sockets —
o equivalente de CI a puxar o cabo). Ambos: `FaultConfig` (packet_loss_pct,
latency_us, crash_after_frames, seed), enable/disable dinâmico.

## Consequências
**Boas:** testes de resiliência reproduzíveis e não-flaky (mesmo seed = mesmos
drops, verificado); recuperação faz parte do experimento (outage → heal → 100%);
`ChaosHarness` é `#[cfg(test)]`/feature-gated — **nunca** em produção. Cobre
30% loss degrada sem parar o stream, outage total, latência observável.
**Ruins/custos:** o chaos é sintético — cobre perda/latência/crash, mas
**ainda não** reorder/duplicação de pacote nem frame rasgado no fio (GAP
rastreado em RT-004). Chaos físico literal (puxar cabo) exige hardware.

## Alternativas rejeitadas
- **Injeção aleatória (`rand`)** — testes flaky, não-reproduzíveis; contradiz o
  valor inteiro de um chaos test.
- **Só teste in-process** — não pega o comportamento real de socket/UDP; por
  isso o `UdpChaosProxy` com datagramas reais foi adicionado.
- **Chaos em produção (à la Netflix)** — inadequado e perigoso para um show ao
  vivo de LED; o chaos é de teste/CI, gated para nunca rodar em produção.
