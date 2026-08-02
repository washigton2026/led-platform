# ADR-0020 — `WhiteMode::MinSubtract`: derivação subtrativa do branco

- **Status:** aceito
- **Data original:** 2026-08-02
- **Fonte:** revisão externa (GLM) + verificação no código; achado com consequência **física**

## Contexto e problema
O [ADR-0011](0011-colorformat-rgbw-no-mapper.md) introduziu `WhiteMode::Min`, que calcula
`W = min(r,g,b)` e — como a própria documentação registra em `led-core/src/types.rs:48-50` —
**deixa os bytes RGB inalterados** ("simple, non-destructive"). O código confirma
(`types.rs:97-103`): para `Rgbw`, escreve `b[0],b[1],b[2]` e **acrescenta** `wm.white(c)`.

O efeito é aditivo, não colorimétrico:

| Cor lógica | Fio com `Min` | Dies acesos |
|---|---|---|
| branco `(255,255,255)` | `[255,255,255,255]` | **4 no máximo** |

**Consequência elétrica.** Num SK6812 cada die desenha ~20 mA a 5 V. Para branco pleno:

| Modo | por pixel | 720 px | vs RGB |
|---|---|---|---|
| RGB (3 canais) | 60 mA | 43,2 A | — |
| **RGBW `Min` (atual)** | **80 mA** | **57,6 A** | **+33 %** |
| RGBW subtrativo | 20 mA | 14,4 A | **−67 %** |

Ou seja: o modo atual desenha **4× mais corrente que o subtrativo** para branco pleno, e
**+33 % em relação a uma fita RGB comum** — exatamente o oposto do motivo de existir um die
branco dedicado (mais eficiente e com melhor CRI que somar três coloridos).

**Consequência fotométrica.** O die branco **soma** luz ao branco RGB, então a saída é mais
brilhante que a cor lógica pedida — o branco deixa de ser neutro e o `brightness` do
`Calibration` passa a mentir sobre a intensidade real.

> Números nominais por die; fitas reais variam e o ABL do controlador limita de qualquer forma.
> O que decide este ADR não é o valor absoluto, mas a **razão de 4×** e o fato de o
> comportamento atual contrariar a intenção do hardware.

## Decisão
Adicionar **`WhiteMode::MinSubtract`**: `W = min(r,g,b)` e **subtrai** esse componente neutro
dos três canais coloridos (saturando em zero), que é o comportamento colorimétrico padrão.

```text
(255,255,255) -> W=255, RGB=(0,0,0)      // branco puro sai só pelo die branco
( 10, 20, 30) -> W=10,  RGB=(0,10,20)    // o excedente colorido permanece
( 255, 0,  0) -> W=0,   RGB=(255,0,0)    // cor saturada não muda
```

- **`WhiteMode::Min` permanece**, sem alteração de bytes — é comportamento já testado, e há
  uso legítimo (firmwares/fixtures que esperam branco aditivo, ou quem quer branco extra
  brilhante conscientemente). Sua documentação passa a **avisar** sobre a corrente.
- **Os presets RGBW embutidos passam a usar `MinSubtract`.** Nenhum deles foi validado em
  hardware ainda, e o padrão seguro deve ser o que não surpreende a fonte.
- `ColorFormat` é **`Evolving`** ([ADR-0007](0007-semver-certified-seams.md)) → variante nova é
  **aditiva**; bump MINOR de `led-core` (1.3.0 → 1.4.0), sem quebrar contrato Frozen.

## Escopo / Não-escopo
- **Escopo:** a variante subtrativa, os presets embutidos, a documentação de risco elétrico.
- **Não-escopo:** white balance / temperatura de cor · RGB+CCT (5 canais) · qualquer forma de
  limitação de corrente em software — ver limites de segurança.

## Alternativas rejeitadas
- **Mudar `Min` para subtrair** — alteraria silenciosamente os bytes no fio para qualquer
  configuração existente e quebraria testes que fixam o comportamento atual. Uma variante nova
  é aditiva e explícita.
- **Remover `Min`** — há uso legítimo do branco aditivo; remover seria decidir pelo operador.
- **Derivar o branco no efeito/engine** — violaria o Invariante L↔P: a conversão acontece uma
  vez, no mapper (ADR-0011).
- **Escalar o RGB por um fator em vez de subtrair** — não é o comportamento padrão e
  introduziria não-linearidade sem necessidade.

## Limites de segurança
Isto **não é proteção elétrica**. `MinSubtract` reduz a corrente de branco, mas o limite real
continua sendo a fonte e o ABL do controlador; `Power` no perfil ([ADR-0018](0018-hardwareprofile-capacidades-design-time.md))
é **declarativo**. Trocar de modo **não autoriza** aumentar carga.

## Isolamento do hot-path
A subtração são três `saturating_sub` por pixel, dentro do `ColorFormat::write` que já roda no
`apply` — nenhuma alocação, nenhuma ramificação nova por pixel além do `match` de variante que
já existia.

## Degradação segura
Configurações existentes que usam `Min` continuam **byte-idênticas**. Um perfil que declare
`MinSubtract` produz menos corrente, nunca mais — a mudança de padrão não pode surpreender
para o lado perigoso.

## Consequências
**Boas:** branco pleno passa a usar o die dedicado (mais eficiente, melhor CRI) em vez de somar
quatro canais; corrente de branco cai ~4×; o `brightness` volta a corresponder à intensidade
real. **Ruins/custos:** duas variantes com semânticas próximas exigem documentação clara;
quem já dependia do brilho extra do modo aditivo precisa declarar `Min` explicitamente; bump
MINOR de `led-core` e atualização do baseline do semver-guardian.

## Métricas / gates
- Prova de bytes: `(255,255,255)` com `MinSubtract` → `[0,0,0,255]`; com `Min` → `[255,255,255,255]`.
- Prova de que `Min` **não mudou** (byte-idêntico ao ADR-0011).
- Saturação: nenhum canal fica negativo; cor saturada (`255,0,0`) não é alterada.
- `channels()` continua 4 para ambas as variantes.
- `semver-guardian` verde **com** bump 1.3.0 → 1.4.0.
- Gate elétrico como teste: soma dos canais no fio para branco pleno é **menor** com
  `MinSubtract` que com `Min` — a redução é verificada, não afirmada.

## Critério de reversão
Se uma fita real exigir o branco aditivo como padrão (medido, não suposto), trocar o padrão
dos presets de volta — a variante `Min` continua disponível exatamente para isso. Remover
`MinSubtract` só se a subtração se mostrar incorreta em hardware, o que contrariaria a prática
colorimétrica padrão.
