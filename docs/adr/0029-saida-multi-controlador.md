# ADR-0029 — Saída multi-controlador: N nós, um mapa, um só caminho

- **Estado:** aceito
- **Data:** 2026-08-14
- **Decide sobre:** `led-daemon-bin` (camada de saída). **Não** toca `led-daemon`, `led-core` nem o IPC v1.

## Contexto e problema

O daemon acende **um** controlador: `Config.output` é `Option<String>`, um só alvo. O rig
real são **cinco** WLED (`192.168.2.156–160`, 6.200 px). Hoje o daemon acende **720 px de
6.200** — 12 % do palco.

**E o enunciado "multi-controlador não existe" está errado.** A inspecção antes de decidir
mostrou que existe, mas só num dos dois caminhos — e não é o que o hardware validou:

| Caminho | Fan-out para N nós | Porquê |
|---|---|---|
| Art-Net / sACN | **já existe** | `Hal::new(layout, vec![dev])`; o `Hal` guarda `Vec<Arc<dyn DeviceDriver>>` e o `CompiledLayout` mapeia píxel→device. O daemon passa um vector **de um**. |
| DDP | **não existe** | `DdpOutput` contorna o `Hal` por decisão deliberada (2026-07-09d: *"pixel-nativo, sem mapa de universos"*). Sem `Hal`, não há fan-out. |

E o `RigBuilder` (`led-layout/src/rig.rs`) já constrói N instâncias de um template com
endereçamento **livre de conflito por construção**, testado à escala real (86 strips × 5
robôs = 8.600 px).

## Decisão

### 1 · O DDP leva o multi-controlador **primeiro**, apesar de ser o caminho mais caro

O barato seria ligar o `Hal` a N devices: Art-Net ganhava cinco nós hoje, com peças já
testadas. **E seria um erro que este repositório já cometeu uma vez.** O achado de
2026-08-07f está escrito:

> *"A correcção óbvia produziria calibração no Art-Net e no sACN e silenciosamente nenhuma
> no DDP — pior que a ausência uniforme de hoje, porque **pareceria feito**."*

Um daemon que acende cinco nós em Art-Net e **um** em DDP, sem nada no ecrã a dizê-lo, é
exactamente essa assimetria. E o DDP é o caminho do GS4.5: 94/94 frames na primeira luz
(2026-07-20), e é o que o preset `esp32-poe-wled-ddp` declara.

Art-Net e sACN entram na **mesma** fatia — não numa seguinte. A regra é: ou os três
protocolos acendem N nós, ou nenhum acende.

### 2 · Um alvo é `(endereço, intervalo de píxeis)`, e o intervalo é **derivado**

O operador declara **endereços**; não declara offsets. Com um `--profile` e N endereços, o
intervalo de cada nó deriva de `max_pixels` do profile e da ordem dos endereços — a mesma
disciplina do `pixels_per_datagram`, que é **derivado** do MTU e não declarado ao lado dele
(GS4.3: *"escrever a mesma verdade duas vezes, e a segunda apodrece em silêncio"*).

O `DdpDevice` já endereça por `pixel_offset` (`ddp.rs:262`), portanto o mecanismo existe.

**Um show maior do que a soma dos nós é recusado na construção**, nunca descoberto no palco
com metade da fita apagada — a mesma regra que o GS4.3 já aplica a um nó.

### 3 · A `Calibration` **não** força emenda ao ADR-0019 — e isso foi verificado, não assumido

A Emenda 1 do ADR-0019 avisa: *"se o multi-controlador chegar com calibrações divergentes
por nó, a emenda tem de ser revisitada"*. Chegou. Mas a premissa não se cumpre:

`HardwareProfile.calibration` (`lib.rs:155`) é do **tipo** de hardware. `device_id`,
`address` e `first_universe` **não estão** no profile — são da instância (ADR-0018), e
`grep` confirma que não existem lá. **Cinco nós do mesmo preset partilham a calibração por
construção**, e uma LUT serve os cinco.

**A consequência é uma regra, não uma omissão: todos os alvos partilham o mesmo
`--profile`.** Um rig de perfis mistos é **recusado**, não silenciosamente mal calibrado. É
a diferença entre não suportar e suportar mal, e o ADR-0019 só precisa de ser revisitado no
dia em que houver razão real para perfis mistos.

### 4 · Fan-out **sequencial**. O ADR-0012 mantém-se adiado.

O ADR-0012 adiou o fan-out paralelo *"até ao 2.º nó físico"*, por anti-overengineering a um
nó. Com cinco declarados a premissa parece mudar — **mas o gatilho era físico, e o rig
continua offline** (os cinco IPs sem resposta, esta máquina nem na rede deles).

Cinco `sendto` sequenciais de ~910 bytes não são obviamente um problema. Afirmar que são —
ou que não são — sem medir contra nós reais seria inventar um número, e é o que o TD-011 e
o TD-012 já ensinaram a não fazer: **medir antes de optimizar, e a medição decidir**.

O paralelo entra quando houver medição que o justifique, com o gatilho onde já está.

### 5 · Um erro de envio a um nó **não** derruba os outros

Um cabo partido no robô 3 não pode apagar os robôs 1, 2, 4 e 5. Cada alvo contabiliza os
seus envios e as suas falhas; o laço continua. É a extensão da regra que já existe para um
nó (*"erro de envio não derruba o laço, e também não inunda o journal"*), e a razão é a
mesma: **show parcial é melhor que palco escuro.**

O que fica **proibido** é o inverso — um nó em silêncio não pode ser indistinguível de um
nó a funcionar. As estatísticas são **por alvo**, nunca somadas numa só.

### 6 · O pré-voo com N alvos: **todos** para a excepção, **qualquer um** para a recusa

Duas regras que com um alvo eram a mesma coisa, e com N deixam de o ser. Ficam escritas
porque substituir `cfg.addr` por `cfg.alvos[0].addr` mecanicamente daria a resposta errada
nas duas, **em silêncio**.

**A excepção do loopback exige `all`.** Hoje: *"um alvo de loopback não atravessa interface
nenhuma, logo o gate do ADR-0005 não se lhe aplica"*. Com N, isso só vale se **todos** forem
loopback. Um rig com quatro nós em loopback e um em `192.168.2.156` atravessa o fio — e um
`any(is_loopback)` desligaria o gate do WiFi para o rig inteiro por causa dos quatro que não
contam. Seria a mutação que o controlo negativo `num_alvo_de_rede_o_wifi_ativo_reprova_mesmo`
já apanhou uma vez, reintroduzida pela porta do lado.

**A presença exige que se sondem todos, e um ausente reprova.** Hoje sonda-se um endereço.
Com N, sondar só o primeiro deixaria quatro nós por verificar — e o RT-003 existe
precisamente contra o palco escuro por controlador ausente. Um nó em silêncio reprova o
pré-voo, mesmo que os outros quatro respondam: **a resposta de um nó nunca mascara o silêncio
de outro**, que é a propriedade que o `presence()` do `led-protocols` já garante e que os
testes de `discovery.rs` já afirmam.

**Consequência para os avisos:** `devices_missing` passa a **nomear quais** faltam, não
apenas que faltam. Com cinco robôs, *"SEM resposta"* sem dizer de quem manda o operador
procurar em cinco sítios.

### 7 · O universo viaja **com o endereço**, e não há omissão silenciosa

*(Emenda de 2026-08-15, escrita antes do código.)*

**O achado que forçou esta decisão.** O daemon **nunca expôs** `first_universe`: está escrito
`1` em `stage.rs`, sem flag. E a única validação de hardware que existe diz o contrário —
2026-07-23, Art-Net contra o WLED do rig: *"`--first-universe 0` (universo 0 alinha com
`dmx.uni:0`; **`1` desloca ~170 px**)"*. O daemon está fixo no valor que a bancada contradiz,
e nenhum teste o apanha porque todos chamam `from_profile` **directamente** com o parâmetro,
sem atravessar o `Stage::open` que fixa o `1`. É a classe do `RgbOrder` do GS4.3, e é
**anterior ao multi-controlador**: existe hoje, com um nó.

**A sintaxe.** O universo é dado da **instância**, tal como o endereço — o `HardwareProfile`
tem `pixels_per_universe` mas não tem `first_universe`, e o ADR-0018 fixou porquê. Por isso
viajam juntos, num só token por nó:

```
--output IP[:PORTA][@UNIVERSO]
--output 192.168.2.156@0 --output 192.168.2.157@0
--output [::1]:6454@0                      # IPv6 entre colchetes
```

**Porque não duas listas paralelas.** `--output` × N e `--first-universe` × N podem divergir
em comprimento e em ordem, e o operador só descobre no palco. Com um token por nó a
divergência **não é representável** — o mesmo tipo de garantia que escolheu SSE em vez de
WebSocket (ADR-0026 §5). `@` não colide com a sintaxe de IPv6 nem com `proto://`.

**Não há omissão silenciosa.** O protocolo vem do profile, logo o daemon sabe se ele usa
universos, e **recusa em vez de adivinhar**:

| Protocolo | `@` ausente | `@` presente |
|---|---|---|
| Art-Net / sACN | **erro** — o preset usa universos, declare `@N` | honrado, **se couber na faixa** |
| DDP | aceite — endereça por byte | **erro** — o DDP ignora universos |

Escrever `@5` num alvo DDP significa que o operador julga que aquilo tem efeito; aceitar em
silêncio confirmaria a crença errada. É a decisão do GS4.4 outra vez: *"um valor errado por
omissão é pior que a ausência de valor, porque parece configuração"*.

#### 7.1 · A sintaxe é comum; a **semântica não é**. A faixa é do protocolo.

*(Emenda de 2026-08-15, escrita antes do código, a corrigir a própria §7.)*

A primeira versão desta secção tratou *"usa universos"* como um **booleano**, e isso colapsou
Art-Net e sACN como se partilhassem a mesma faixa. **Não partilham** — e o defeito que isso
produziu não foi teórico: o teste da matriz passou a **afirmar que `@0` é válido para sACN**,
fixando como esperado um valor que a E1.31 não define. É a classe de 2026-08-07f, e desta vez
estava dentro do teste escrito para provar a regra.

**Medido no código deste repositório, não presumido:**

| Protocolo | Faixa | Onde se lê | Zero é válido? |
|---|---|---|---|
| **Art-Net** | `0 … 32767` | `artnet.rs:277` — `((universe >> 8) & 0x7F)`, port-address de **15 bits** | **sim** |
| **sACN (E1.31)** | `1 … 63999` | `packet.rs:145` — *"universe field round-trips 1..=63999"*, teste percorre `[1, 512, 1024, 32768, 63999]` | **não** |
| **DDP** | — | endereça por byte | não se aplica |

O `1` do sACN não é convenção nossa: é a E1.31, e o repositório **já o sabia** — o comentário
e o vector de teste começam em 1 desde que o `packet.rs` existe. Nunca virou validação. Há um
segundo sintoma independente: `device.rs:42` deriva `239.255.hi.lo`, e o universo 0 daria
**239.255.0.0**, que não é um grupo E1.31 válido.

**Uma fonte, não duas.** `faixa_de_universos() -> Option<(u16, u16)>` é a autoridade;
`usa_universos()` passa a ser `faixa_de_universos().is_some()`. Um booleano ao lado de uma
faixa seriam duas verdades sobre a mesma coisa, e a segunda apodreceria — a regra do GS4.3.

**Matriz normativa**, a testar por protocolo e nas duas fronteiras:

| Protocolo | Aceita | Recusa |
|---|---|---|
| Art-Net | `0`, `32767` | `32768` |
| sACN | `1`, `63999` | `0`, `64000` |
| DDP | — | **qualquer** `@` |

**Registado e deliberadamente FORA desta fatia:** `build_art_dmx` **mascara** um universo
acima de 32767 em vez de o recusar — `40000` vira `7232` sem uma palavra. É *fail-closed*
violado, vive em `led-protocols`, e afecta qualquer chamador. A faixa acima apanha-o **na
fronteira do daemon**; corrigi-lo na origem é fatia própria e ADR próprio.

**O que isto NÃO decide.** Se dois nós devem usar o mesmo universo (modelo WLED, um espaço
por controlador) ou universos contíguos (modelo xLights, 1–149 pelos cinco robôs). Com esta
sintaxe **o operador declara**, e as duas convenções são exprimíveis sem que o daemon
escolha por ele. Continua sem verificação em hardware: o rig está offline.

### 8 · O estado por alvo chega ao operador pelo `status`, **sem segunda superfície**

*(Emenda de 2026-08-15.)*

A decisão 5 exige que a perda por nó seja observável. `por_alvo()` existe e **não tem
consumidor** — é a forma do TD-014 outra vez: contador com ADR a exigir reporte e nenhum
caminho até ao operador.

**`Snapshot` ganha `outputs: Vec<{addr, frames, errors}>`**, campo **aditivo** na resposta do
`status` que o IPC v1 já produz e que o `/api/state` já repassa verbatim. Lista **vazia**
significa ausência de saída — nunca zeros fabricados, nunca um total somado (a decisão D das
alternativas rejeitadas).

**As duas saídas rejeitadas, e porquê.** Ligar o `led-readmodel` criaria uma **segunda
superfície de métricas** para o mesmo facto. Uma rota `/api/outputs` no console obrigá-lo-ia
a inventar o dado, porque o IPC não o transporta — a mesma parede que mantém o
`/api/profiles` em 501 (ADR-0026 §15). **Não se abre IPC v2**: isto é um campo numa resposta
existente, não um comando novo, e a política está no ADR-0027 §6.

O gate de contrato que já existe apanha o esquecimento: o **caminho B** extrai os campos do
arm `Cmd::Status` do produtor e confronta-os com o TypeScript gerado.

### 9 · O custo do fan-out é **medido antes** de ser optimizado

*(Emenda de 2026-08-15.)*

`OutputManager::send` faz `buf.clone()` e toma um `Mutex` **por alvo, por frame**. A
alocação **não é nova** — o `send()` da calibração já a fazia em código commitado — e
**nenhum gate de `no_alloc` cobre o caminho de saída do daemon**, o que explica que nem uma
nem outra tenham sido apanhadas.

**Não se promete `no_alloc` e não se optimiza agora.** Mede-se o caminho real com 1 e com 5
alvos, com e sem calibração, e regista-se alocações/frame e ns/frame — a disciplina do
TD-011 e do TD-012, que mediram e mandaram **não** optimizar. A asserção do benchmark é
relativa (5 alvos não custam mais que ~5× um), para apanhar desvio superlinear sem inventar
um orçamento.

Qualquer optimização que toque `led-core` — por exemplo `send_frame` a aceitar
`&[PixelColor]` em vez de `LogicalFrame` — é **seam congelado e exige ADR próprio**.

## Alternativas rejeitadas

**A · Art-Net primeiro, DDP depois.** Rejeitada: cria a assimetria silenciosa que o achado
de 2026-08-07f nomeou, no caminho que o hardware validou.

**B · Dar ao `DdpOutput` um `Hal` próprio.** Rejeitada: o DDP é pixel-nativo e não tem mapa
de universos; embrulhá-lo no `Hal` obrigaria a inventar universos que o protocolo não tem.

**C · O operador declara offsets à mão.** Rejeitada: é a mesma verdade duas vezes — o
profile já diz `max_pixels`, e um offset escrito à mão diverge dele no primeiro dia.

**D · Somar as estatísticas dos N nós.** Rejeitada: `frames_sent` agregado torna um nó morto
indistinguível de um nó vivo, que é a observabilidade a mentir (ADR-0026 §9).

## Consequências

`OutputManager` passa a ter N saídas em vez de uma. A LUT continua **uma**, dobrada uma vez
no arranque, porque o profile é um. `--output` passa a aceitar N valores.

**Aceite e escrito:** nada disto prova que cinco nós físicos acendem sincronizados. Prova
que cinco fluxos de bytes **saem** do daemon com os intervalos certos. A distinção é a
mesma de sempre, e o runbook do GS4.5 continua a ser quem a fecha.

## Critério de reversão

Se surgir razão real para perfis mistos no mesmo rig, a decisão 3 cai e o ADR-0019 tem de
ser revisitado. Se a medição com nós reais mostrar que o envio sequencial não cabe no
orçamento de tick, a decisão 4 cai e o ADR-0012 é retomado — **com o número na mão**.
