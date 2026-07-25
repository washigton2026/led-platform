# Hardware Validation Report — bancada 1 controlador (robô led 1)

- **Data:** 2026-07-20
- **Validador:** LUMYX Live Hardware Validator (uma etapa por vez, só avança com evidência observada)
- **Marco:** primeira luz física do LUMYX — o pipeline `led-player → DDP → WLED → LED real`
  provado em hardware, encerrando a barreira que estava aberta desde o início (rig offline).

## Hardware sob teste

| Item | Valor |
|---|---|
| Controlador | ESP32, WLED **16.0.1** (arch esp32, core 4.4.8) |
| Fita | WS2812B, **720 px físicos**, Color Order **GRB** |
| Alimentação | bateria 12 V 14 Ah → DC/DC TOBSUN 50 W **5 V/10 A** |
| Dado | **GPIO2** + resistor 330 Ω em série + cap 1000 µF |
| Proteção | ABL (Max PSU Current) **850 mA** ligado |
| IP | **192.168.2.156** (= robô led 1 do rig) |
| Link | WiFi (bancada), RSSI −37…−44 dBm |

## Resultados por etapa

| # | Etapa | Resultado | Evidência observada |
|---|---|---|---|
| 1 | Power | ✅ PASS | fita acende; ESP32 boota; nada aquece/queima (foto) |
| 2 | Energização | ✅ PASS | LEDs do ESP32 acesos; uptime contínuo |
| 3 | WLED | ✅ PASS | 16.0.1; GPIO2; GRB; 720 px; IP .156 (LED Settings) |
| 4 | Cores nativas | ✅ PASS | R→vermelho, G→verde, B→azul (operador confirmou os 3) |
| 5 | Walk test | ✅ PASS | cometa varreu ponta-a-ponta; **sem pixel morto reportado** |
| 6 | Network | ✅ PASS ⚠ | ping 0 % perda; **99 ms avg / 146 ms pico / jitter 31 ms** com sinal forte → power-save do WiFi |
| 7 | **DDP (LUMYX)** | ✅ **PASS** | `led-player striptest.lumyx --ddp 192.168.2.156` → **94/94 frames, 0 falhas**, hash `0x23b8ee876a18e5a5`; WLED `live:true` de 192.168.2.32; visual (R→G→B→cometa) confirmado |
| 8 | Metrics | ✅ PASS | `/metrics`: frames_total 1→222, **0 drops**, p50 128 µs, p99 8.2 ms |
| 9 | Mini burn-in | ✅ PASS | **74/74 passes, 0 falhas, 0 aborts**, 1 hash; pós-burn: uptime 2388→3296 s (sem reset), freeheap 117228 (sem leak) |
| 10 | Certificação | ✅ este documento | |

## Conclusão

**VALIDADO** — em **1 controlador (robô led 1, 720 px, WiFi de bancada)**. O software LUMYX
aciona LED físico real, de ponta a ponta, com replay determinístico estável por ~10 min
contínuos contra hardware.

## O que este relatório valida — e o que NÃO valida (honestidade)

**Valida (observado em metal):**
- Fluxo `software → fio → pixel real` — a barreira histórica.
- Determinismo do replay: **77 passes** (3 + 74) do mesmo striptest, hash idêntico em todos.
- Estabilidade do `led-player` e do ESP32/WLED por ~10 min contínuos (sem crash, reset ou leak).
- Latência de envio do LUMYX folgada (p50 128 µs, 0 drops).

**NÃO valida — permanece ⚠ NÃO VALIDADO EM HARDWARE:**
- **O rig completo**: só 1 dos 5 controladores, 720 de 6.200 px. Multi-controlador não testado.
- **O caminho Ethernet cabeado**: a bancada foi **WiFi**, e o WiFi mostrou 99 ms de jitter —
  reforça (não substitui) a migração para Ethernet antes de qualquer show ao vivo
  (ver [ADR-0005](../adr/0005-wifi-proibido-producao.md) e o
  [runbook de migração](../runbooks/wifi-to-ethernet-migration.md)).
- **Um show musical real**: isto foi um `striptest.lumyx` sintético, não `robot_sequence.lumyx`.
- **Burn-in longo (72 h)** e **chaos físico** (puxar cabo com o show rodando).

## Ressalvas técnicas registradas

- **Latência ≠ entrega**: os 128 µs/8,2 ms do `/metrics` são o tempo interno do `led-player`
  (DDP é fire-and-forget, sem ACK). O atraso **real** que a fita sente é o ping de ~99 ms —
  medição downstream, invisível aos contadores do player.
- **Config vs físico**: `info.leds.count` = 1560, mas a fita real são 720 px (output 1,
  Start 0). O DDP de 720 px acerta a fita; os 1560 são pixels-fantasma de config.
- **GPIO2 (strapping pin)**: bootou e rodou 10 min sem problema — o risco não se materializou.
- **Nível de dado 3,3 V**: cometa correu limpo, sem flicker → nível OK na prática nesta fita.

## Próximos passos sugeridos (NÃO executados)

1. Repetir em **Ethernet** (ex.: Olimex ESP32-POE) e re-medir o ping — expectativa: de ~99 ms
   para <1 ms, fechando a ressalva da ETAPA 6.
2. Tocar um slice real de `robot_sequence.lumyx` (720 px) no nó físico.
3. Ligar o **2º controlador** e validar 2 nós (base do multi-controlador do rig).

## Artefatos

- Gerador do teste: `crates/led-show-recorder/examples/make_striptest.rs`
- Show de teste: `striptest.lumyx` (720 px, 94 frames, `0x23b8ee876a18e5a5`)
- Log do burn-in: 74 passes, arquivo de saída da sessão (exit 0)
