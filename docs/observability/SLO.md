# LUMYX — SLOs formais e error budgets

Válido para shows ao vivo em rede cabeada (Hardware Rule: WiFi proibido).
Fonte de dados: `led_hal::prometheus` (`GET /metrics`), scrape 15s.

## SLOs

| SLO | Alvo | Janela | Métrica (PromQL) | Error budget |
|---|---|---|---|---|
| **Entrega de frames** | ≥ 99,9% dos frames chegam a ≥1 segmento | show (rolling 1h) | `1 - rate(lumyx_drops_total[5m]) / rate(lumyx_frames_total[5m])` | 0,1% ≈ 144 frames/h a 40fps |
| **Latência de envio p99** | < 5 ms (release) | rolling 5m | `lumyx_frame_latency_seconds{quantile="0.99"}` | 5 min acumulados/show acima do alvo |
| **Gap de heartbeat** | nunca > 2,4 s (GOSL CRIT) | sempre | alerta `AlertCondition::HeartbeatGapMs` | **zero** — violação = incidente |
| **Disponibilidade do cluster** | ≥ 1 segmento ativo 100% do show | show | `SyncedCluster::active_segment_count() > 0` | zero |

## Política de error budget

- Budget de entrega esgotado na janela → **congela mudanças** (sem novo efeito,
  firmware ou config durante o evento) até a causa raiz ser registrada no ledger.
- Violação de heartbeat/cluster (budget zero) → post-mortem obrigatório com
  entrada no `docs/technical-debt-ledger.md` e teste de regressão.
- Os alertas `AlertEngine::lumyx_standard()` espelham estes SLOs em processo
  (P99ExceedsUs, DropRatePct, HeartbeatGapMs) — o Prometheus é a visão externa.

## Queries de burn rate (multi-window)

```promql
# fast burn (5m/1h): páginas
(rate(lumyx_drops_total[5m]) / rate(lumyx_frames_total[5m])) > 14.4 * 0.001
# slow burn (30m/6h): ticket
(rate(lumyx_drops_total[30m]) / rate(lumyx_frames_total[30m])) > 6 * 0.001
```
