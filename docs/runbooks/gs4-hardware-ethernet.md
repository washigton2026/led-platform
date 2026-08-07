# GS4.3–GS4.7 — runbook de validação física (Ethernet, ESP32-POE)

> **Estado: BLOQUEADO POR HARDWARE.** Em 2026-08-05 os cinco nós do rig
> (`192.168.2.156–160`) **não responderam a ping**. Nada abaixo foi executado, e **nada
> abaixo pode ser afirmado** até ser executado por um operador com o rig ligado.
>
> O que **está** feito e verificado em software: **GS4.1** (OutputManager, três protocolos) e
> **GS4.2** (pipeline `.lumyx` → fio, bytes reais em loopback UDP). Ver o changelog do
> `CLAUDE.md`.

Formato herdado de [`HARDWARE-VALIDATION-2026-07-20.md`](../certification/HARDWARE-VALIDATION-2026-07-20.md):
**uma etapa de cada vez, só avança com evidência observada.** Sem evidência, a etapa fica ⏳ —
nunca ✅.

## Material

| Item | Nota |
|---|---|
| 1× Olimex ESP32-POE | **suficiente para o GS4** — não é preciso o rig de 5 |
| Switch Gigabit | com PoE, ou switch + injetor PoE |
| Cabo CAT5e/CAT6 | ≥ 2 |
| Fita LED + fonte | reusar a de bancada (720 px WS2812B, DC/DC 5 V/10 A, cap 1000 µF, R 330 Ω) |

⚠️ **ABL do WLED em 850 mA** antes de energizar, como na bancada de 2026-07-20 — a fita
enrolada não dissipa.

---

## ETAPA 1 — Rede física

```sh
ping -c 5 <IP-DO-ESP32>
```

**Evidência:** 5/5 respostas, e **jitter**. Registar o número: a bancada WiFi deu
**99 ms avg / jitter 31 ms**, e foi isso que confirmou o ADR-0005. Ethernet tem de ser
**ordens de grandeza melhor** — se não for, o cabo/switch tem problema e o resto não vale.

⏳ Resultado: _____ · jitter: _____

## ETAPA 2 — WLED responde

```sh
curl -s http://<IP>/json/info | head -c 400
```

**Evidência:** JSON válido, e anotar `ver`, `arch`, `freeheap`, `uptime`.

⏳ Resultado: _____

## ETAPA 3 — Descoberta (ArtPoll)

```sh
./target/release/led-player striptest.lumyx --ddp <IP> --discover --require-all
```

**Evidência:** `--require-all` **aborta com exit 1** se o controlador silenciar. Ver
exit code **sem pipe** (KB-013).

⏳ Resultado: _____

## ETAPA 4 — Primeiro frame do **daemon** (não do player)

```sh
./target/release/led-daemon --socket /tmp/lumyx.sock --tick-ms 25 --keep-running &
./target/release/ledctl --socket /tmp/lumyx.sock load striptest.lumyx --assume-integrity
./target/release/ledctl --socket /tmp/lumyx.sock play
```

> ⚠️ **O daemon ainda não tem a saída ligada ao laço.** GS4.1/4.2 entregaram o
> `OutputManager` e o `FrameSource` com pipeline provado em loopback, mas **`--output` no
> `led-daemon` é a próxima fatia de código**. Até lá, esta etapa valida-se com o
> `led-player`, que já é o caminho validado em hardware:

```sh
./target/release/led-player striptest.lumyx --ddp <IP>
```

**Evidência:** `played N/0`; WLED `/json/info` com `live:true` e `lm:"DDP"`; **e o visual
R→G→B→cometa confirmado a olho**. O `lm` do WLED é evidência de aceitação mais forte que
tcpdump (precedente 2026-07-23).

⏳ DDP: _____ · Art-Net: _____ · sACN: _____

> Sobre **sACN**: em 2026-07-23 provou-se que o WLED 16.0.1 **não faz bind na 5568** —
> ICMP port-unreachable idêntico a porta não usada, e um sender de referência independente
> falha igual. **Se falhar aqui, é firmware, não LUMYX.** Registar como bloqueio externo.

## ETAPA 5 — Transporte: Play/Pause/Stop/Seek/Finished no físico

Com o daemon (após a fatia `--output`) ou o player, verificar **a olho** que cada comando
tem efeito visível, e que **`Pause` e `Stop` NÃO apagam o rig** — o heartbeat continua a
reenviar o último frame (ADR-0023 §3). Apagar seria blackout, que está bloqueado pelo
ADR-0017.

⏳ Play: _____ · Pause: _____ · Stop: _____ · Seek: _____ · Finished: _____

## ETAPA 6 — Heartbeat e reconnect

1. Com o show a correr, **desligar o cabo** 5 s e voltar a ligar.
2. Observar: o rig recupera? Em quanto tempo? Há reset do ESP32 (`uptime` volta a zero)?

**Evidência:** `uptime` monotónico = sem reset; `freeheap` estável = sem leak.

⏳ Recuperação: _____ · uptime antes/depois: _____ · freeheap: _____

## ETAPA 7 — Burn-in 2 h

```sh
scripts/burnin.sh 2 striptest.lumyx <IP>
```

**Registar em `docs/certification/burnin-gs4-<data>.md`:**

| Métrica | Como medir | Valor |
|---|---|---|
| Passes / aborts | saída do burn-in | ⏳ |
| Hash por pass | tem de ser **idêntico** em todos | ⏳ |
| Jitter de rede | `ping` em paralelo | ⏳ |
| Perda | `ping` loss % | ⏳ |
| CPU do daemon | `top -pid <pid>` | ⏳ |
| Memória do daemon | idem, RSS | ⏳ |
| `freeheap` do ESP32 | `/json/info` antes/depois | ⏳ |
| `uptime` do ESP32 | idem — **reset = falha** | ⏳ |

> **Honestidade obrigatória:** DDP é *fire-and-forget* sem ACK. `played` mede sucesso do
> `sendto`, **não** exibição no WLED. A continuidade sob carga é **observação visual** — foi
> assim que se registou em 2026-07-20 e não mudou.

## ETAPA 8 — Golden Slice físico

```
ledctl load → play → Ethernet → ESP32-POE → WLED → fita → Finished
```

Só marcar ✅ com **todas** as etapas anteriores com evidência escrita. Depois, escrever o
relatório em `docs/certification/` no formato de 2026-07-20, incluindo **o que NÃO ficou
validado**.

---

## O que fica fora, mesmo com tudo verde

- **1 nó de 5.** 720 px de 6.200. Multi-controlador é outra fatia.
- **Burn-in de 2 h não é 72 h.** O critério de certificação é 72 h.
- **Show musical real** (`robot_sequence.lumyx`) — o `striptest` é síntese de bring-up.
- **Chaos físico** além do cabo puxado uma vez.
