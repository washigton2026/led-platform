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
