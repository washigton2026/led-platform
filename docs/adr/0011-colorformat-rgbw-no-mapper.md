# ADR-0011 — `ColorFormat` no mapper: suporte RGBW/4-canais aditivo

- **Status:** aceito
- **Data original:** 2026-07-25
- **Fonte:** Revisão Constitucional HardwareProfile (FASE 1–8), achado C1

## Contexto e problema
A stack inteira assumia **3 canais/pixel (RGB)** hardcoded: `RgbOrder` só tinha
`Rgb/Grb/Bgr` com `bytes() -> [u8; 3]`; `CompiledLayout::apply` escrevia exatamente
3 bytes; `linear`/`mapper`/`rig` avançavam `+3`. Consequência: **SK6812-RGBW, TM1814
e RGB+CCT — metade da lista de hardware alvo — não tinham representação possível**
(achado C1 da Revisão Constitucional). Pior: `UniverseData`, `CompiledLayout`,
`DeviceDriver`, `ProtocolOutput` e `IDevice` são **Frozen** (ADR-0007), então a
correção não podia quebrar suas assinaturas.

## Decisão
Introduzir `ColorFormat` como o descritor de cor/canais **por-pixel na fronteira
Lógico↔Físico** — o único ponto onde a conversão L↔P acontece:

- `ColorFormat::Rgb(RgbOrder)` (3 canais) preserva 100% do comportamento atual.
- `ColorFormat::Rgbw(RgbOrder, WhiteMode)` (4 canais) adiciona o canal branco,
  **derivado da cor lógica RGB** por `WhiteMode` (`None` = W=0; `Min` = W=min(r,g,b)).
- O **espaço lógico continua RGB** (`PixelColor{r,g,b}` intocado; Invariante L↔P e
  Constituição §2 preservados) — o branco é computado em `apply`, no mapper, nunca
  num efeito.
- `PixelPhysical.order: RgbOrder` → `PixelPhysical.format: ColorFormat` (tipo de
  suporte, **não** Frozen). `RgbOrder` permanece como a sub-ordenação da tripla RGB.
- `CompiledLayout::apply` passa a escrever `format.channels()` bytes — **mesma
  assinatura**. `CompiledLayout::linear(…, RgbOrder)` fica **inalterado** (Frozen);
  um **novo** `CompiledLayout::linear_format(…, ColorFormat)` cobre RGBW.

## Consequências
**Boas:** RGBW/4-canais viável sem quebrar nenhuma assinatura Frozen (ADR-0007
honrado); sACN/Art-Net são byte-transparentes (padding a DMX_SLOTS) → RGBW flui sem
tocá-los; `linear` e seus ~20 call-sites permanecem intactos. Bump **MINOR** aditivo
(`led-core` 1.2.0 → 1.3.0), validado pelo semver-guardian.
**Ruins/custos:** `PixelPhysical` mudou de campo (`order`→`format`) — 5 sites de
construção atualizados (led-core/led-layout/led-player/led-xlights). O serializador
**DDP** ainda assume 3 bytes/pixel (dtype 0x01 RGB); **RGBW sobre DDP precisa de um
modo 4-canais** (rastreado, fora desta slice — sACN cobre RGBW já). `WhiteMode` cobre
os casos comuns (`None`/`Min`); estratégias mais ricas (por-temperatura, RGB+CCT de
5 canais) ficam para variantes futuras de `ColorFormat` (Evolving).

## Alternativas rejeitadas
- **Estender `RgbOrder::bytes()` para retornar N bytes** — mudaria a assinatura de um
  método usado em testes/efeitos; `ColorFormat` novo é aditivo e não quebra nada.
- **Adicionar `Option<ColorFormat>` a `PixelPhysical` mantendo `order`** — proibido
  pela Constituição §11 (campo `Option<T>` "temporário") e cria duas fontes de verdade.
- **Mudar a assinatura de `CompiledLayout::linear`** — viola o contrato Frozen do
  ADR-0007; por isso `linear` fica intacto e `linear_format` é aditivo.
- **Derivar o branco fora do mapper (num efeito)** — violaria o Invariante L↔P
  (conversão acontece exatamente uma vez, no mapper).
