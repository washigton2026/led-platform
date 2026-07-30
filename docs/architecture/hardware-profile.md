# HardwareProfile — arquitetura

> **Decisão e justificativa** vivem no [ADR-0018](../adr/0018-hardwareprofile-capacidades-design-time.md).
> Este documento descreve **como funciona** e **como usar/estender**.
> Crate: `crates/led-hardware-profile` · Guardião: `.claude/agents/hardwareprofile-guardian.md`

## O que é

Um **descritor declarativo de capacidades**, em **design-time**, de um *tipo* de hardware.
Não é um catálogo de produtos, não é estado de runtime, e não executa nada.

Três frases que resumem a arquitetura inteira:

1. **O profile declara; o `DeviceDriver` executa.**
2. **Preset é um *tipo* de hardware; endereço/`device_id` são da *instância*.**
3. **O profile compila no startup e desaparece** — nunca é consultado na renderização.

## Onde ele fica no sistema

```text
DESIGN-TIME                                  │  RUNTIME
                                             │
preset (linha da tabela, dado)               │
   │                                         │
   ▼                                         │
HardwareProfile ──▶ validate() ──▶ ok?       │
   │                                         │
   ├──▶ compile_layout() ──▶ CompiledLayout ─┼──▶ Hal ──▶ DeviceDriver ──▶ hardware
   └──▶ driver_config()  ──▶ DriverConfig ───┘        (o profile já não existe aqui)
```

O crate é **leaf**: depende **apenas** de `led-core`. Ele nunca importa `led-hal`,
`led-protocols`, `led-pixel-engine` ou `led-sequencer` — é isso que garante que a fronteira
design-time/runtime não vaze.

## Estrutura

```text
HardwareProfile
 ├─ schema_version                 versionamento do schema (migração é explícita)
 ├─ Identity      vendor · model · firmware · firmware_version · serial
 ├─ Capabilities  protocol · output_interface · color · supports_discovery · supports_metrics
 ├─ Limits        pixels_per_universe · max_pixels · refresh_hz
 ├─ Power         voltage_v · max_current_a          (DECLARADO, não medido)
 └─ Calibration   gamma · brightness
```

Regras de forma que o guardião cobra:

- **`Capabilities` só tem valor declarativo ou booleano.** Limites numéricos vivem **apenas**
  em `Limits` — sem `Capabilities.PixelLimits`.
- **Cor é `led_core::ColorFormat`/`WhiteMode`** ([ADR-0011](../adr/0011-colorformat-rgbw-no-mapper.md)),
  reusada como está. Uma segunda representação de RGBW é proibida.
- **Nada de runtime aqui.** Online/temperatura/corrente *medida*/métricas ficam em
  `led_core::DeviceStatus` e no `led-readmodel`. `Power` são **limites declarados**;
  tensão/corrente medidas são runtime — dimensões diferentes, sem conflito.

## `OutputInterface` — declara, não implementa

`Ethernet` · `WiFi` · `Spi` · `Pwm`.

O schema **expressa** todas; **implementá-las é outra coisa**. Hoje existem drivers para
Ethernet/WiFi (via sACN/Art-Net/DDP); **SPI e PWM não têm driver** — declará-los é legítimo, e
o validador **recusa explicitamente** quando não há driver disponível. Nunca falha em silêncio.

> O nome é `OutputInterface` e não `Connection` porque `led_core::DeviceStatus` já carrega
> `connected` (conectividade de **runtime**) — a colisão semântica confundiria justamente a
> fronteira que este desenho separa.

## Presets são dado

`crates/led-hardware-profile/src/presets.rs` é uma **tabela**: `const PRESETS: &[PresetRow]`
com **zero `fn`, zero `impl`, zero ramificação**. Como todo campo é literal ou variante de
enum, a tabela é uma `const` genuína — dado de tempo de compilação onde é impossível esconder
lógica. A conversão linha→profile mora no `registry`, porque é responsabilidade do registro.

Embutidos hoje: `esp32-devkit-wled-artnet`, `esp32-poe-wled-ddp`, `falcon-f16v3-sacn`,
`advatek-pixlite16-sacn`, `raspberry-fpp-sacn`, `generic-sk6812-rgbw-sacn`, `custom`.

**ESP32, Falcon, Advatek, Raspberry Pi e WLED são LINHAS, não variantes de enum.** WLED é
*firmware* e usa protocolos que já existem — nenhum código específico de fabricante existe nem
é necessário.

> Os números dos presets (`max_pixels`, `refresh_hz`, `Power`) são **pontos de partida
> plausíveis por família**, não medições. Ajuste por instalação contra a folha de dados e a
> fonte usada.

### Como adicionar hardware novo

**Uma linha na tabela.** Nenhum `match`, nenhum `if`, nenhum tipo novo:

```rust
PresetRow {
    name: "meu-controlador",
    vendor: "…", model: "…", firmware: "…", firmware_version: "…",
    protocol: Protocol::Sacn,
    output_interface: OutputInterface::Ethernet,
    color: ColorFormat::Rgb(RgbOrder::Grb),
    supports_discovery: true, supports_metrics: false,
    pixels_per_universe: 170, max_pixels: 4_096, refresh_hz: 44,
    voltage_v: 5.0, max_current_a: 20.0,
    gamma: 2.2, brightness: 1.0,
}
```

Um `DeviceDriver` novo só é necessário quando houver um **transporte físico novo** de verdade
(SPI, PWM, ESP-NOW) — não para um controlador novo que fale um protocolo já suportado.

## Validação (design-time)

```rust
let available = Available { interfaces: &[…], protocols: &[…] };  // injetado como DADO
let report = validate(&profile, &available);
if report.has_errors() { /* não compile, não envie */ }
```

**Por que `Available` é injetado:** detectar "driver inexistente" exige saber quais drivers
existem — mas o crate é leaf. Quem conhece os drivers (o HAL, no startup) passa a lista **como
dado**. Zero dependência criada.

| Regra | Severidade |
|---|---|
| `schema_version` desconhecida | erro |
| interface sem driver (hoje `Spi`/`Pwm`) | erro |
| protocolo sem driver | erro |
| `pixels_per_universe × canais > UNIVERSE_SIZE` (só protocolos com universo) | erro |
| limites zerados · `Power` não-positivo · `Calibration` fora de faixa (inclui `NaN`) | erro |
| **RGBW sobre DDP** (o cabeçalho DDP fixa data type RGB8) | **aviso** |
| **WiFi** — proibido ao vivo ([ADR-0005](../adr/0005-wifi-proibido-producao.md)) | **aviso** |

WiFi é **aviso, não erro**: o profile **declara**, o `NetworkGuard` **bloqueia** o início do
show. O validador não usurpa o enforcement. Achados **acumulam** — um profile ruim reporta tudo
de uma vez, não a primeira falha.

## Compilação (startup)

```rust
let layout = compile_layout(&profile, pixel_count, device_id, first_universe)?;
let cfg    = driver_config(&profile, device_id, address, first_universe);
```

- **Honra o `pixels_per_universe` declarado**, mesmo abaixo do máximo teórico — controladores
  legitimamente empacotam menos, e ignorar o valor esvaziaria o campo.
- **Recusa em vez de mapear errado:** `ExceedsMaxPixels`, `ZeroPixelsPerUniverse`,
  `PixelsExceedUniverse`.
- `DriverConfig` é **dado**: descreve o que construir. Instanciar o driver exige socket (I/O =
  runtime) e é responsabilidade do chamador — por isso a compilação cabe no crate leaf.

## Prova ponta a ponta

`integration-tests/tests/hardware_profile_e2e.rs` percorre
**preset → validate → CompiledLayout → DriverConfig → Hal → SimulatorDevice** e verifica os
**bytes** que chegam ao dispositivo: a ordem GRB declarada no preset, os 4 canais do preset
RGBW com o branco derivado no mapper, e um `pixels_per_universe` declarado abaixo do máximo
sobrevivendo até o fio. Inclui o controle negativo: sem driver para a interface, o fluxo **para
na validação**.

Ele vive no `integration-tests` (e não no crate do profile) porque o `SimulatorDevice` está no
`led-hal` — assim o E2E existe sem o profile ganhar dependência de HAL.

## O que o guardião bloqueia

`.claude/agents/hardwareprofile-guardian.md` — 8 checks executáveis: enum de produto · lógica
em preset · runtime no descritor · dependência de HAL/driver/engine · segunda representação de
RGBW · preset sem teste de validação · seam Frozen alterado · profile referenciado no runtime.

## Fora de escopo (têm ADR próprio)

Drivers **SPI/PWM/ESP-NOW** · mover `gamma`/`brightness` do engine para por-output · RGB+CCT /
5 canais · UI/editor de profiles · `profile_version` separado de `schema_version` (a decidir se
o catálogo de presets vier a precisar de revisão própria).
