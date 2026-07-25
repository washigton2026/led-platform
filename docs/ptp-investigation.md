# PTP — investigação (missão: "PTP investigado")

## Pergunta

Multi-node shows precisam de relógios alinhados. Qual mecanismo, com que
precisão, e o que o LUMYX implementa hoje?

## Opções avaliadas

| Mecanismo | Precisão típica | Requisitos | Veredicto |
|---|---|---|---|
| **PTP (IEEE 1588) c/ HW timestamping** | ±1 µs | NIC com timestamping + switches PTP-aware (boundary/transparent clock) | Ideal, mas o hardware do rig (ESP32/WLED, switch doméstico) **não suporta** |
| **PTP por software (ptp4l s/ HW)** | ±10–100 µs | Linux com ptp4l; daemon externo | Bom, mas adiciona dependência de sistema fora do controle da plataforma |
| **NTP (chrony/systemd)** | ±0,5–5 ms LAN | daemon do SO | Suficiente, porém não observável de dentro do show |
| **LUMYX `net_time` (two-way UDP)** | ±1 ms LAN cabeada (medido nos testes) | nada além do próprio binário | **Implementado** — `led-hal/src/net_time.rs` |

## O que foi implementado

`led_hal::net_time` — two-way time transfer (a mesma matemática do NTP e do
delay request-response do PTP, sem timestamping de hardware):

- `TimeServer` no líder (management plane, nunca no hot path)
- `measure_offset` → `TimeSample { offset_ms, delay_ms }`
- `best_of(n)` — gating por delay (a troca mais rápida tem menos ruído de fila)
- `sync_to` — calibra o `SharedClock` do follower; o líder nunca se ajusta
- 5 testes: offset ≈ 0 em loopback, offset injetado ±500/−300 ms detectado a
  ±10 ms, pós-sync dentro do budget de drift (5 ms) do `SyncedCluster`,
  robustez a pacotes malformados

## Conclusão

Para o alvo do LUMYX (drift tolerance 5 ms no `SyncedCluster`, frames de 25–50
ms), **±1 ms por software é suficiente** e mantém a plataforma std-only e
auto-contida. PTP com hardware vira requisito apenas se um futuro rig exigir
sincronia sub-milissegundo (ex.: vídeo genlock + LED no mesmo palco). Nesse
caso: NICs Intel i210/i225 + switch transparent-clock + ptp4l, e o LUMYX
consome o relógio do SO já disciplinado — nenhuma mudança de arquitetura,
apenas trocar a fonte do `SharedClock` do líder.

**Recomendação de rede do rig** (independente de PTP): backbone cabeado,
IGMP snooping se multicast sACN, e o `net_time` re-sincronizando a cada
troca de música (não por frame).
