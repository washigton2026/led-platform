# ADR-0003 — DDP como protocolo de saída preferencial

- **Status:** aceito
- **Data original:** 2026-06-26 (implementação DDP); confirmado 2026-07-09
  (DDP no player) e em `docs/capacity.md`
- **Fonte:** CLAUDE.md changelog 2026-06-26; docs/capacity.md

## Contexto e problema
O rig real do usuário são controladores WLED (ESP32). Cada universo de
sACN/ArtNet carrega no máximo 170 pixels (510 canais / 3). Para um rig denso, o
número de pacotes UDP por frame cresce rápido e a rede — especialmente WiFi
(ver ADR-0005) — vira o gargalo antes de qualquer coisa no motor de render.

## Decisão
Suportar DDP e tratá-lo como o **caminho de capacidade preferencial** para
alvos WLED de controlador único:
- `DDP_MAX_PIXELS = 487` (487×3 = 1461 bytes ≤ 1462 do limite de MTU), com
  auto-fragmentação e sequência por-device.
- `DdpOutput` no player é pixel-nativo (sem mapeamento de universo).
- Ganho medido: **487 px/pacote vs 170 do ArtNet ≈ 3× menos pacotes** para o
  mesmo rig. `led-player --ddp` é o caminho recomendado para WLED.

## Consequências
**Boas:** ~3× menos pacotes por frame → mais folga de rede (crítico enquanto o
rig estiver em WiFi); WLED aceita DDP nativamente; endereçamento por byte-offset
é mais simples que universos para um controlador único. `docs/capacity.md`
mostra que, com DDP + Ethernet, 12–15k px por robô cabem no orçamento de fio.
**Ruins/custos:** DDP é ponto-a-ponto (não multicast) — para muitos
controladores, o `RouterDevice` roteia por universo, mas o DDP puro bypassa o
HAL (sem mapa de universos). sACN/ArtNet permanecem para interoperabilidade e
para rigs multi-controlador com necessidade de multicast.

## Alternativas rejeitadas
- **Só sACN/ArtNet** — teto de 170 px/universo multiplica pacotes; ArtNet foi
  mantido (o rig está configurado assim hoje) mas não é o caminho de capacidade.
- **DDP multicast** — DDP é unicast por natureza; multicast fica com sACN.
- **Protocolo proprietário** — quebra interoperabilidade com WLED/FPP/Falcon,
  que é um objetivo do projeto.
