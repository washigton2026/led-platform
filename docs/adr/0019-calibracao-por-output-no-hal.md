# ADR-0019 — Calibração por-output aplicada no HAL

- **Status:** aceito
- **Data original:** 2026-07-29
- **Fonte:** Achado H5 da Revisão Constitucional (FASE 1) + `Calibration` declarado no ADR-0018

## Contexto e problema
O achado H5 dizia: *"gamma/brightness estão no lugar errado — globais no engine
(`led-pixel-engine/src/color.rs:28,35`), quando gamma é propriedade por-chip/por-controlador"*.

Ao verificar o código, **o achado estava impreciso em dois pontos**:

1. **`Gamma` é código morto.** A struct com a LUT de 256 entradas existe em `color.rs:35`, mas
   **não tem nenhum consumidor de produção** — `Gamma::new` não é chamado em lugar nenhum, e o
   tipo nem é reexportado no `lib.rs` do engine. Não está "no lugar errado": não está em lugar
   nenhum.
2. **`color::scale` não é calibração de saída.** Seus dois usos (`reactive.rs:142,189`) aplicam
   a **intensidade do efeito** (`level` derivado da energia de áudio e do decay do flash). Isso
   é espaço lógico e está **correto onde está** — mover seria quebrar os efeitos reativos.

O problema real, portanto, não é *mover* nada: é que o `Calibration { gamma, brightness }`
**declarado** por nó no [ADR-0018](0018-hardwareprofile-capacidades-design-time.md) **não é
honrado por ninguém**. Um campo declarado e ignorado é pior que um campo ausente.

## Decisão
A calibração por-output é aplicada **no HAL, por device, entre o `apply` do mapa e o fan-out**:

```text
layout.apply(frame, &mut scratch)              // mapeamento — INALTERADO
for dev in devices {
    let range = layout.device_range(dev.id());
    if let Some(lut) = calibration_of(dev.id()) {   // uma ramificação POR DEVICE
        for b in &mut scratch[range] { *b = lut[*b] }
    }
    dev.send_physical(&scratch[range])
}
```

Regras que esta decisão fixa:

1. **Gamma e brightness são dobrados numa ÚNICA LUT de 256 entradas**, pré-computada no
   startup: `lut[i] = ((i/255)^gamma * brightness * 255).round()`. No hot path há apenas uma
   leitura indexada por canal — nenhuma potenciação, nenhuma multiplicação em ponto flutuante.
2. **Custo zero quando não há calibração.** Sem LUT registrada para o device, o laço nem entra:
   a ramificação é **por device** (tipicamente ≤ 5 por frame), nunca por pixel.
3. **O `led-hal` recebe `f32` (gamma, brightness), não o tipo do profile.** Assim ele **não**
   ganha dependência de `led-hardware-profile`; a ligação profile→HAL é do app, no startup.
4. **Nenhum contrato Frozen muda.** `CompiledLayout`, `PixelPhysical`, `UniverseData`,
   `DeviceDriver`, `ProtocolOutput` e o `led-core` inteiro ficam **intocados** — a calibração
   opera sobre o scratch já mapeado, que é contíguo por device (`device_range`).
5. **`color::scale` e os efeitos reativos não são tocados.** Intensidade de efeito e calibração
   de saída são conceitos distintos e permanecem separados.

## Escopo / Não-escopo
- **Escopo:** aplicação por-device de gamma+brightness na borda de saída; a LUT combinada; o
  construtor aditivo no `Hal`.
- **Não-escopo:** ligar automaticamente o `HardwareProfile` ao `Hal` (é o app quem cabla, no
  startup); calibração por-canal ou por-strip (hoje é por device); correção de temperatura de
  cor / white balance; mexer em `color::scale` ou nos efeitos.

## Alternativas rejeitadas
- **Aplicar em `CompiledLayout::apply`** (dentro do mapper) — exigiria mudar `CompiledLayout` /
  `PixelPhysical`, ambos no caminho de um contrato **Frozen** (ADR-0007), para um ganho que a
  aplicação por-device já entrega. Rejeitado por custo de contrato desnecessário.
- **Aplicar em cada `DeviceDriver`** — duplicaria a mesma lógica em sACN, Art-Net, DDP e em
  todo driver futuro; o HAL é o ponto único onde já se itera por device.
- **Calibrar em espaço lógico (no engine)** — gamma é propriedade do chip/controlador; aplicá-la
  antes do mapeamento a tornaria global de novo, que é exatamente o achado H5.
- **Mover `color::scale`** — seria quebrar a intensidade de efeito, que não é calibração.
- **Deixar `Gamma` como está (morta)** — mantém um campo declarado no profile sem efeito, o
  pior dos mundos.

## Limites de segurança
Uma LUT com `brightness` alto **não** protege contra sobrecorrente — `Power` no profile é
declarativo (ADR-0018) e o limite elétrico real é a fonte e o ABL do controlador. A calibração
é correção óptica, nunca proteção.

## Isolamento do hot-path
Uma leitura indexada por canal, sobre uma LUT de 256 bytes que cabe em L1, num scratch já
contíguo por device. **Sem alocação** (a LUT é construída no startup), sem lock adicional, sem
ramificação por pixel. O gate `no_alloc` do `led-hal` continua sendo a prova; a latência é
medida pelo bench de layout/latência antes e depois.

## Compatibilidade de OS
Agnóstico — aritmética e memória apenas.

## Degradação segura
Sem calibração registrada, o comportamento é **byte-idêntico** ao de hoje (o laço não roda).
`gamma <= 0` ou `brightness` fora de `0..=1` já são recusados pelo validador do profile
(ADR-0018); na construção da LUT os valores são saneados por `clamp`, de modo que uma entrada
absurda nunca gera índice inválido.

## Consequências
**Boas:** o `Calibration` declarado no preset passa a ter efeito real; gamma deixa de ser código
morto; a correção fica por-nó, como o hardware exige; zero contrato tocado; custo nulo quando
não usada.
**Ruins/custos:** um segundo passe sobre os bytes do device quando há calibração (medido, não
assumido); a ligação profile→HAL ainda é manual no app; calibração é por device, não por strip
— um nó com fitas de chips diferentes precisaria de granularidade maior (fora de escopo).

## Métricas / gates
- `led-hal/tests/no_alloc.rs` verde **com** calibração ativa (zero alocação no hot path).
- Bench de latência do `send_frame` **com e sem** calibração — o custo é medido e reportado,
  nunca estimado. **Medição (2026-07-29, 6.200 px / 37 universos, build debug,
  `led-hal/tests/calibration_output.rs::bench_calibration_cost`):**
  sem calibração `338.775 ns/frame`, com calibração `472.583 ns/frame` →
  **delta `+133.808 ns` (×1,39), ~2,7% do orçamento de 5 ms**. Em release o custo é
  substancialmente menor; o número acima é o pior caso medido.
- Prova de bytes: um device calibrado emite bytes transformados pela LUT; um device sem
  calibração emite bytes **idênticos** ao comportamento atual.
- `semver-guardian` verde: `led-core` inalterado.

## Critério de reversão
Se a medição mostrar que o segundo passe consome fatia relevante do orçamento de 5 ms num rig
grande, dobrar a calibração para dentro do `apply` (pagando o custo de contrato) passa a se
justificar. Enquanto o custo for marginal, a colocação no HAL é preferível por não tocar
contrato algum.

---

## Emenda 1 (2026-08-07) — a calibração passa para a **fronteira lógica de saída**

**Estado:** aceite. Substitui a colocação da §Decisão para o caminho do daemon; o HAL
mantém a sua implementação e os seus testes.

### O que forçou a emenda

A decisão original diz que a calibração é aplicada *"no HAL, por device, entre o `apply` do
mapa e o fan-out"*. Essa frase pressupõe que **todo o caminho de saída atravessa o HAL**.

Não atravessa. O `DdpOutput` **contorna o HAL por decisão anterior e deliberada**
(changelog de 2026-07-09d: *"DDP bypassa o Hal — pixel-nativo, sem mapa de universos"*):
`send_frame` entrega `frame.pixels` diretamente ao `DdpDevice`, sem `CompiledLayout` e sem
`scratch` por device. **Não existe, no caminho DDP, o sítio que esta ADR nomeia.**

A consequência foi medida por leitura de código, não suposta:

| Caminho | Calibração antes desta emenda |
|---|---|
| `led-player` → Art-Net / simulador | aplicada (`main.rs:399`, `:403`) |
| `led-player` → DDP | **ausente** |
| `led-daemon` → Art-Net / sACN | **ausente** |
| `led-daemon` → DDP | **ausente** |

DDP é o protocolo **validado em hardware** (94/94 frames, 2026-07-20) e o que o preset
`esp32-poe-wled-ddp` declara — ou seja, o caminho do GS4.5. Um nó declarado a gamma 2.2
receberia bytes lineares.

### Decisão

A calibração passa a ser aplicada **no `OutputManager`, sobre o `LogicalFrame`, antes do
fan-out protocolar**. Um só ponto, os três protocolos, sem exceção:

```
HardwareProfile.calibration
        │
        ▼
OutputManager::send  ──►  LUT aplicado ao frame  ──►  DDP | Art-Net | sACN  ──►  fio
```

### Porque não a alternativa óbvia

**Acrescentar `.with_calibration()` só às ramificações que usam `Hal`** daria calibração no
Art-Net e no sACN e **nenhuma no DDP** — pior que a ausência uniforme, porque *pareceria
feito*. É a classe do defeito de `RgbOrder` do GS4.3: um campo declarado que um caminho
honra e outro não.

**Dar ao `DdpOutput` calibração própria** seria uma **segunda implementação** da mesma
transformação. Duas curvas de gamma no projeto é uma a mais.

### O que se perde, e porque é aceitável

O argumento original — *"a ramificação é por device (tipicamente ≤ 5 por frame), nunca por
pixel"* — deixa de valer da mesma forma: na fronteira lógica o LUT percorre os canais do
frame. **Continua a ser uma leitura indexada por canal**, sem `powf` e sem ponto flutuante;
o que muda é *onde* o laço corre, não o seu custo assintótico. E com **um alvo por saída**
(o `--output` do daemon aceita um), a ramificação por device que a ADR protegia não existe
hoje. Se o multi-controlador chegar com calibrações divergentes por nó, esta emenda tem de
ser revisitada — é o critério de reversão desta emenda.

### O que NÃO muda

- `CalibrationLut` e `Calibration` do `led-hal` são **reusados tal como estão**. Nenhum tipo
  novo, nenhuma segunda LUT.
- `Hal::with_calibration` **permanece**, com os seus testes: o `led-player` continua a
  usá-lo, e o `led-hal` não perde uma capacidade que já está provada.
- O contrato do `led-daemon` (GS1.6) não é tocado — a calibração vive no `led-daemon-bin`.
- Calibração continua a ser **correção óptica, nunca proteção elétrica** (§Limites de
  segurança permanece integralmente em vigor).

### Gate desta emenda

Um teste discriminante por protocolo que compare os **bytes no fio** com e sem calibração, e
que reprove se algum protocolo voltar a ignorá-la. Sem esse teste a emenda não está feita.
