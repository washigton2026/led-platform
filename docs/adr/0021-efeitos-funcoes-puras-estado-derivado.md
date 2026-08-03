# ADR-0021 — Efeito é função pura; estado é derivado, nunca armazenado

- **Status:** ✅ **aceito**
- **Data:** 2026-08-03
- **Fonte:** FASE E1 do roadmap (`docs/ROADMAP.md`) — fechar a maior lacuna de paridade com
  o xLights: 5 efeitos contra ~40

## Contexto e problema

O LUMYX tem **5 efeitos visuais** (`SolidColor`, `Rainbow`, `Pulse`, `Plasma`, mais os
reativos `BandPulse`/`BeatFlash`); o xLights tem cerca de 40. É a lacuna de paridade mais
larga da plataforma, e a única grande que **não depende de nenhuma decisão pendente** nem de
hardware.

Mas escrever 35 efeitos sem uma regra antes é como o problema fica insustentável. A maioria
das bibliotecas de efeitos de LED (FastLED, WLED, xLights) guarda **estado por-pixel entre
frames**: mapa de calor do fogo, posição acumulada do cometa, `random()` avançando a cada
quadro. É o padrão do setor.

**Esse padrão é incompatível com o LUMYX.** Não por gosto — por três invariantes já pagos:

1. **Replay determinístico verificado por hash** (ADR-0001). Um efeito com estado renderiza
   diferente na segunda passada do mesmo `time_ms`, e o hash agregado deixa de bater. Toda a
   cadeia de replay, assinatura Ed25519 e burn-in por hash depende disso.
2. **Zero alocação no hot path.** Estado por-pixel quer um buffer; buffer quer alocação ou
   mutabilidade interior.
3. **A assinatura já decidiu.** `Effect::render(&self, …)` recebe `&self`
   (`led-pixel-engine/src/effect.rs:26`). Guardar estado exigiria `&mut self` ou
   interior mutability — ou seja, exigiria **mudar o contrato**, não escrever um efeito.

## Decisão

**Todo efeito é uma função pura de `(time_ms, position, index)`.** Três regras derivadas:

### 1. Sem estado guardado

O que num efeito tradicional seria "o que aconteceu no frame anterior" é aqui **derivado do
tempo**. Um cometa não acumula posição: sua posição *é* `speed × t`. Uma cauda não decai
frame a frame: seu brilho *é* uma função da distância até a cabeça.

### 2. Aleatoriedade é hash de coordenadas, nunca um fluxo

Um `Rng` avança a cada chamada — dois renders do mesmo `time_ms` divergem. Em vez disso,
`led_pixel_engine::noise` expõe `hash01(chave, semente)`: **função pura**. "Este pixel
cintila?" é `hash01(índice, semente) < densidade` — estável entre frames por construção, sem
guardar nada.

`mix64` compartilha as constantes do finalizador do SplitMix64 já usado em
`led_sequencer::show_intent` e `led_hal::chaos`, e isso está documentado no módulo. **Não é
uma terceira cópia da mesma coisa:** aqueles são geradores **com estado**
(`fn(&mut u64) -> u64`, avançam um fluxo); este é uma **função pura** (`fn(u64) -> u64`).
Compartilham o misturador porque ele é bom e testado, não porque façam o mesmo trabalho.

### 3. Parâmetro espacial é taxa por metro, nunca coordenada normalizada

O efeito recebe **posições**, não as dimensões do rig. Não existe "0..1 ao longo da fita"
porque o efeito não sabe onde a fita termina. O idioma correto já existia no `Rainbow`:
`cycles_per_m`. Uma taxa por metro funciona igual num rig de 2 m e de 200 m.

Onde a extensão é mesmo indispensável — um cometa que dá a volta precisa saber onde é o fim —
ela é **parâmetro declarado** (`Meteor::span_m`), fornecido por quem monta o show. É a mesma
disciplina do ADR-0018 ("injeção de dado, não de dependência"): quem sabe, passa o valor;
o componente não vai buscar.

## Consequências

### O que se ganha

- Replay, assinatura e burn-in por hash continuam válidos **para qualquer efeito futuro**.
- Efeito é trivialmente testável: sem cenário, sem aquecimento, sem ordem de chamadas.
- Paralelizável e portável para GPU sem reescrita — é a mesma forma do `ComputeKernel`.
- Custo constante por pixel; nada cresce com o histórico.

### O que se perde — e o exemplo concreto

**Algoritmos genuinamente iterativos não são portáveis como estão.** O caso real é o
`Fire2012` do FastLED: ele propaga calor entre pixels vizinhos **de um frame para o
seguinte**. Não existe forma fiel de escrevê-lo sem estado.

O `Fire` do LUMYX usa **ruído fractal deslizante** — visualmente próximo, algoritmicamente
diferente. Isso está escrito na doc do próprio tipo, não escondido. Quem esperar
`Fire2012` byte-a-byte não vai encontrar; quem quiser fogo na fita vai.

O mesmo se aplicará a qualquer efeito com difusão, física ou memória (fluidos, autômatos
celulares, "Life"). **Quando aparecer um caso que valha o custo**, a saída não é furar esta
regra: é um tipo novo e explícito — um `StatefulEffect` com contrato próprio e que declare
que não é replayável — decidido em ADR próprio, não improvisado.

### Segurança de plateia

`Strobe` traz a faixa de risco de convulsão fotossensível como **constante consultável**
(`SEIZURE_RISK_HZ`, `is_in_seizure_risk_band`), e **não clampa em silêncio**. Segue o
precedente do ADR-0018: o componente **declara**, a camada com contexto **decide**. Um
estroboscópio que muda de frequência sozinho no palco é pior que um parâmetro documentado.

## Como isto é verificado (não é aspiração)

| Regra | Gate executável |
|---|---|
| Pureza | `library::tests::every_effect_is_a_pure_function_of_time` — renderiza `t`, depois `t+1`, depois `t` de novo; qualquer estado interno faria a terceira passada divergir da primeira |
| Zero alocação | `crates/led-pixel-engine/tests/no_alloc.rs` — alocador contador, 2.000 frames × 512 px por efeito, **com controle negativo**: um efeito que aloca de propósito tem que ser pego (KB-012) |
| Entrada não-finita | `negative_control_non_finite_positions_do_not_panic` + `negative_control_non_finite_input_never_produces_nan` — a classe de falha do BUG-3 (`smoothstep(NaN)` propagando até posição de drone) |

## Alternativas descartadas

**Dar `&mut self` ao `Effect`.** Resolveria fogo e fluidos de imediato — e quebraria o replay
determinístico, que é uma das poucas coisas que o LUMYX tem e o xLights não. Trocar a
garantia diferenciadora por conveniência de implementação é o negócio errado.

**Interior mutability (`Cell`/`Mutex`) dentro do efeito.** Contorna o compilador sem
contornar o problema: o render deixa de ser reprodutível do mesmo jeito, só que sem o
compilador avisando.

**Cache de estado no chamador, passado como argumento.** Empurraria a complexidade para
todo consumidor e mudaria a assinatura do `Effect` — quer dizer, mudaria o contrato para
resolver um caso que ainda não apareceu.

## Escopo

- **Escopo:** a biblioteca de efeitos de `led-pixel-engine` e todo efeito futuro dela.
- **Não-escopo:** os efeitos reativos (`BandPulse`, `BeatFlash`) leem o `AudioShare`, que é
  estado **externo** publicado por outra thread — continuam corretos como estão; a regra é
  sobre estado *interno ao efeito*. Também fora: `Timeline`/`SectionClip`, que compõem
  efeitos e não são efeitos-folha.
