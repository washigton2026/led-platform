# ADR-0026 — A fronteira console↔daemon: o console é cliente do IPC v1, e traduz sem interpretar

- **Estado:** aceite
- **Data:** 2026-08-07
- **Relacionados:** ADR-0013 (engine headless, UI é cliente) · ADR-0014 (IPC e segurança) · ADR-0015 (preview lossy) · ADR-0016 (stack do console, **ainda provisório**) · ADR-0017 (blackout, **adiado**) · ADR-0023 (transporte, **congelado** na GS1.6)

## Contexto e problema

O ADR-0013 decidiu que a UI é um **processo separado**, e o ADR-0014 decidiu a **segurança do
canal**. Nenhum dos dois diz **quem fala com o daemon, por onde, e o que acontece quando esse
canal cai**.

Sem essa decisão escrita, a primeira linha de código do console escolheria em silêncio. E a
escolha cómoda — pôr um servidor HTTP dentro do `led-daemon-bin` — violaria o ADR-0013 (*"o
output não partilha processo de falha"*) e desfaria a garantia que o GS3 estabeleceu: *as
threads de ligação nunca tocam o `ShowRuntime`; enfileiram, e o laço aplica*.

## Decisão

```
Browser ──HTTP/SSE──► led-console-bin ──UDS/IPC v1──► led-daemon ──► fio
   (N)                   (1 processo)                 (1 processo)
```

### 1 · O console é **cliente** do IPC v1

Não tem acesso privilegiado ao `ShowRuntime`. Fala o mesmo protocolo v1 que o `ledctl` já
exercita em 14 testes de integração. Se o console e o `ledctl` divergirem, isso é um defeito
visível — não uma superfície nova a apodrecer sozinha.

### 2 · Processo separado (ADR-0013)

Um pânico no parser HTTP mata o console, **não** o show: o daemon continua a ticar e o
heartbeat continua a reenviar o último quadro válido.

### 3 · Duas ligações UDS: comando e eventos

Espelha a separação que o GS3 já fez por dentro. A ligação de eventos **nunca escreve**; a de
comando serializa um pedido de cada vez. Um evento lento não atrasa um `play`.

### 4 · Uma subscrição no daemon, fan-out para N browsers

O console mantém **uma** ligação `subscribe`. Sem isto, cada separador aberto seria um
subscritor no daemon, e a lista de subscritores cresceria com o comportamento do operador em
vez de com a arquitetura.

### 5 · SSE para eventos, POST para comandos — **não** WebSocket

Três razões técnicas:

1. **O fluxo de eventos já é unidirecional e orientado a linhas.** O IPC entrega uma linha
   JSON por evento; o SSE transporta uma linha por evento. Quase 1:1 — nenhuma camada de
   enquadramento nova para errar.
2. **A assimetria é o ponto.** Um canal bidirecional *convida* alguém a enviar comandos por
   ele, criando um **segundo caminho de comando** ao lado do POST. Com SSE isso não é sequer
   representável — e "não representável" é mais forte que "proibido por convenção", tal como
   *"sem TCP, `0.0.0.0` não é representável"* foi mais forte no GS3.
3. **Reconexão vem do browser.** O `EventSource` reconecta com backoff sozinho. Com
   WebSocket, isso seria código nosso, a testar sob queda de rede — o cenário mais difícil de
   testar de todos.

### 6 · Os códigos de erro atravessam **verbatim**

O corpo carrega sempre `{"code":"<código do daemon>","detail":"…"}`. O estado HTTP transporta
**apenas** significado de transporte e **nunca substitui** o código:

| Situação | HTTP | `code` |
|---|---|---|
| Daemon recusou | `409` | `no_show_loaded`, `not_armed`, … |
| JSON malformado | `400` | `bad_request` |
| Laço não respondeu | `504` | `engine_busy` |
| UDS inacessível | `503` | `console.daemon_offline` |

Mapear `refused_by_policy` para 403 e parar aí perderia a razão. Foi para o código significar
o mesmo dos dois lados que o contrato foi congelado na GS1.6.

### 7 · `OFFLINE` é um **estado**, não um erro

Quando o UDS cai, o console **não** apaga o ecrã nem devolve zeros. Devolve o último snapshot
conhecido, marcado com **`stale_ms`**, e o estado `OFFLINE`. Um `frames: 4210` de há dois
minutos apresentado como atual seria a mentira mais fácil desta arquitetura.

### 8 · A cadeia de evidência não colapsa

```
software_sent           ← OutputStats.frames_sent           (sabemos)
network_delivered       ← NOT_MEASURED sem instrumentação   (não sabemos)
controller_received     ← WLED live:true                    (só com hardware)
controller_acknowledged ← WLED lm == protocolo              (só com hardware)
led_verified            ← observação humana                 (nunca automático)
```

O console reporta **até onde a evidência chega** e `NOT_MEASURED` a partir daí. Nunca um
booleano.

### 9 · **OBSERVABILITY ≠ PHYSICAL EVIDENCE**

Esta é a regra que este ADR existe sobretudo para fixar.

`lumyx_frames_total`, `OutputStats.frames_sent` e `frames_sent` do `DeviceStatus` são
**observabilidade operacional**: dizem que o processo tentou e que a chamada local teve
sucesso. Um `sendto` UDP para um destino **inexistente** tem sucesso local — foi exatamente
assim que o `lumyx-hwcheck` se apanhou a si próprio a reportar `PASS` de heartbeat contra um
IP que não existia.

Estas métricas **não constituem prova** de `network_delivered`, `controller_received`,
`controller_acknowledged` nem `led_verified`.

**A UI nunca pode apresentar uma métrica local como prova física.** Um contador a crescer é o
dado mais tentador de mostrar como "está a funcionar", e é o mais local de todos.

### 10 · Loopback-only enquanto o ADR-0014 não der auth

O console **recusa bind não-loopback**, como o `led-readmodel` já faz. O `ClientRegistry` do
ADR-0014 está declarado e **vazio**; enquanto estiver, `0.0.0.0` não é uma opção de
configuração — é uma recusa. Sem elevação de privilégio: o console corre como o mesmo
utilizador, e o socket do daemon continua `0o600`.

### 11 · Limites herdados do GS3, pelas mesmas razões

**64 KiB** por corpo (`MAX_LINE` — sem ele, um cliente que nunca feche cresce sem limite) e
**profundidade JSON 16** (`MAX_DEPTH` — `[[[[[…` estoura a pilha, e um cliente derrubaria o
processo com uma linha de texto). Teto explícito de ligações SSE.

### 12 · Timeout HTTP **derivado** do `REPLY_TIMEOUT`

O daemon desiste de esperar pelo laço em `REPLY_TIMEOUT`. O timeout HTTP tem de ser
**estritamente maior**: se fosse menor ou igual, o browser receberia "falhou" enquanto o
daemon ainda aplicaria o comando — e o operador veria o show mudar depois de a UI ter dito que
não mudou.

`HTTP_TIMEOUT = REPLY_TIMEOUT + MARGEM`, com a margem nomeada. **Nunca um segundo número
escrito à mão**, e há um teste que compara as duas constantes.

### 13 · Backpressure só do lado do browser

Lossy por contrato (ADR-0015), e a **direção importa**: um browser lento nunca atrasa a
leitura do IPC, e o console nunca atrasa o daemon. Fila cheia → descarta o **mais antigo** e
incrementa `console.dropped`, que é **reportado**, não escondido. O polling de `/api/state`
corrige a deriva.

### 14 · Sem `shutdown`, sem blackout

`shutdown` é irreversível, tem duas fases e não há auth — fica no `ledctl`, que exige acesso
ao socket. **Blackout não existe**: o ADR-0017 está adiado, e a ausência é a decisão.

### 15 · Nenhuma segunda fonte de verdade

O console **transporta** contratos; não os reimplementa. Nada de máquina de estados, regras de
hardware, `Calibration`/LUT, MTU, `refresh_hz`, `HardwareProfile` ou serialização canónica
dentro dele.

## Alternativas rejeitadas

| Alternativa | Porque não |
|---|---|
| HTTP dentro do `led-daemon-bin` | Viola o ADR-0013 e cria um segundo aplicador ao lado do laço |
| WebSocket para tudo | Abre um segundo caminho de comando; reconexão passa a ser código nosso |
| Uma subscrição IPC por browser | A carga no daemon passaria a depender de quantos separadores estão abertos |
| Console a calcular saúde própria | Segunda fonte de verdade — a classe de defeito que a auditoria de 2026-08-07 fechou |

## Consequências

**Positivas.** O daemon fica intocado. A UI ganha uma fronteira testável sem browser. A
distinção observabilidade↔evidência fica escrita antes de existir um ecrã que a possa violar.

**Negativas.** Um processo a mais para iniciar e supervisionar. A ponte é superfície nova e
precisa dos seus próprios gates.

**Não coberto.** A stack de UI (ADR-0016, ainda provisório — depende de medição humana).
Autenticação (ADR-0014). Preview de pixels (ADR-0015).

## Critério de reversão

Se o console vier a precisar de estado próprio que não seja derivável do daemon, isso é sinal
de que uma decisão de domínio escorregou para ele — e a correção é devolvê-la ao daemon, não
alargar o console.
