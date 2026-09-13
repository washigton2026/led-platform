# ADR-0015 — Caminho de dados do preview (cópia lossy fora do hot-path)

- **Status:** aceito (pré-implementação)
- **Data original:** 2026-07-26
- **Fonte:** Decisão de arquitetura UI/Preview + invariante do triple buffer (ADR-0008)

## Contexto e problema
O preview ao vivo precisa mostrar os pixels que o engine está renderizando. O caminho
render→send é um **triple buffer lock-free** cuja invariante de segurança é "render e send
**nunca** compartilham buffer" (ADR-0008). Se a UI (ou um publicador de preview) **ler o
triple buffer** ou impuser backpressure, quebra o isolamento e ameaça o jitter do output.

## Decisão
O preview é alimentado por uma **cópia separada, downsampled, rate-limited e lossy**,
publicada pelo daemon **fora do hot-path**. A UI **nunca** lê o triple buffer e **nunca** faz
backpressure no engine. Se o consumidor de preview está lento, frames de preview são
**descartados** (best-effort), sem afetar render/send.

## Escopo / Não-escopo
- **Escopo:** contrato de que o preview é uma via read-only, lossy, downsampled, desacoplada
  do hot-path.
- **Não-escopo:** o algoritmo exato de downsample/LOD; renderização 2D/3D no cliente
  (WebGPU, ADR-0016); a taxa numérica final (a medir).

## Alternativas descartadas
- **Tap direto no triple buffer** — proibido; quebra a invariante do ADR-0008.
- **Preview síncrono/backpressured** — ameaça o jitter do output.

## Limites de segurança
O preview carrega só cor de pixel downsampled — nenhum dado sensível de rede/controle.
Trafega pelo mesmo canal restrito do ADR-0014.

## Isolamento do hot-path
A publicação de preview é uma **cópia** tomada fora do caminho crítico (não em `send_frame`,
não lendo os slots do triple buffer). Sem alocação no hot-path; sem lock compartilhado com
render/send. Esta é uma **regra de isolamento, não uma otimização**.

## Compatibilidade de OS
Agnóstico: é um stream de dados. O rendering do preview no cliente depende de GPU
(ADR-0016), com fallback (abaixo).

## Degradação segura
Preview lento → descarta frames (lossy). GPU do cliente indisponível → preview cai para
2D/CPU ou desliga com aviso; o engine **já** é `gpu`-gated com fallback CPU
(`AutoGpuPlasma`), então o **output nunca depende da GPU da UI**. Preview ausente ≠ show
ausente.

## Consequências
**Boas:** preview rico sem risco ao output; escala a rigs grandes via downsample/LOD.
**Ruins/custos:** preview não é pixel-perfeito nem frame-exato (é aproximação lossy) —
aceito conscientemente; custo de CPU/banda da cópia (mitigado por rate-limit/downsample).

## Métricas / gates
Gate: teste/prova de que ativar o preview **não altera** o p99 de output nem introduz
alocação no hot-path (reusa o `no_alloc` + `MetricsEmitter`). Preview alvo ~30 fps,
degradável.

## Emenda 1 (2026-09-13) — o mecanismo é **evento no canal `subscribe` existente**

**Estado:** decisão do operador. Esta emenda preenche **só o mecanismo** que este ADR deixou
aberto em «Critério de reversão» (*«o mecanismo de publicação (canal, downsample) é
substituível»*). **A regra de isolamento não é tocada.**

### O que NÃO muda, e é dito primeiro de propósito

A proibição do tap direto no triple buffer (§«Alternativas descartadas», §«Isolamento do
hot-path») e a ausência de critério de reversão para ela continuam **exactamente** como
estavam. A cópia continua a ter de ser tomada **fora do caminho crítico**, sem alocação no
hot-path e sem lock compartilhado com render/send. Esta emenda escolhe **por onde o dado
viaja**; não afrouxa **onde ele é tomado**.

O gate também não muda: continua a exigir prova de que activar o preview **não altera o p99**
de output nem introduz alocação no hot-path. E ele tem hoje um número a defender —
`crates/led-daemon-bin/tests/custo_do_fanout.rs` mede **0 alocações/frame** no caminho rápido
(1 alvo, offset 0), com asserção que reprova se subir. Qualquer cópia enxertada tem de
preservar esse zero.

### A decisão

O preview é publicado pelo daemon como um **tipo de evento novo no canal `subscribe` já
existente** — daemon → console por UDS (ADR-0014), console → browser por SSE (ADR-0026). É
*push*, lossy, best-effort, com descarte no consumidor lento, como a Decisão original exige.

**Não é canal novo.** Esse caminho já transporta **sete** tipos de evento ponta-a-ponta, e a
sua metade de transporte é **transparente ao payload**: `Fanout::difundir(&str)`
(`crates/led-console-bin/src/fanout.rs:204`) enfileira `String` (`:24`) e **não parseia** o
conteúdo. Honra o §«Limites de segurança» sem excepção: trafega pelo mesmo canal restrito do
ADR-0014.

### Porque isto NÃO exige `PROTOCOL_V = 2` nem a Fatia 1.2 do ADR-0031

Ancorado no **ADR-0027 §6 + Emenda 4**: um tipo de evento novo no canal `subscribe` é
**aditivo** — regenerar e commitar o `.ts`, sem versão nova de protocolo. Não é comando novo no
`enum Cmd`, logo não é a classe «vocabulário» da Emenda 3.

**E é a *forma* que decide, não o preview.** Se o mecanismo escolhido fosse qualquer variante
de *pull* — o console a **pedir** um frame — isso seria um comando novo, portanto vocabulário,
portanto `PROTOCOL_V = 2`, portanto as Fatias 1.2 e 1.3 do ADR-0031 **antes** de qualquer
preview. A escolha de *push* é o que mantém o ADR-0031 fora do caminho crítico desta
funcionalidade. Fica escrito para que ninguém troque a forma mais tarde a pensar que é
detalhe de implementação.

**O custo aditivo é real e não é zero.** O gerador de contrato do console escreve a união
`EventoPayload` membro a membro como literal (`crates/led-console-bin/src/contract.rs:259-263`):
um tipo novo **exige** editar ali e regenerar o `.ts`. É precisamente o *«regenerar e
commitar»* que a classe aditiva prescreve — e o `switch` sem `default` do
`console-web/src/eventos.ts` deixa de compilar até alguém decidir o que mostrar.

### O que esta emenda NÃO resolve — e é o trabalho que resta

**O «fora do hot-path» não existe no daemon hoje.** Medido, não presumido:
`grep -rn "triple\|Triple" crates/led-daemon-bin/src/` devolve **zero**. Este ADR foi escrito
contra o `led-pixel-engine`, onde render e send **já estão desacoplados** por um triple buffer
— existe ali uma costura natural onde tirar a cópia «ao lado». O daemon não tem essa costura:
o caminho é `.lumyx` → `FrameSource::frame_at` (`source.rs:51`) → `Stage::on_tick`
(`stage.rs:85`), e o **único** sítio onde um `LogicalFrame` completo existe é `stage.rs:88-93`,
entre `heartbeat.record(&frame)` (`:92`) e `output.send(&frame)` (`:93`) — **dentro** do tick
de 40 Hz, que é exactamente o que o §«Isolamento do hot-path» proíbe.

Criar essa costura (segunda thread, ou buffer de handoff) é **trabalho de desenho**, não desta
emenda. Esta emenda escolhe o canal; não decide a costura, e não autoriza tomar a cópia dentro
do `on_tick` só porque o canal já está escolhido.

**Continuam em não-escopo, como estavam:** o algoritmo de downsample/LOD e a taxa numérica
final. Consequência aritmética disso: os números de capacidade abaixo são o **pior caso sem
downsample**, e com downsample descem por um factor que ninguém decidiu.

### Capacidade — medida, e não é o bloqueio

Contra o `MAX_LINE` de 64 KiB do IPC v1 (`crates/led-daemon-bin/src/server.rs:35`), com o
custo de base64 que o SSE impõe por ser texto (+33 %):

| rig | raw | base64 | % da linha |
|---|---|---|---|
| 720 px | 2 160 B | 2 880 B | 4 % |
| **6 200 px** (rig real) | 18 600 B | **24 800 B** | **37 %** |
| 7 500 px | 22 500 B | 30 000 B | 45 % |

O frame do rig real **cabe**, a 37 % do tecto. A 40 Hz são ~968 KiB/s sobre UDS. O bloqueio é
a costura fora do hot-path, não a capacidade do canal.

## Critério de reversão
Nenhum para a **regra de isolamento** (tap direto no triple buffer permanece proibido). O
*mecanismo* de publicação (canal, downsample) é substituível se não bater o budget, desde
que preserve lossy + fora-do-hot-path.

**Da Emenda 1:** o mecanismo agora escolhido (evento no `subscribe`) herda esse critério — se
não bater o budget, é substituível. Mas trocá-lo por qualquer forma de *pull* **não é
substituição de mecanismo**: passa a ser comando novo, logo `PROTOCOL_V = 2` e as Fatias 1.2 e
1.3 do ADR-0031 como pré-requisito. Essa troca é decisão nova, com a mesma formalidade.
