# LUMYX — Relatório de capacidade (2026-07-09)

**Pergunta do produto:** o xLights + 5× WLED ESP32 (WiFi) limita o rig de robôs
a ~6.200 px. Quanto o LUMYX aguenta?

## Medição

Pipeline completo por frame (Plasma render → LogicalFrame → mapeamento Hal →
fan-out DeviceDriver → controladores simulados), 100 frames, **release**,
MacBook Air (M-series). `cargo run --release -p led-demo --example capacity_bench`.

| Pixels | Controladores (28 universos) | avg/frame | max/frame | 40 fps (25 ms)? |
|---:|---:|---:|---:|---|
| 6.200 *(rig atual)* | 2 | **0,55 ms** | 0,68 ms | ✅ 45× de folga |
| 24.800 *(4×)* | 6 | 2,87 ms | 17,7 ms | ✅ |
| 62.000 *(10×)* | 14 | 5,79 ms | 8,3 ms | ✅ |
| 124.000 *(20×)* | 27 | 12,27 ms | 38,0 ms | ✅ (avg) |
| 248.000 *(40×)* | 53 | 23,05 ms | 31,9 ms | ✅ (avg, no limite) |

Acima de ~50k px o `AutoGpuPlasma` (feature `gpu`) move o render para WGSL —
os números acima são **CPU pura**; o teto real com GPU é maior.

## Conclusão

- **O software não é o gargalo.** A plataforma sustenta 40× o rig atual em CPU.
- O limite do rig é o **transporte**: ESP32+WiFi+ArtNet ≈ 1.500–2.500 px
  utilizáveis por controlador (jitter + perda). Caminhos, na ordem:
  1. **DDP** (`led-player --ddp`): 487 px/pacote vs 170 do ArtNet → ~3× menos
     pacotes, mesma infra — ganho imediato sem trocar hardware.
  2. **Ethernet nos controladores** (ESP32-ETH/QuinLED/Olimex POE): remove o
     jitter WiFi — obrigatório para show ao vivo (Hardware Rule).
  3. **Mais controladores por robô** (`RouterDevice` roteia universos por
     segmento): 2 controladores/robô dobra o rig sem tocar no software.
- Para o objetivo "mais LEDs nos robôs": com DDP + ETH, **12.000–15.000 px por
  robô** (10× a densidade atual) ficam dentro do orçamento de fio e a plataforma
  nem percebe (62k px = 5,8 ms/frame).
