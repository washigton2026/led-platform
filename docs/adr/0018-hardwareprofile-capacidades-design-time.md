# ADR-0018 — `HardwareProfile`: descritor de capacidades em design-time

- **Status:** aceito (pré-implementação — congela o contrato antes do código)
- **Data original:** 2026-07-29
- **Fonte:** Revisão Constitucional (FASE 3) + Atualização de Contexto do LUMYX (governança) +
  resolução do Conflito A (relatório técnico 2026-07-29)

## Contexto e problema
A configuração de um nó de hardware está hoje **espalhada** e **incompleta**: argumentos de
construtor por driver (`SacnDevice::unicast/multicast`, `DdpDevice::new`,
`DdpBackend::with_channels`), o mapa/universos no `CompiledLayout`
(`led-core/src/mapping.rs`), a cor por-pixel no `ColorFormat` (`led-core/src/types.rs:68`,
ADR-0011), **gamma/brightness globais no engine** (`led-pixel-engine/src/color.rs:28,35`) em
vez de por-output, e **nenhuma** representação para `voltage`, `max_current` ou `refresh_hz`.

Falta o **descritor** que amarra isso. O risco previsto é modelar hardware como **enum plano
de produtos** (`ESP32 | ESP32-POE | WLED | Falcon | Advatek | …`), que mistura dimensões
ortogonais (placa, firmware, interface física, formato de cor) num só nível. Duas evidências
decidem contra:

1. **Explosão combinatória.** "ESP32-POE rodando WLED, saída SPI, SK6812-RGBW, falando DDP"
   exigiria uma variante por combinação (≈120+), cada uma propagando braços de `match`.
   Hardware novo passaria a exigir código novo.
2. **Duplicaria um ADR aceito.** `ColorFormat`/`WhiteMode` já resolvem RGBW (ADR-0011).

E o próprio codebase **já trata hardware como dado**: o importador xLights, que roda no rig
real de 5 robôs, lê `Type="Ethernet" Vendor="WLED" Protocol="ArtNet"`
(`led-xlights/src/lib.rs:781,785`) — vendor é **dado**; interface e protocolo são
**capacidades**. `Falcon`/`Advatek`/`WLED` **não existem como tipo** em nenhum crate.

## Decisão
`HardwareProfile` é um **descritor declarativo de design-time, modelado por capacidades
ortogonais** — nunca um enum de produtos. Estrutura canônica:

```text
HardwareProfile                      (DADO puro, serializável, design-time)
 ├─ schema_version                   versionamento do próprio schema (migrações)
 ├─ Identity        : vendor, model, firmware, firmware_version, serial
 ├─ Capabilities    : protocol, output_interface, color,
 │                    supports_discovery (bool), supports_metrics (bool)
 ├─ Limits          : pixels_per_universe, max_pixels, refresh_hz   ← ÚNICO lar dos limites
 ├─ Power           : voltage, max_current            (limites DECLARADOS, design-time)
 └─ Calibration     : gamma, brightness               (hoje globais no engine — a mover)
```

Regras que esta decisão fixa:

1. **`OutputInterface` declara; `DeviceDriver` executa.** (Resolução do Conflito A, opção B.)
   O profile **nomeia** a interface física de saída (`Ethernet`, `WiFi`, `Spi`, `Pwm`, …) como
   **dado**; a **implementação** de cada uma é um `DeviceDriver` — SPI e PWM são drivers, não
   campos de comportamento do profile. Declarar uma interface **não a implementa**: um preset
   que declare uma interface sem driver disponível **falha explicitamente no startup**.
   **Nome:** `OutputInterface` (e não `Connection`) porque `DeviceStatus { connected: bool }`
   já existe em `led-core` (Frozen, runtime) e `Connection` colidiria semanticamente com
   conectividade de runtime — justamente a fronteira que este ADR separa.
2. **Presets são dado, não código.** ESP32, ESP32-POE, Falcon, Advatek, Raspberry Pi, WLED e
   Custom são **pacotes nomeados de valores de capacidade**. Hardware novo = **linha de dado
   nova**: zero braço de `match`, zero `if`, zero código. **Nenhuma lógica dentro de preset.**
3. **`HardwareRegistry` registra e localiza presets.** O `HardwareProfile` permanece
   descrição declarativa pura; a busca/registro vive no registry, separado do descritor.
4. **`RuntimeState` NÃO faz parte do `HardwareProfile`.** Estado de runtime (online,
   temperatura, tensão/corrente medidas, métricas) fica **fora**, reutilizando o que já
   existe — `DeviceStatus` (`led-core`, Frozen) e `ReadModel`/`MetricsView`
   (`led-readmodel`). Campos ainda inexistentes (temperatura, tensão/corrente **medidas**)
   entram por **extensão do read-model**, nunca por segunda representação.
   *Distinção:* `Power` no profile = limites **declarados** (design-time); tensão/corrente em
   runtime = **medidas** (read-model). Dimensões diferentes, sem conflito.
5. **`Capabilities` só contém capacidades declarativas ou booleanas.** Limites numéricos de
   pixel vivem **apenas** em `Limits` (sem duplicação `Capabilities.PixelLimits`).
6. **Versionamento:** `schema_version` (evolução do schema, migrações futuras) e
   `firmware_version` (em `Identity`, capacidade dependente de firmware).
7. **Compila e desaparece.** Resolvido **uma vez, no startup**:
   `HardwareProfile → CompiledLayout + Driver Configuration → Runtime`. **Nunca** consultado
   durante renderização; nunca no caminho `Show → Logical Pixels → ProtocolOutput → HAL →
   DeviceDriver → Hardware`.
8. **Zero mudança em seam Frozen.** `ProtocolOutput`, `DeviceDriver`, `IDevice`,
   `CompiledLayout`, `UniverseData` (ADR-0007) intocados — o profile **alimenta** suas
   construções, não altera assinaturas. **`ColorFormat`/`WhiteMode` reusados como estão**
   (ADR-0011); nenhuma segunda representação de RGBW.

## Escopo / Não-escopo
- **Escopo:** o schema de capacidades; `HardwareRegistry`; presets como dado; versionamento;
  a regra de que o profile compila para os seams e desaparece.
- **Não-escopo:** implementação dos drivers **SPI/PWM/ESP-NOW** (são `DeviceDriver` novos, ADR
  próprio — o profile só declara a interface); mover gamma/brightness do engine para
  por-output (achado H5, slice separada); RGB+CCT / 5 canais (variantes futuras de
  `ColorFormat`, que é `Evolving`); UI/editor (ADRs 0013–0016); blackout (ADR-0017);
  `RuntimeState` (fica no read-model).

## Alternativas rejeitadas
- **Enum plano de produtos** (`ESP32 | WLED | Falcon | … | RGBW`) — explosão combinatória,
  `match` infinito, duplicaria o ADR-0011. Contradito pelo próprio `led-xlights`.
- **`Connection` como nome da capacidade** — colide semanticamente com
  `DeviceStatus.connected` (runtime). `OutputInterface` é inequívoco.
- **`RuntimeState` dentro do profile** — misturaria design-time com runtime e criaria segunda
  representação de `DeviceStatus`/`ReadModel`.
- **`PixelLimits` em `Capabilities`** — duplicaria `Limits`.
- **Profile como objeto de runtime** (estado/comportamento, consultado por frame) — God Object
  no hot-path.
- **Registro de presets dentro do `HardwareProfile`** — misturaria descrição com localização;
  daí o `HardwareRegistry` separado.
- **Um profile por protocolo** (SacnProfile, DdpProfile…) — reintroduz a combinatória e impede
  um rig misto num só descritor.

## Limites de segurança
`OutputInterface` é onde a Hardware Rule "WiFi proibido ao vivo" (ADR-0005) é **declarada**: um
profile com interface WiFi **não pode iniciar show ao vivo**. O **enforcement** permanece no
`NetworkGuard` (`led-hal/src/network_guard.rs`) — o profile declara a intenção, o guard bloqueia.
`Power` é declarativo (aviso de orçamento de corrente), **não** é proteção elétrica.

## Isolamento do hot-path
Resolvido no startup (management plane) e inexistente quando o primeiro frame roda. Nenhuma
leitura de profile em `send_frame`, `apply` ou no render. Nenhuma alocação nova no hot-path.

## Compatibilidade de OS
Agnóstico: é dado serializável. Dependências de OS ficam nos mecanismos existentes —
`NetworkGuard` (macOS/Linux; **no Windows retorna `ProbeUnavailable`**, gap conhecido) e nos
drivers de protocolo.

## Degradação segura
Profile ausente/parcial → construção explícita de driver + layout continua válida (o profile é
**aditivo**, não obrigatório). Campo/valor desconhecido → **rejeitado na validação de
design-time**, antes do show, nunca ignorado em silêncio. Preset declarando interface sem
driver → **erro explícito no startup**. `schema_version` desconhecida → rejeitada (migração
explícita, nunca best-effort).

## Consequências
**Boas:** hardware novo entra como dado (**Falcon/Advatek exigem zero código** — já falam
sACN/Art-Net); rig misto cabe num conjunto de profiles; a configuração espalhada ganha um lar;
abre `Power`/`Limits` que hoje não existem; contratos Frozen e SemVer preservados;
`schema_version` permite migração sem quebra.
**Ruins/custos:** um schema de capacidades é menos "óbvio" que uma lista de produtos — exige
catálogo de presets bem documentado; validação de design-time é código novo; interfaces
declaráveis mas sem driver (`Spi`/`Pwm`) criam expectativa — mitigado por erro explícito no
startup; o `HardwareRegistry` adiciona uma peça a manter.

## Métricas / gates
- Profile compila para o `CompiledLayout` esperado (round-trip de valores).
- Preset RGBW produz os bytes de fio corretos (reusa a prova do ADR-0011).
- **Isolamento:** `no_alloc` do output verde e p99 de `send_frame` inalterado com profile em
  uso (profile fora do hot-path).
- **Gates negativos:** interface WiFi **não** inicia show ao vivo; interface sem driver
  **falha** no startup; `schema_version` desconhecida é **rejeitada**; preset inválido é
  **rejeitado** na validação (Slice 2/4).
- `semver-guardian` verde: nenhuma assinatura Frozen alterada.
- **`HardwareProfileGuardian`** aprova: sem enum gigante, sem lógica em preset, design-time
  isolado de runtime, sem dependência de HAL/Driver/Show Engine, todos os presets validados.

## Plano de slices (ordem obrigatória)
1. **Slice 1** — schema puro (dado). Sem lógica, sem HAL, sem drivers.
2. **Slice 2** — validador: driver inexistente, protocolo inválido, combinação inválida,
   RGBW incompatível.
3. **Slice 3** — presets (somente dados) + `HardwareRegistry`.
4. **Slice 4** — validação de todos os presets.
5. **Slice 5** — integração ao HAL (compilação para `CompiledLayout` + Driver Configuration).

## Critério de reversão
Se o schema de capacidades provar-se insuficiente para um hardware real (configuração
inexpressável por composição), **adicionar a capacidade faltante como campo** — não converter
o modelo em enum de produtos. Reverter para enum plano exigiria revogar este ADR **e** o
ADR-0011 e reintroduzir a combinatória; só se justificaria com evidência de que a composição é
inexpressável, o que o `led-xlights` hoje contradiz.
