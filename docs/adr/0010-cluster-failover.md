# ADR-0010 — Failover de cluster por saúde de segmento

- **Status:** aceito
- **Data original:** 2026-06-28 (`SyncedCluster`)
- **Fonte:** CLAUDE.md changelog 2026-06-28; cluster_sync.rs

## Contexto e problema
Um rig grande é dividido em segmentos, cada um atrás de um controlador. Se um
segmento falha no meio do show (controlador reinicia, cabo cai), o show **não
pode parar** — a regra "never stop sending" (heartbeat nunca zera, ADR-0005) se
estende ao cluster. Mas também não se pode tratar uma falha transitória como
morte permanente, nem continuar martelando um segmento morto para sempre.

## Decisão
`SyncedCluster` com estado de saúde por segmento (`SegmentHealth`) e limiares:
- **Healthy** → **Degraded** após **3 falhas** consecutivas → **Failed** após
  **10 falhas** consecutivas (excluído dos envios até `rejoin_segment`).
- Envio parcial é melhor que nenhum: um frame que chega a ≥1 segmento retorna
  `Ok`; só falha quando **todos** falham.
- `hot_join` adiciona segmento a um cluster rodando; `rejoin_segment`
  reintegra um Failed recuperado externamente; cache do último frame válido
  para reenvio de heartbeat.
- Timestamps alinhados por `SharedClock`; drift detectado mas não bloqueia
  (pode ser primeiro frame ou calibração em curso).

## Consequências
**Boas:** o cluster sobrevive à morte de um nó (provado:
`failover_continues_when_one_node_fails`, `two_node_cluster` 6/6); recuperação
via hot-join/rejoin; degradação graciosa sob perda (cluster + chaos 30%).
Histerese (3 vs 10) evita marcar Failed por soluço transitório.
**Ruins/custos:** o `send_frame` toma um `write()` lock por frame para atualizar
a saúde — **contenção que escala mal a muitos segmentos** (achado da revisão
CIO; candidato a health lock-free por atômicos, mas prematuro enquanto houver
1 nó físico). O sync é **LAN-only** (`net_time` two-way); multi-site/WAN não tem
história de relógio ainda.

## Alternativas rejeitadas
- **Falhar o show inteiro se um segmento cai** — viola "never stop sending"; um
  robô morto não pode apagar os outros quatro.
- **Marcar Failed na primeira falha** — trata soluço transitório como morte;
  a histerese 3/10 é o meio-termo.
- **Lock-free desde o início** — micro-otimização de um caminho que hoje roda
  com um elemento; adiado até existir um segundo nó físico (evita
  overengineering — ver revisão CIO 2026-07-12).
