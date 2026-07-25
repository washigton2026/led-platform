# Runbook — Início de show (led-player)

Sequência de partida de um show, do arquivo `.lumyx` até os pixels. Cada passo
tem **pré-condições**, **comando exato**, **saída esperada** e **o que fazer se
falhar** — todos fiéis ao que o `led-player` realmente imprime hoje
(`crates/led-player/src/main.rs`).

> Documento apenas. Nenhum comando altera o código da plataforma.
>
> **Legenda de validação**
> - ✅ **VALIDADO** — o caminho já foi exercido (CLI/simulador) e a saída abaixo
>   é a real.
> - ⚠ **NÃO VALIDADO EM HARDWARE** — o caminho existe no código mas **nunca
>   acendeu um LED físico** (rig offline). A saída é a que o código emite; o
>   comportamento no metal é presumido, não observado.

Ordem de execução: **1 → 2 → 3 → 4 → 5 → 6**. Não pule a verificação (2–3) nem o
ensaio no simulador (4) antes de acionar hardware (6).

---

## 0. Pré-condições gerais

- Binário compilado: `cargo build -p led-player --release` (ou rode via
  `cargo run -p led-player --`).
- O arquivo do show existe (ex.: `robot_sequence.lumyx`, 3925 frames, 6.200 px).
- Para o passo 3 (autenticidade): o sidecar `<show>.sig` existe e você conhece a
  **pubkey fixada** do estúdio (viaja out-of-band — ver [ADR-0004](../adr/0004-ed25519-pinned-verification.md)).
- Para os passos 5–6 (hardware): rede já migrada para Ethernet cabeada
  (ver [wifi-to-ethernet-migration.md](./wifi-to-ethernet-migration.md)); nenhuma
  interface WiFi ativa (o `NetworkGuard` recusa iniciar com WiFi ligado).

---

## 1. Inspecionar o show — `--info`  ✅ VALIDADO

Confirma que o arquivo abre, quantos frames/pixels tem e qual é o hash. **Não
emite nada na rede.**

**Comando:**
```
cargo run -p led-player -- robot_sequence.lumyx --info
```

**Saída esperada** (a 1ª linha é sempre impressa — `ShowInfo::to_json()`):
```
{"frames":3925,"pixels":6200,"duration_ms":...,"beats":...,"hash":"0xd8f1479ff3645e1e"}
```
`--info` retorna sucesso (exit 0) sem tocar em nenhuma saída.

**Se falhar:**
- `cannot open '<path>': ...` → caminho errado ou arquivo ausente. Confira o nome.
- `cannot read '<path>': ...` → arquivo corrompido/truncado. Recupere de backup e
  re-verifique o hash.

---

## 2. Verificar integridade — `--verify <hash>`  ✅ VALIDADO

Confere que o show bate com um hash conhecido (detecta corrupção/edição
acidental). Use o hash da linha `--info` ou o do ledger de shows.

**Comando:**
```
cargo run -p led-player -- robot_sequence.lumyx --verify 0xd8f1479ff3645e1e --info
```

**Saída esperada:**
```
{"frames":3925,"pixels":6200,...,"hash":"0xd8f1479ff3645e1e"}
verify: OK (0xd8f1479ff3645e1e)
```

**Se falhar:**
```
VERIFY FAILED: recorded 0x<real> != expected 0x<informado>
```
exit 1. O arquivo **não** é o que você pensa que é. **Pare.** Não acione hardware
com um show cujo hash não bate — recupere a versão correta.

---

## 3. Verificar autenticidade — `--verify-key <pubkey>`  ✅ VALIDADO (no CLI)

Prova que o show foi assinado pela **chave fixada** do estúdio, não re-assinado
por um atacante. Este é o caminho de fronteira de confiança (RT-001) — verificação
de integridade sozinha (passo 2) **não** basta.

### 3a. Produtor (estúdio) assina — uma vez, antes de levar ao palco

**Comando:**
```
cargo run -p led-show-recorder --example sign_show -- robot_sequence.lumyx <seed-hex-64>
```

**Saída esperada:**
```
signed: robot_sequence.lumyx.sig
hash:   0xd8f1479ff3645e1e
pubkey: d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737
verify at venue: led-player robot_sequence.lumyx --verify-key d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737
```
Leve o `.sig` junto com o show; anote a **pubkey** (ela é o que o palco fixa).

### 3b. Consumidor (palco) verifica com a chave fixada

**Comando:**
```
cargo run -p led-player -- robot_sequence.lumyx --verify-key d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737 --info
```

**Saída esperada:**
```
{"frames":3925,...,"hash":"0xd8f1479ff3645e1e"}
sig-verify: OK (authentic, pinned key)
```

**Se falhar:**
- `--verify-key needs a signature sidecar 'robot_sequence.lumyx.sig': ...`
  → o `.sig` não está ao lado do show. Copie-o.
- `malformed sidecar '...'` → sidecar corrompido. Re-assine no estúdio (3a).
- `SIG VERIFY FAILED: sidecar covers 0x..., show is 0x...` → o `.sig` é de **outro**
  show (ou o show foi editado). Os dois não combinam — não toque.
- `SIG VERIFY FAILED: signed by a key that is NOT the pinned key (possible re-signed tamper)`
  → exit 1. **Este é o sinal vermelho de segurança.** O show foi assinado por uma
  chave que **não** é a do estúdio. Trate como comprometido; não acione hardware.
  (Prova e2e: atacante com pubkey `34b4…` é rejeitado pela chave fixada do estúdio.)

---

## 4. Ensaio no simulador (dry-run)  ✅ VALIDADO

Toca o show inteiro **sem hardware**, no ritmo real, com métricas ligadas.
Confirma que o pipeline roda ponta-a-ponta e que a latência está no orçamento.

**Comando** (sem `--artnet`/`--ddp` ⇒ simulador; `--metrics` expõe Prometheus):
```
cargo run -p led-player -- robot_sequence.lumyx --metrics 9464
```

**Saída esperada:**
```
{"frames":3925,...,"hash":"0xd8f1479ff3645e1e"}
metrics: http://0.0.0.0:9464/metrics
output: simulator
{"pass":1,"played":3925,"failed":0,"duration_ms":...,"hash":"0xd8f1479ff3645e1e"}
```
Durante a reprodução, `curl http://localhost:9464/metrics` mostra latência e drops
ao vivo (validado mid-show: p50 ≈ 0,5 ms, p99 ≈ 4,1 ms em debug, 0 drops).

**Se falhar:**
- `metrics bind failed: ...` → porta 9464 ocupada. Escolha outra (`--metrics 9465`)
  ou libere a porta.
- `played < frames` ou `"failed" > 0` na linha do pass → há frames caindo mesmo no
  simulador. **Não prossiga para hardware** — investigue antes.

> Dica: para acelerar o ensaio, `--speed max` toca o mais rápido possível (o hash
> não muda com a velocidade). Para um teste de estabilidade curto, `--loop 30`
> repete 30× re-verificando o hash a cada passe (ver [controller-offline.md](./controller-offline.md)
> para o significado de um BURN-IN ABORT).

---

## 5. Discovery contra o rig — `--discover` / `--require-all`

Antes do 1º frame no hardware, confirma que o controlador-alvo responde ao
ArtPoll. Fecha o footgun "controlador ausente = palco escuro sem erro" (RT-003).

- ⚠ **NÃO VALIDADO EM HARDWARE** — o caminho **✅ respondeu** (controlador vivo)
  nunca foi observado (rig offline).
- ✅ **VALIDADO** — o caminho **⚠ SEM resposta → ABORT** foi provado contra o rig
  real desligado.

**Comando** (aborta o show se o alvo não responder):
```
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --require-all --info
```

**Saída esperada quando o controlador responde** (⚠ presumido, não observado):
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ✅ respondeu
```

**Saída quando NÃO responde** (✅ observado no rig offline):
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ⚠ SEM resposta
ABORT: --require-all e 192.168.2.156 não respondeu ao ArtPoll (desligado? WiFi morto? subnet errada?)
```
exit 1. Ver [controller-offline.md](./controller-offline.md) para o diagnóstico.

**Se falhar:**
- `discovery falhou (socket ArtPoll :6454 — precisa de porta livre): ...` → outro
  processo (outro controlador de luz, xLights) segura a porta 6454. Feche-o.
- `--discover requer um alvo IPv4 (--artnet/--ddp)` → você pediu `--discover` sem
  `--artnet`/`--ddp`. Informe o IP do controlador.
- Sem `--require-all`, um alvo silencioso **não** aborta — só avisa
  `aviso: seguindo mesmo assim (sem --require-all); o palco pode ficar escuro`.
  No palco, **sempre use `--require-all`**.

---

## 6. Reprodução no hardware — `--ddp` ou `--artnet`  ⚠ NÃO VALIDADO EM HARDWARE

Toca o show acendendo os LEDs físicos. **Nenhum LED real foi aceso por este
caminho ainda** — o rig está offline. A saída abaixo é a que o código emite; o
resultado luminoso é presumido.

**Pré-condições extras:** passos 2–3 verdes, passo 5 com `✅ respondeu` para o
alvo, rede cabeada, sem WiFi ativo.

### Opção A — DDP (recomendado para WLED: pixel-nativo, ~3× menos pacotes)
```
cargo run -p led-player -- robot_sequence.lumyx --ddp 192.168.2.156 --require-all --metrics 9464
```
**Saída esperada:**
```
{"frames":3925,...,"hash":"0xd8f1479ff3645e1e"}
metrics: http://0.0.0.0:9464/metrics
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ✅ respondeu
output: DDP 192.168.2.156 (pixel-native)
{"pass":1,"played":3925,"failed":0,"duration_ms":...,"hash":"0xd8f1479ff3645e1e"}
```

### Opção B — Art-Net (universos a partir de `--first-universe`, 170 px/universo)
```
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --first-universe 1 --require-all --metrics 9464
```
**Saída esperada:**
```
...
output: ArtNet 192.168.2.156 (universes 1..37)
{"pass":1,"played":3925,"failed":0,...}
```

**Se falhar:**
- `⚠ --first-universe 0: WLED/xLights number universes from 1 — did you mean 1?`
  → você passou `--first-universe 0`. WLED/xLights numeram de 1. Corrija para 1.
  (É só um aviso; não bloqueia.)
- `⚠ universes N..M exceed the 15-bit Art-Net range — likely a typo` → provável
  erro de digitação no `--first-universe`. Confira.
- `ddp socket: ...` / `artnet socket: ...` → falha ao abrir o socket UDP de saída
  (permissão, interface). Verifique a rede da máquina de controle.
- `"failed" > 0` na linha do pass, ou trecho do palco apagado → um segmento não
  está recebendo. Rode o passo 5 individualmente por controlador; ver
  [controller-offline.md](./controller-offline.md).

---

## 7. Critério de partida bem-sucedida

- Passos 1–4 verdes (info, verify, sig-verify, ensaio no simulador com 0 failed).
- Passo 5: `✅ respondeu` para **todos** os controladores usados (`--require-all`).
- Passo 6: linha do pass com `"played" == "frames"` e `"failed":0`; `/metrics`
  dentro do SLO (p99 ≤ 5 ms cabeado); palco aceso conforme o design.

Enquanto o rig estiver offline, a partida é **certificada só até o passo 4**; os
passos 5–6 no metal continuam **NÃO VALIDADOS EM HARDWARE**.
