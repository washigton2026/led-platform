# ADR-0030 — Portas físicas: a porta é subdivisão de **endereçamento**, e a repartição tem um só dono

- **Estado:** 🟢 **implementado** — o contrato foi congelado **antes** do código, e o código
  chegou depois dele, na ordem que este ADR fixou.
- **Data:** 2026-08-30 · **Implementado:** 2026-09-01 (FASE C, `6617794`..`24c5b78`)
- **Decide sobre:** `led-hardware-profile` (schema + compilação) e quem o consome
  (`led-player`, `led-daemon-bin`). **Não** toca `led-core`, a calibração, o IPC v1 nem os
  protocolos.

**O que «implementado» afirma.** Que as decisões deste ADR têm código e teste discriminante:
`Capabilities.ports` (§5) em `crates/led-hardware-profile/src/lib.rs:126`; o **dono único** da
aritmética (§6) em `crates/led-hardware-profile/src/reparticao.rs:95`; o alinhamento de cada
porta a uma fronteira de universo (§4-bis) em `crates/led-hardware-profile/src/compile.rs:133`;
e os **dois** consumidores a pedirem o endereçamento ao dono (§8) — o daemon por
`compile_layout_de` (`crates/led-daemon-bin/src/output.rs:593`) e o `led-player --profile` por
`compile_layout` (`crates/led-player/src/main.rs:333`). `led-core` ficou intocado (§9).

**O que «implementado» NÃO afirma — e nenhuma das duas é implementação por acabar:**

1. **A pendência do §5 continua por decidir.** `max_pixels % ports != 0` não tem política
   normativa, e isso é **deliberado**. Verificado, não presumido: `validate.rs` emite
   `Finding::NoPhysicalPorts` para `ports == 0` e **nada** para a divisão inexacta. O gatilho
   nomeado no §5 — *o primeiro preset com divisão inexacta* — **não disparou**: os oito presets
   dividem exacto (seis com `ports: 1`; Falcon `16384/16 = 1024`; Advatek `16320/16 = 1020`).
   Não é uma excepção à implementação: é a decisão que o §5 escolheu **não** tomar, com o
   gatilho por disparar. Implementar uma política por omissão agora está **proibido** pelo
   *Critério de reversão*.
2. **Portas continuam NÃO MEDIDO em hardware**, como as *Consequências* já diziam. Nenhum
   controlador multi-porta foi alguma vez observado; o `16` é folha de catálogo. O
   `raspberry-fpp-sacn` declara `ports: 1` **sem fonte** para a contagem — e com `ports: 1` a
   divisão é exacta **por construção**, o que *esconde* a pendência do §5 em vez de a resolver.
   É o candidato mais provável a disparar o gatilho no dia em que a contagem real for
   estabelecida (`max_pixels: 32_768`).

## Contexto e problema

O `HardwareProfile` modela **uma** porta física por nó. O preset `falcon-f16v3-sacn` declara
`max_pixels: 16_384` e o Falcon F16V3 tem **16** portas — 1024 px cada. O descritor não sabe
dizer isso, e por isso ninguém abaixo dele pode endereçar por porta.

**O que NÃO é o problema, verificado antes de decidir.** `PixelPhysical.format` já é
**por-pixel** desde o ADR-0011, portanto o `CompiledLayout` **já consegue exprimir** portas
com formatos diferentes. Quem achata é só o descritor de design-time. Isto confirma a leitura
do `ROADMAP.md` e é a razão de o `led-core` ficar fora desta decisão.

**O que é o problema, e é maior do que o enunciado.** A inspecção antes de escrever encontrou
que **existem dois caminhos independentes que constroem o mapa**, e que eles **já divergem em
comportamento** — não só em estrutura:

| Caminho | Onde | Quem usa |
|---|---|---|
| `compile_layout(profile, …)` | `led-hardware-profile/src/compile.rs:65` | **só** `led-player/src/main.rs:333` |
| `CompiledLayout::compile(&assigns)` inline | `led-daemon-bin/src/output.rs:719,730` | o daemon (Art-Net/sACN) |
| `DdpOutput::with_limits(…)` sem mapa | `led-daemon-bin/src/output.rs:700` | o daemon (DDP) |

O slice C2 do `ROADMAP.md` diz *«`compile_layout` distribui por porta»*. Executado à letra,
isso daria **portas no `led-player` e nenhuma porta no daemon** — que é o binário do GS4.5, do
console e do rig de cinco nós. É a classe de defeito que este repositório já pagou três vezes:
o `RgbOrder` da GS4.3, o MTU da GS4.4 e a calibração de 2026-08-07f, cuja nota é o aviso
exacto de que precisamos aqui — *«pior que a ausência uniforme, porque pareceria feito»*.

## Decisão

### 1 · A porta é subdivisão de **endereçamento**, não uma camada de capacidade

Uma `Port` responde a **onde os píxeis desta fatia entram no nó**. Não responde a *como são
corrigidos* nem a *que formato de cor têm*. É o mesmo eixo do `Alvo` do ADR-0029, um nível
abaixo: o `Alvo` reparte o **show** entre nós; a `Port` reparte a **fatia de um nó** entre as
saídas físicas desse nó.

### 2 · `color` **não** entra na porta

O `ROADMAP.md` propõe `Port { index, pixel_count, color, calibration }`. O `color` fica fora,
e a razão não é de gosto:

- `Capabilities.color` é **uma** por profile, e o profile descreve um **tipo** de hardware
  (ADR-0018). Um segundo `color` por porta criaria duas fontes para a mesma pergunta;
- a capacidade técnica de portas heterogéneas **já existe** noutro sítio —
  `PixelPhysical.format` é por-pixel (ADR-0011) — portanto declará-la na porta não desbloqueia
  nada que o `CompiledLayout` não faça já;
- e não há **um único preset** no catálogo que precise disso hoje. Modelar heterogeneidade de
  cor sem hardware que a exija é inventar um requisito.

Se um controlador real exigir RGB numa porta e RGBW noutra, isso é um ADR próprio, com o
preset na mão. Fica no *Não-escopo*, nomeado.

### 3 · `calibration` **não** entra na porta — e isto é o ponto que mais restringe

O ADR-0029 §3 decidiu que a `Calibration` fica **fora** do `Alvo`, e a ausência **é** a
decisão: *«pô-la ali sugeriria que pode divergir por nó — e aí o ADR-0019 teria de ser
revisitado»*.

O §9 fechou isso **por medição**, não por argumento. `crates/led-daemon-bin/tests/custo_do_fanout.rs`
afirma **1 alocação com 1 alvo e 1 com 5** — se alguém mover a calibração para dentro do laço
por nó, o segundo número passa a 5 e o gate reprova. A `Calibration` vive no `OutputConfig`
(`output.rs:260`), uma por saída, aplicada **uma vez antes do fan-out**.

Uma calibração por porta faria a LUT deixar de ser aplicável uma só vez. **Reprovaria esse
gate por construção.** Não é detalhe de implementação: é reabrir o ADR-0019 Emenda 1.

**A porta vive abaixo do ponto de calibração.** A repartição por porta é uma propriedade do
**mapa** (design-time); a calibração é uma transformação do **quadro** (uma vez, antes do
fan-out). São camadas diferentes, e a porta não atravessa a fronteira.

### 4 · O tamanho da porta é **derivado, nunca declarado**

Precedente directo, e é citação do próprio `Alvo` (`output.rs:157-163`):

> *«**Derivado, nunca declarado** — o operador dá endereços, e a repartição sai do `max_pixels`
> do profile. É a mesma disciplina do `pixels_per_datagram`, que deriva do MTU em vez de viver
> escrito ao lado dele: a mesma verdade em dois sítios apodrece no segundo.»*

Portanto o profile declara **quantas portas o hardware tem**, e mais nada. A capacidade por
porta é `max_pixels / ports`; o número de píxeis que cada porta **recebe** sai da repartição do
show, exactamente como o `Alvo`.

### 4-bis · Uma porta começa **sempre** numa fronteira de universo, e nunca a partilha

*(Decisão do operador, 2026-08-30. Fecha a ambiguidade que a análise de implementação encontrou.)*

A restrição é sobre **inícios**, não sobre tamanhos:

1. `universe_start(porta)` é sempre um universo inteiro, **canal 0**.
2. Uma porta **pode** terminar dentro do seu último universo.
3. Esse universo **não é partilhado** com a porta seguinte.
4. A porta seguinte começa no **universo seguinte**, canal 0.
5. O espaço não utilizado do último universo de uma porta fica **explicitamente sem atribuição**.

```
universos_por_porta  = ceil(capacidade_por_porta / pixels_per_universe)
universe_start(k)    = first_universe + k × universos_por_porta
pixel_offset(k)      = k × capacidade_por_porta
universe_start(0)    = first_universe
```

**O `ceil` aqui é o que impede a partilha, não o que a causa.** A proibição da decisão é contra
um `ceil` que deixasse a porta *k+1* entrar no universo onde a porta *k* acabou; esta fórmula
avança **para lá** dele. Verificado no Falcon: porta 0 leva 1024 px em 7 universos — o universo
6 leva os píxeis 1020–1023 (canais 0–11) e os canais 12–509 ficam sem atribuição; a porta 1
começa no universo 7, canal 0.

**Porque o desperdício de canal é aceitável e o desalinhamento não é.** Controladores de N
portas são configurados com cada porta a arrancar num universo próprio. Uma porta que
começasse a meio de um universo produziria a fita **deslocada sem erro nenhum** — exactamente
o que a bancada de 2026-07-23 registou sobre o universo errado. Canais por usar não acendem
nada; canais desalinhados acendem a coisa errada.

### 4-ter · Uma porta **sem píxeis** é válida, e não invalida o profile

*(Decisão do operador, 2026-08-30.)*

Um show mais pequeno que o hardware é normal: 16 portas físicas com um show que só precisa de
6 dá `pixel_count = 0` nas portas 6–15. Elas continuam a existir, o `pixel_offset` continua
**derivado**, não sai tráfego para elas, e o `HardwareProfile` **não** é recusado.

**Isto NÃO é a regra do `Alvo`, e a diferença tem razão.** O `repartir` do ADR-0029 recusa um
nó sem píxeis, e a justificação está escrita: *«cinco endereços para um show que cabe em dois
significa que o operador está enganado»*. Ali o operador **escolheu** os endereços. Aqui não
escolheu nada — `ports` vem do hardware. Medido: com a regra do nó aplicada a portas, um Falcon
de 16 portas exigiria um show de **≥ 15 301 px** para arrancar, e o show real do rig (6 200 px)
seria **recusado**.

*O profile descreve o hardware físico; o show determina quanto dele é usado.*

### 5 · `ports` entra em `Capabilities`, e a decisão 5 do ADR-0018 fica **intacta**

O ADR-0018 decisão 5 diz: *«`Capabilities` só contém capacidades declarativas ou booleanas.
Limites numéricos de pixel vivem **apenas** em `Limits`.»*

`ports` é uma **capacidade declarativa estrutural** — quantas saídas o hardware físico tem —
e **não é um limite de píxeis**. Por isso vive em `Capabilities`, e **nenhum campo novo entra
em `Limits`**. `max_pixels` continua o único lar do tecto de píxeis do nó, e a capacidade por
porta é derivada dele.

Esta é a razão de a emenda ao ADR-0018 ser mínima: não há segunda fonte de verdade a criar.

**O que esta escolha NÃO consegue exprimir, dito em vez de escondido:** portas
**não-uniformes** (uma porta com tecto diferente das outras) e um tecto total **abaixo** de
`ports × capacidade_por_porta` (limite de largura de banda agregada). Nenhum preset do catálogo
precisa disso, e o único dado real que temos — Falcon F16V3, 16 384 / 16 = 1024 — é uniforme e
fecha exactamente. **Critério de reversão nomeado abaixo.**

**PENDÊNCIA REGISTADA — `max_pixels % ports != 0`.** Quando a divisão não for exacta, sobra
capacidade **declarada e não endereçável**: com `max_pixels = 1500` e 16 portas, `16 × 93 =
1488` e 12 px do tecto ficam inalcançáveis. **O repositório não tem política normativa para
isto** — verificado, não presumido: `validate.rs` nunca cruza `max_pixels` com
`pixels_per_universe` (só a guarda de zero, `:157`), e **6 dos 8 presets** têm `max_pixels`
que não é múltiplo de `pixels_per_universe`, portanto a prática corrente é tratar `max_pixels`
como tecto físico bruto. Recusar (`Error`), avisar (`Warning`) ou não emitir achado nenhum são
três respostas com consequências diferentes, e **nenhuma está decidida**.

Não é alcançável hoje: os dois únicos presets candidatos a multi-porta dividem exacto
(Falcon `16384/16 = 1024`; Advatek `16320/16 = 1020`). **Gatilho:** o primeiro preset com
`max_pixels % ports != 0`. Até lá fica registada, não inventada.

### 6 · A repartição tem **um só dono**, e ele é o `led-hardware-profile`

Há hoje duas repartições possíveis e elas não podem virar duas implementações da mesma regra:

```
show ──repartir()──▶ Alvo{pixel_offset, pixel_count}     [ADR-0029, hoje em output.rs:201]
                          │
                          └──repartir()──▶ Port{index, pixel_offset, pixel_count}   [este ADR]
```

**É o mesmo *núcleo*, aplicado duas vezes — mas não a mesma função, e a diferença é
comportamental.** O cálculo dos intervalos (`inicio = i × tecto`, `conta = min(total − inicio,
tecto)`) é idêntico nos dois níveis. Duas regras **não** transferem, e as §§4-bis/4-ter dizem
porquê:

| | Nó (ADR-0029) | Porta (este ADR) |
|---|---|---|
| Unidade sem píxeis | **recusa** — o operador escolheu os endereços | **válida** — `ports` vem do hardware (§4-ter) |
| Fronteira de universo | não se aplica — cada nó tem espaço próprio | **obrigatória** — a porta arranca em canal 0 (§4-bis) |

Portanto a implementação **parametriza** o núcleo (a política do zero entra como dado, e o
alinhamento é uma camada por cima), em vez de o chamar tal como está. Copiá-lo com as regras
trocadas seria a segunda implementação que este § existe para impedir; chamá-lo sem parametrizar
recusaria hardware normal.

A regra passa a ter **uma** casa, e ela é o crate leaf: é dado puro, design-time, sem I/O, e o
`led-hardware-profile` já é o sítio onde o profile compila e desaparece (ADR-0018 decisão 7).

Consequência para a implementação: o `repartir` privado do `led-daemon-bin` (`output.rs:201`)
passa a **chamador**, não a segunda implementação. **Este ADR não faz esse movimento** — é
trabalho da FASE C, e está registado como obrigação, não como feito.

> **FASE C (2026-09-01) — obrigação cumprida.** O núcleo vive em
> `crates/led-hardware-profile/src/reparticao.rs:95` e o `repartir` do daemon
> (`crates/led-daemon-bin/src/output.rs:210`) é hoje um **chamador** que escolhe a política do
> nó (`UnidadeVazia::Recusa`) e traduz o erro para o vocabulário do daemon. A prova foi uma
> **mutação cruzada**: mutar o núcleo reprova testes do `led-daemon-bin` — se o daemon tivesse
> cópia própria, teriam passado.

### 7 · Onde a repartição acontece: **uma vez, no arranque**

`HardwareProfile → CompiledLayout + DriverConfig → Runtime`, e depois o profile desaparece
(ADR-0018 decisão 7). A porta **nunca** é consultada durante a renderização, nunca por quadro,
nunca no caminho `Show → Logical Pixels → ProtocolOutput → HAL → DeviceDriver`. O gate de
alocação do hot-path continua a valer sem alteração.

### 8 · Os dois consumidores têm de partilhar a semântica, e isso é um gate

`led-player` e `led-daemon-bin` **não podem** acabar com semânticas diferentes de porta. Como o
daemon hoje **não chama** `compile_layout`, a implementação da FASE C tem de o fazer passar a
chamá-lo — ou a porta existirá só num binário.

Isto é a obrigação central da implementação, e o teste que a guarda está listado em
*Invariantes que precisam de teste novo*.

> **FASE C (2026-09-01) — obrigação cumprida, e o TD-019 fechou no fio.** Os dois binários
> pedem o endereçamento ao `led-hardware-profile`: o daemon por `compile_layout_de`
> (`crates/led-daemon-bin/src/output.rs:593`) e o `led-player --profile` por `compile_layout`
> (`crates/led-player/src/main.rs:333`). Os arms Art-Net e sACN do daemon deixaram de usar
> `led_player::linear_assignments`, e com eles saíram o `170` e o `× 3` escritos à mão.
>
> **A fronteira exacta, para não ser arredondada:** o `led-player` **sem** `--profile` continua
> no caminho histórico (`linear_assignments`, `170` px/universo, `RgbOrder::Rgb`,
> `crates/led-player/src/main.rs:305`). Isso **não** é uma segunda semântica de porta — sem
> profile não há porta nenhuma a exprimir, e o daemon **não** arranca sem `--profile` desde a
> GS4.4. Logo não existe configuração em que os dois binários tenham profile e discordem. Fica
> nomeado porque é o resíduo do TD-019 que o próprio TD-019 não descreve.

### 9 · Zero mudança em `led-core`

`PixelPhysical`, `CompiledLayout`, `UniverseData`, `ProtocolOutput`, `DeviceDriver`, `IDevice`
intocados. Sem bump de `LED_CORE_CONTRACT_VERSION`. A porta **alimenta** a construção do
`CompiledLayout`; não altera a sua assinatura. Verificado: `PixelPhysical{device, universe,
channel, format}` já exprime tudo o que uma porta precisa de produzir.

## Como a porta se relaciona com o que já existe

| Conceito | Origem | Relação com `Port` |
|---|---|---|
| `Alvo` | ADR-0029 | **Nível acima.** Um `Alvo` é um nó; uma `Port` é uma saída dentro dele. Mesma forma, mesma regra. |
| `pixel_offset` | ADR-0029, derivado | A porta tem o seu, derivado da mesma maneira. **Nunca declarado.** |
| `pixel_count` | ADR-0029, derivado | Idem. Sai da repartição, não do profile. |
| `first_universe` | instância (ADR-0018) | Continua **da instância**, não do profile. É o universo da **porta 0**; as seguintes derivam por `+ k × universos_por_porta` (§4-bis). Em DDP desloca o byte. |
| `max_pixels` | `Limits` (ADR-0018) | **Inalterado.** Continua o tecto do **nó**. A capacidade por porta deriva dele. |
| `pixels_per_universe` | `Limits` (ADR-0018) | **Inalterado e agora obrigatório de honrar por porta** — ver a dívida DL-2 abaixo. |
| `Calibration` | `OutputConfig` (ADR-0019 Em.1) | **Acima da porta.** Aplicada uma vez, antes do fan-out. A porta não a vê. |

## Alternativas rejeitadas

- **`Port { index, pixel_count, color, calibration }` como o `ROADMAP.md` propõe** — o
  `calibration` reprova um gate medido (§3) e o `pixel_count` declarado contradiz a disciplina
  do `Alvo` (§4). Rejeitada por evidência, não por preferência.
- **Um `max_pixels_per_port` declarado ao lado de `max_pixels`** — seria a mesma verdade em dois
  sítios para todo o hardware uniforme, e o segundo apodreceria. É literalmente o argumento do
  `pixels_per_datagram`/MTU da GS4.3.
- **Portas só no `led-player`, deixando o daemon para depois** — produz a assimetria silenciosa
  do §Contexto. Rejeitada pelo precedente de 2026-08-07f.
- **Resolver os dois caminhos de `CompiledLayout` aqui** — é refactor do daemon, fora deste
  slice. Registado como dívida com fonte normativa declarada (DL-1).

## Dívida arquitetural registada (não corrigida neste ADR)

### DL-1 — dois caminhos independentes constroem o mapa

`compile_layout` (um chamador: `led-player/src/main.rs:333`) e a construção inline do daemon
(`led-daemon-bin/src/output.rs:719,730`). Nenhum ADR nomeia esta divergência; ela é **anterior**
à FASE C.

**Fonte normativa declarada:** depois da implementação da FASE C, `led-hardware-profile` é o
único sítio que decide endereçamento a partir do profile. O daemon consome-o. **Nenhum refactor
do daemon nesta etapa.**

### DL-2 — o caminho Art-Net/sACN do daemon ignora dois campos declarados

Encontrado ao inspeccionar para este ADR. **Lido no código, não executado:**

- `linear_assignments` (`led-player/src/lib.rs:303`) tem `const PX_PER_UNIVERSE: usize = 170`
  **escrito à mão** e calcula `channel = (i % 170) * 3` — ignora o `pixels_per_universe`
  declarado no profile;
- `cfg.rgb_order()` (`output.rs:569-574`) devolve só a `RgbOrder` para
  `ColorFormat::Rgbw(o, _)`, **descartando o RGBW e o `WhiteMode`**; e
  `From<RgbOrder> for ColorFormat` (`led-core/src/types.rs:141-146`) devolve sempre
  `ColorFormat::Rgb(o)` — **três canais**.

O preset `generic-sk6812-rgbw-sacn` declara `pixels_per_universe: 128` e cor RGBW. Pelo caminho
Art-Net/sACN do daemon sairia **RGB a 170 px/universo**. O caminho DDP não sofre disto — usa
`cfg.color` directamente.

É a mesma família do `RgbOrder` da GS4.3 e do MTU da GS4.4: campo declarado que o fio ignora. E
o gate `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida` não o apanha porque lê
`output.rs`, `stage.rs` e `run.rs` — **não** `led-player/src/lib.rs`.

**Não corrigido aqui**, por três razões: é anterior à FASE C, é código de produção (proibido
nesta etapa), e a correcção certa é consequência do §6 — quando a repartição tiver um só dono,
o 170 escrito à mão deixa de ter onde viver. Registado para entrar no ledger de dívida técnica.

### DL-3 — o `ROADMAP.md` está incompatível com esta decisão

O slice C1 propõe `color` e `calibration` na porta; C2 aponta `compile_layout` como se fosse o
caminho do daemon. Ambos contrariam este ADR. **O `ROADMAP.md` não foi alterado** (proibido
nesta etapa) — a inconsistência fica aqui, para ser reconciliada por quem tiver autorização.

## Invariantes que precisam de teste novo na implementação

> **Estado após a FASE C (2026-09-01): 8 dos 11 têm teste discriminante; 3 não têm, e a razão
> de cada um está escrita.** Cobertos: **1** (`repartir_por_nos_e_depois_por_portas_e_uma_so_regra`),
> **3** e **5** (`ports_vive_em_capabilities_e_limits_nao_ganhou_nada` — destruturação
> exaustiva: um campo novo deixa de **compilar**, `E0027`), **6**
> (`o_sacn_rgbw_honra_os_128_px_por_universo_e_os_quatro_canais`, com `ppu: 128 ≠ 170`), **8**
> (SemVer Guardian), **9** (`falcon_reparte_1024_por_porta_e_arranca_de_7_em_7_universos`),
> **10** (`duas_portas_nunca_partilham_universo`), **11**
> (`porta_sem_pixeis_e_valida_e_o_show_real_do_rig_compila`).
>
> **Não cobertos, e o que isso significa em cada caso:**
>
> - **2 — nenhum teste compara os dois binários.** A propriedade está garantida por
>   **construção**, que é mais forte do que a comparação que este item pedia: desde o §8 há um
>   só dono do endereçamento e ambos o chamam. Um teste de comparação continua a valer a pena
>   no dia em que houver um segundo caminho; hoje não há um para comparar.
> - **4 — `custo_do_fanout.rs` NÃO foi estendido com portas.** O item chamava-lhe *"o mais
>   importante"*, e a razão de não ser um vão é o §7: a porta é resolvida **no arranque** e
>   nunca é consultada por quadro, portanto não pode acrescentar alocação por porta ao laço de
>   fan-out. O gate existente (1 alocação com 1 alvo, 1 com 5) continua a guardar o ADR-0019
>   Emenda 1 sem alteração.
> - **7 — bytes no fio, por porta: NÃO coberto.** Os testes de fio (`wled_driver.rs`) usam
>   **apenas presets com `ports: 1`** (os quatro: três `esp32-*` e o
>   `generic-sk6812-rgbw-sacn`), portanto provam ordem de canais, MTU e
>   universos consecutivos — **não** que duas portas escrevem intervalos disjuntos. Essa
>   propriedade está provada ao nível da **aritmética** (invariantes 9 e 10), não ao nível do
>   **byte**. Fecha quando houver um nó multi-porta no rig, e é a mesma medição que o
>   *NÃO MEDIDO* das *Consequências* exige.

Listados como obrigação da implementação:

1. **Uma só regra de repartição** — o resultado de repartir por nós e depois por portas tem de
   ser o mesmo produzido por uma chamada, com N = nós × portas. Um teste que compare os dois
   caminhos falha se alguém escrever a segunda implementação.
2. **`led-player` e `led-daemon-bin` produzem o mesmo endereçamento** para o mesmo profile e o
   mesmo show. É o gate do §8; sem ele a porta pode existir num binário só.
3. **A porta não declara tamanho** — um teste que rejeite um profile onde o tamanho da porta
   seja um campo de entrada, fixando o §4.
4. **A calibração continua a alocar uma vez** — `custo_do_fanout.rs` estendido com portas: o
   delta tem de continuar **1**, não `n_portas`. É o gate do §3, e é o mais importante.
5. **Controlo negativo do §2** — um profile com portas continua a ter **um** `color`; se alguém
   acrescentar cor por porta, reprova.
6. **`pixels_per_universe` honrado por porta** — o discriminante tem de usar um preset com
   valor ≠ 170 (existe: `generic-sk6812-rgbw-sacn`, 128), senão passa sem provar nada.
7. **Bytes no fio, por porta** — na disciplina do `wled_driver.rs`: portas diferentes escrevem
   offsets diferentes, e duas portas nunca escrevem o mesmo intervalo.
8. **`led-core` intocado** — o SemVer Guardian já o faz; a implementação não pode produzir bump.
9. **Toda porta arranca em canal 0** (§4-bis) — para cada `k`, `universe_start(k)` tem de ser
   múltiplo inteiro de universo a partir de `first_universe`. O discriminante obrigatório é o
   **Falcon**, onde `1024 / 170` **não** é inteiro: com um preset de divisão exacta o teste
   passaria sem provar nada.
10. **Duas portas nunca partilham universo** (§4-bis) — controlo negativo do anterior: trocar o
    `ceil` por divisão inteira faz a porta 1 entrar no universo 6 do Falcon, e o teste reprova.
11. **Porta vazia é válida** (§4-ter) — um show de 6 200 px num nó de 16 portas **compila**.
    Falsificação: aplicar a regra do zero do `Alvo` faz este caso recusar, e o teste apanha-o.

## Ficheiros potencialmente afectados na implementação futura

| Crate | Ficheiros |
|---|---|
| `led-hardware-profile` | `lib.rs` (schema: `Capabilities.ports`), `presets.rs` (C3 — dado), `validate.rs`, `compile.rs` |
| `led-daemon-bin` | `output.rs` (**o caminho real**), possivelmente `run.rs` |
| `led-player` | `main.rs:333`, `lib.rs` (`linear_assignments`, ver DL-2) |
| `led-core` | **nenhum** |

**18 ficheiros** tocam `compile_layout`/`pixels_per_universe`/`max_pixels`. Os que congelam
comportamento e exigem reexame um a um: `wled_driver.rs`, `custo_do_fanout.rs`,
`calibration_path.rs`, `e2e_output.rs`, `profile_validation.rs`.

## Decisões que este ADR **não** toma

- **Não** decide portas heterogéneas em cor (§2, *Não-escopo*).
- **Não** decide portas não-uniformes em capacidade (§5, com critério de reversão).
- **Não** decide como o operador escolhe a porta na CLI — superfície, e não há requisito.
- **Não** resolve DL-1 nem DL-2 (registados, com fonte normativa declarada).
- **Não** altera o `ROADMAP.md` (DL-3).
- **Não** reabre o ADR-0019 nem o ADR-0029 — este ADR foi escrito para os **preservar**.

## Consequências

**Boas.** O Falcon F16V3 passa a ser descritível como é; a repartição ganha um só dono, o que
torna DL-2 corrigível em vez de eterno; e a decisão fecha sem tocar `led-core`, calibração ou
protocolos.

**Custo aceite.** A implementação obriga o daemon a passar a consumir `led-hardware-profile`
para endereçamento — mudança real num ficheiro que guarda o caminho validado em hardware. Não é
edição cirúrgica, e o §8 existe precisamente para que não seja feita pela metade.

**Não entrega nada físico.** Nenhum controlador multi-porta foi testado; o único dado real é a
folha do Falcon F16V3. Portas continuam **NÃO MEDIDO** em hardware até haver um nó com mais de
uma saída no rig.

## Critério de reversão

- Se aparecer um controlador com portas **não-uniformes** ou com tecto total abaixo de
  `ports × capacidade_por_porta`, o §5 tem de ser revisitado — e a mudança é aditiva
  (`Limits` ganha o campo que hoje é derivado).
- Se aparecer um preset com **`max_pixels % ports != 0`**, a pendência do §5 deixa de ser
  teórica e exige decisão antes de o preset entrar no catálogo. **Não implementar uma política
  por omissão nessa altura** — é a decisão que este ADR deliberadamente não toma.
- Se um controlador real configurar as portas **sem** as alinhar a universos (fluxo contíguo
  repartido internamente), o §4-bis cai e a porta deixa de ser subdivisão de endereçamento —
  o §1 vai atrás. É o critério que exige medição contra hardware multi-porta, hoje ausente.
- Se aparecer um controlador que exija formatos de cor diferentes por porta, o §2 cai e nasce
  ADR próprio — o `CompiledLayout` já o suporta, portanto o custo é de schema, não de seams.
- Se a medição mostrar que a repartição em dois níveis custa alocação no arranque acima do
  orçamento, o §6 mantém-se e o que muda é a estratégia de construção, não o dono da regra.
