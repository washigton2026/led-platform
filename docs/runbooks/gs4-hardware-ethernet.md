# GS4.3 — runbook de validação física (Ethernet, ESP32-POE)

> **NENHUMA ETAPA ESTÁ CONCLUÍDA.** Todos os campos de resultado estão vazios de propósito.
> Em 2026-08-05 os cinco nós do rig (`192.168.2.156–160`) **não responderam a ping**, e o
> material (ESP32-POE, switch, cabos) ainda não foi adquirido.
>
> **O software está pronto e provado até onde se pode provar sem hardware.** O que falta
> é o rig. Cada etapa abaixo tem cinco campos, e nenhuma pode ser marcada sem o quinto
> preenchido por um operador que a executou.

## O que já está provado sem hardware (não repetir)

| Facto | Como foi provado |
|---|---|
| `.lumyx` → daemon → UDP, nos três protocolos | `tests/e2e_output.rs`, datagramas lidos de socket |
| Fragmentação prevista = fragmentação no fio | `tests/wled_driver.rs`, 720 px → 2 (DDP) / 5 (Art-Net) |
| Ordem de canais vem do `HardwareProfile` | `tests/wled_driver.rs`, GRB confirmado byte-a-byte |
| Universos consecutivos a partir do primeiro | `tests/wled_driver.rs` |
| ArtPoll ida-e-volta e lógica de presença | `tests/discovery.rs`, sockets reais |
| Pré-voo reprova com WiFi ativo e nó ausente | execução real, 2026-08-07 |
| Heartbeat reenvia o último frame, nunca zeros | `src/stage.rs`, gap medido < 2400 ms |

## O que **só** o hardware pode dizer

Que um WLED **aceita** estes bytes e acende os pixels certos. Tudo acima descreve o que sai
do daemon; nada acima descreve o que entra no controlador.

## Material

| Item | Nota |
|---|---|
| 1× Olimex ESP32-POE | **suficiente para o GS4** — não é preciso o rig de 5 |
| Switch Gigabit | com PoE, ou switch + injetor PoE |
| Cabo CAT5e/CAT6 | ≥ 2 |
| Fita LED + fonte | reusar a de bancada (720 px WS2812B, DC/DC 5 V/10 A, cap 1000 µF, R 330 Ω) |

⚠️ **ABL do WLED em 850 mA** antes de energizar, como na bancada de 2026-07-20 — a fita
enrolada não dissipa.

⚠️ **Desligue o WiFi da máquina** antes das etapas com `--output`. O pré-voo **vai bloquear**
com WiFi ativo: é o ADR-0005 a funcionar, não uma avaria. O journal nomeia a interface.

---

## ETAPA 1 — Rede física

**Objetivo.** Saber se o caminho Ethernet existe e é estável antes de culpar o software.

**Procedimento.**
```sh
ping -c 20 <IP-DO-ESP32>
```

**Critério de aceite.** 20/20 respostas, 0 % de perda, **jitter uma ordem de grandeza abaixo
dos 31 ms** medidos na bancada WiFi de 2026-07-20 (que foi o que confirmou o ADR-0005).

**Evidência esperada.** Saída completa do `ping`, com a linha de `min/avg/max/stddev`.

**Resultado.** ⏳ _perda: ____ · avg: ____ ms · stddev: ____ ms_

---

## ETAPA 2 — O controlador responde

**Objetivo.** Confirmar firmware e estado do nó antes de lhe enviar um show.

**Procedimento.**
```sh
curl -s http://<IP>/json/info
```

**Critério de aceite.** JSON válido, `ver` ≥ 16.0.1, `arch` esperado, `freeheap` > 20 000.

**Evidência esperada.** O JSON completo, guardado em ficheiro.

**Resultado.** ⏳ _ver: ____ · arch: ____ · freeheap: ____ · uptime: ____ s_

---

## ETAPA 3 — Descoberta ArtPoll contra hardware real

**Objetivo.** Provar que o nó responde a ArtPoll — o que `tests/discovery.rs` só provou
contra um responder de loopback.

**Procedimento.**
```sh
./target/release/led-player striptest.lumyx --ddp <IP> --discover --require-all
echo "exit=$?"
```

**Critério de aceite.** O nó aparece em `responded`. Com o cabo **desligado**, `--require-all`
tem de abortar com **exit 1** — é o controle negativo, e sem ele a etapa não prova nada.

**Evidência esperada.** As duas execuções (cabo ligado e desligado) com os exit codes.
⚠️ Ler o exit code **sem pipe** (KB-013): um pipe mede o último comando, não este.

**Resultado.** ⏳ _com cabo: exit ____ · sem cabo: exit ____ (esperado 1)_

---

## ETAPA 4 — Primeiro frame do **daemon** no hardware

**Objetivo.** Fechar o caminho `.lumyx` → daemon → Ethernet → WLED → pixel. É a primeira vez
que o daemon (e não o `led-player`) acende um LED físico.

**Procedimento.**
```sh
./target/release/led-daemon striptest.lumyx --assume-integrity \
    --profile esp32-poe-wled-ddp \
    --output ddp://<IP> --tick-ms 25 --max-ticks 200
```

`--profile` é **obrigatório** sempre que há `--output` (GS4.4): protocolo, ordem de canais,
universos, MTU e heartbeat vêm todos do preset. Sem ele o daemon sai com **exit 2** e manda
usar `--list-profiles`.

Repetir com os outros dois protocolos. **Art-Net e sACN exigem o universo na própria
especificação** (`IP@N`, ADR-0029 §7); o DDP recusa-o, porque endereça por byte:
```sh
--profile esp32-devkit-wled-artnet --output artnet://<IP>@0
--profile falcon-f16v3-sacn        --output sacn://<IP>@1
```
O mínimo difere por protocolo — o Art-Net define o universo 0, o E1.31 não. Escrever
`artnet://<IP>` sem `@N` faz o daemon recusar com *"exige o universo"*, e a razão está na
bancada de 2026-07-23: o universo errado **deslocou a fita sem erro nenhum**.

**Critério de aceite.** Journal sem `output_error`; WLED com `live:true` e `lm:"DDP"` (ou
`"Art-Net"`); e o **visual R→G→B→cometa confirmado a olho**. O `lm` do WLED é evidência de
aceitação mais forte que tcpdump (precedente 2026-07-23).

**Evidência esperada.** Journal JSONL completo + `/json/info` durante a reprodução + foto ou
vídeo curto da fita.

**Resultado.** ⏳ _DDP: ____ · Art-Net: ____ · sACN: _____

> Sobre **sACN**: em 2026-07-23 provou-se que o WLED 16.0.1 **não faz bind na 5568** — ICMP
> port-unreachable idêntico a porta não usada, e um sender de referência independente falha
> igual. **Se falhar aqui, é firmware, não LUMYX.** Registar como bloqueio externo, não como
> defeito.

---

## ETAPA 5 — Ordem de canais e fragmentação, no vidro

**Objetivo.** Confirmar em pixels aquilo que `tests/wled_driver.rs` confirmou em bytes — e
apanhar um eventual desacordo entre o preset e a fita real.

**Procedimento.** Reproduzir o segmento **vermelho puro** do `striptest` com
`--output ddp://<IP>` e olhar para a fita. Depois, verificar que o pixel 488 (o primeiro do
segundo datagrama DDP) tem a mesma cor que o 487.

**Critério de aceite.** Vermelho aparece **vermelho**. Nenhuma descontinuidade de cor na
fronteira dos 487 px. Se aparecer verde, o preset declara a ordem errada para esta fita — é
uma linha no `presets.rs`, não código.

**Evidência esperada.** Foto da fita no segmento vermelho e na fronteira dos 487 px.

**Resultado.** ⏳ _cor: ____ · fronteira 487: ____ · preset usado: _____________

---

## ETAPA 6 — Transporte: Play/Pause/Stop/Seek/Finished no físico

**Objetivo.** Provar que a decisão 3 do ADR-0023 — *transporte não apaga o palco* — se
comporta no vidro como se comporta no teste.

**Procedimento.**
```sh
./target/release/led-daemon --socket /tmp/lumyx.sock \
    --profile esp32-poe-wled-ddp --output ddp://<IP> \
    --tick-ms 25 --keep-running &
./target/release/ledctl --socket /tmp/lumyx.sock load striptest.lumyx --assume-integrity
./target/release/ledctl --socket /tmp/lumyx.sock play
./target/release/ledctl --socket /tmp/lumyx.sock pause     # a fita NÃO pode apagar
./target/release/ledctl --socket /tmp/lumyx.sock seek 2000
./target/release/ledctl --socket /tmp/lumyx.sock stop      # também NÃO pode apagar
```

**Critério de aceite.** Cada comando tem efeito visível, e **`pause` e `stop` deixam a fita
acesa** no último frame. Apagar seria blackout, que continua bloqueado pelo ADR-0017.

**Evidência esperada.** Vídeo da sequência completa + journal do daemon.

**Resultado.** ⏳ _play: ____ · pause: ____ · seek: ____ · stop: ____ · finished: _____

---

## ETAPA 7 — Heartbeat e recuperação de cabo

**Objetivo.** Medir o que acontece quando o meio físico falha — o cenário que o WiFi tornou
impossível de isolar em 2026-07-23.

**Procedimento.** Com o show em `pause` (heartbeat ativo), **desligar o cabo** 5 s e voltar a
ligar. Repetir com o show em `play`.

**Critério de aceite.** A fita recupera sem intervenção. `uptime` do ESP32 **monotónico**
(um reset invalida a etapa). `freeheap` estável.

**Evidência esperada.** `/json/info` antes, durante e depois; journal do daemon com os
`output_error`; cronometragem da recuperação.

**Resultado.** ⏳ _recuperação: ____ s · uptime antes/depois: ____ / ____ · freeheap: ____ / _____

---

## ETAPA 8 — Burn-in 2 h

**Objetivo.** Detetar deriva, fuga de memória e falha de transporte que só aparecem com
tempo. **Não é o burn-in de 72 h da certificação** — é o pré-gate dele.

**Procedimento.**
```sh
scripts/burnin.sh 2 striptest.lumyx <IP>
```

**Critério de aceite.** 0 aborts. Hash **idêntico** em todos os passes. `uptime` do ESP32
monotónico. `freeheap` sem tendência descendente.

**Evidência esperada.** `docs/certification/burnin-gs4-<data>.md` com a tabela abaixo
preenchida.

| Métrica | Valor |
|---|---|
| Passes / aborts | ⏳ |
| Hash por pass (idêntico?) | ⏳ |
| Jitter de rede (ping paralelo) | ⏳ |
| Perda de pacotes | ⏳ |
| CPU do daemon | ⏳ |
| RSS do daemon | ⏳ |
| `freeheap` ESP32 antes/depois | ⏳ |
| `uptime` ESP32 (reset = falha) | ⏳ |

**Resultado.** ⏳ _____________

> **Honestidade obrigatória:** DDP é *fire-and-forget* sem ACK. `frames_sent` mede sucesso do
> `sendto`, **não** exibição no WLED. A continuidade sob carga é **observação visual** — foi
> assim que se registou em 2026-07-20 e não mudou.

---

## ETAPA 9 — Golden Slice físico

**Objetivo.** Fechar o vertical slice: `ledctl load → play → Ethernet → ESP32-POE → WLED →
fita → Finished`, sem intervenção manual no meio.

**Procedimento.** Executar a etapa 6 até ao fim do show, sem tocar em mais nada.

**Critério de aceite.** Todas as etapas 1–8 com resultado preenchido **e** aprovadas. O show
chega a `Finished` sozinho, com a fita acesa no último frame.

**Evidência esperada.** Relatório em `docs/certification/` no formato de
`HARDWARE-VALIDATION-2026-07-20.md`, incluindo uma secção do que **não** ficou validado.

**Resultado.** ⏳ _____________

---

## ETAPA 10 — Multi-nó: da unidade ao rig

**Objetivo.** Provar que N nós recebem **cada um a sua fatia**, e que a perda de um não apaga
os outros. As nove etapas anteriores validam **um** nó; esta é a única que fala do rig.

**Pré-condição.** Etapas 1–9 verdes **no primeiro nó**. Um controlador validado não implica
cinco: escalar é `1 → 2 → 5`, e cada aumento é observado.

**Procedimento.** Medir cada nó **individualmente** primeiro — o `lumyx-hwcheck` aceita vários
endereços e produz um veredito **por alvo**, sem os agregar:
```sh
./target/release/lumyx-hwcheck <IP1> <IP2> ... --profile esp32-poe-wled-ddp
```
Depois, o rig como sistema. A repartição é **derivada** do `max_pixels` do preset, e a **ordem**
dos `--output` decide que fatia vai para cada nó:
```sh
./target/release/led-daemon robot_sequence.lumyx --assume-integrity \
    --profile esp32-poe-wled-ddp \
    --output <IP1> --output <IP2> --output <IP3> --output <IP4> --output <IP5> \
    --tick-ms 25 --keep-running &
./target/release/ledctl --socket /tmp/lumyx.sock status   # `outputs` traz a contagem POR NÓ
```

**Critério de aceite.** Cada nó acende **a sua parte** do show, não o mesmo conteúdo; o
`status` traz uma entrada por nó, nomeada pelo endereço; e ao desligar o cabo de **um** nó os
outros continuam acesos, com o erro atribuído só a ele.

**O que esta etapa NÃO prova.** Sincronização visual entre nós — nenhuma medição de software o
faz. Se os cinco robôs parecerem dessincronizados a olho, isso é observação humana e entra no
relatório como tal.

**Evidência esperada.** Um relatório do `lumyx-hwcheck` por nó + `status` com as N entradas +
vídeo do rig com um nó desligado a meio.

**Resultado.** ⏳ _nós medidos: ____ · fatias correctas: ____ · perda isolada: _____

---

## Operar pelo browser (console + Web Platform)

O daemon é comandado por `ledctl` **ou** pelo browser. As duas superfícies falam o **mesmo**
IPC v1 — o console é um tradutor de transporte, não um segundo cérebro.

```sh
# 1. daemon (como na ETAPA 6), com --socket
./target/release/led-daemon --socket /tmp/lumyx.sock --profile <preset> --output <IP> \
    --tick-ms 25 --keep-running &

# 2. console: ponte HTTP↔IPC. Ambas as flags são OBRIGATÓRIAS — nenhum servidor
#    deste projecto escolhe sozinho onde escuta.
./target/release/led-console --bind 127.0.0.1:7878 --socket /tmp/lumyx.sock &

# 3. a interface (dev server; o proxy do Vite mantém a mesma origem)
cd console-web && npm ci && npm run dev
```

**O que o ecrã mostra:** estado do transporte, posição, duração, ticks, show carregado, o
estado da ligação em **três camadas nomeadas** (`Console API`, `Browser stream`,
`Daemon subscription`) e a contabilidade **por nó** da saída. Comandos: `play`/`pause`/`stop`/
`seek` e `load`/`unload`.

**O que o ecrã NÃO mostra, e é deliberado (ADR-0028 D3):** não há `healthy`, `degraded` nem
`connected` — nada no backend os produz, e inventá-los seria a interface a afirmar evidência
que ninguém mediu. `/api/profiles` responde **501**: a rota existe e o catálogo não atravessa
a fronteira autorizada (ADR-0026 §9-quater). O preset escolhe-se na CLI do daemon, com
`--list-profiles`.

**Loopback-only.** O console recusa fazer bind fora de loopback enquanto o ADR-0014 não
definir autenticação. Não o exponha na LAN.

---

## O que fica fora, mesmo com as dez etapas verdes

- **1 nó de 5 nas etapas 1–9.** 720 px de 6.200. **O software já não é o
  limite:** desde o ADR-0029 o `--output` é **repetível** e a repartição deriva do
  `max_pixels` do preset (`--output <IP1> --output <IP2> …`, e a **ordem** decide que fatia
  vai para cada nó). O que falta é **hardware** — cinco nós, switch e cabos — e a medição do
  rig como sistema, que a ETAPA 10 descreve.
- **Burn-in de 2 h não é 72 h.** O critério de certificação é 72 h.
- **Show musical real** (`robot_sequence.lumyx`, 73 MB) — o `striptest` é síntese de bring-up.
- **Chaos físico** além do cabo puxado uma vez.
- **sACN**, se o firmware continuar sem fazer bind na 5568.
