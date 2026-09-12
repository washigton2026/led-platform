# Plano de controle UI ↔ engine — especificação

> **Decisão** (transporte, auth, comandos tipados/versionados) está no
> [ADR-0014](../adr/0014-ipc-seguranca-ui-engine.md). Este documento concretiza **como**,
> e — mais importante — registra **o que o engine ainda não sabe fazer**.
>
> Status: **especificação, não implementação.** Nada aqui existe em código ainda.

## Dois canais, não um

| Canal | Direção | Estado | Onde |
|---|---|---|---|
| **Read-model** | engine → UI | ✅ **implementado** | `led-readmodel` — `GET /` → JSON, **loopback-only** |
| **Comandos** | UI → engine | ⬜ **não existe** | este documento |

O canal de leitura já roda e recusa bind não-loopback. O canal de comandos é greenfield: hoje
**não há nenhuma superfície de controle** no engine além da construção no startup.

## 🔴 Análise de lacuna — leia antes de projetar telas

Levantei o que existe hoje como comandável. O resultado é pequeno, e isso muda o roadmap:

### O que EXISTE e pode virar comando amanhã

| Capacidade | Onde | Observação |
|---|---|---|
| `connect` / `disconnect` | `IDevice` (`led-core/traits.rs`) | plano de gestão, já separado do frame path |
| `configure(DeviceConfig{name, priority})` | `IDevice` | |
| `reboot` | `IDevice` | **ação irreversível** → exige confirmação + log |
| `update_firmware(image)` | `IDevice` | recusado enquanto o device está ao vivo |
| `discover_controllers(expected, timeout)` | `led-protocols::artnet` | pré-show, ArtPoll |
| `check_network()` | `Hal` | gate WiFi (ADR-0005) |

### O que a UI vai precisar — o que o engine já tem, e o que ainda não

| O console vai querer | Existe? | Realidade |
|---|---|---|
| play / pause / stop do show | ✅ | `Cmd::Play` / `Pause` / `Stop` (`led-daemon-bin/src/proto.rs:63-65`), IPC v1 (ADR-0023, GS1–GS3). A observação sobre o `led-player` continua verdadeira **para o `led-player`** — quem faz transporte é o `led-daemon` |
| seek / scrub na timeline | ✅ | `Cmd::Seek { to_ms }` (`proto.rs:66`); `PositionChanged` carrega a `cause` que distingue avanço de salto (ADR-0023 F2) |
| carregar/trocar show sem reiniciar | ✅ | `Cmd::Load { path, assume_integrity }` / `Cmd::Unload` (`proto.rs:61-62`), na UI desde 2026-08-14. `assume_integrity` é afirmação do operador, **nunca verificação** |
| mudar calibração ao vivo | ❌ | `Hal::with_calibration` é **construtor**; não há setter em runtime |
| grand master / intensidade global | ❌ | não existe |
| blackout | 🟡 | **decidido** ([ADR-0017](../adr/0017-blackout-intencional-vs-heartbeat.md), 2026-09-01): a máscara vive no `OutputManager`, **a jusante** de `heartbeat.record()` — o preto persiste sem ser gravado. **Máscara implementada** em 2026-09-04 (`1030a7e`), com o escape por device. **Falta o comando de operador:** é o D6 (botão + duas fases + log), e a decisão 6 exige `PROTOCOL_V = 2` — ver ADR-0031: **aceite**, com a metade **emissora** da decisão 2 implementada (o `accepts` derivado de `SUPORTADAS`, `led-daemon-bin/src/server.rs:306` e `proto.rs:176`), e as decisões 1 e 3–7 **por implementar** — falta o estado de versão **por ligação** e uma segunda versão em `SUPORTADAS` |

**Consequência para o roadmap — atualizada em 2026-09-02.** A versão anterior desta secção
dizia que a superfície de transporte *"ainda não foi projetada"* e que *"deve virar ADR próprio"*.
**As duas coisas aconteceram:** o [ADR-0023](../adr/0023-superficie-de-transporte-do-engine.md)
projetou-a — *"aceito — implementado em `crates/led-daemon` (GS1)"* — e a matriz exaustiva de
8 estados × 10 comandos está congelada desde a GS1.6.

O que resta desta tabela são **duas** lacunas reais — calibração ao vivo e grand master — e
nenhuma delas bloqueia o console. O blackout deixou de ser uma lacuna de **decisão** e passou
a ser uma de **implementação**.

## Mecânica do protocolo

### Transporte e autorização (do ADR-0014)

| Cenário | Transporte | Autorização |
|---|---|---|
| Mesmo host (autoria) | **Unix domain socket**, permissão de arquivo owner-only | credencial de SO do dono do socket |
| LAN (laptop → appliance) | TCP com **token e/ou mTLS**, bind em **interface específica** | token/certificado |
| Sempre | — | **nunca `0.0.0.0` por padrão** |

Read-model e comandos são **canais distintos**: ler nunca deve exigir a credencial de escrever.

### Enquadramento e versionamento

Uma mensagem por linha (JSON delimitado por `\n`), request/response correlacionados por `id`:

```jsonc
// handshake — primeira mensagem, obrigatória
{"v": 1, "id": 0, "cmd": "hello", "client": "lumyx-console/0.1"}
{"v": 1, "id": 0, "ok": true, "engine": "lumyx/…", "accepts": [1]}

// comando
{"v": 1, "id": 7, "cmd": "device.reboot", "args": {"device": 3}, "confirm": "<token>"}
{"v": 1, "id": 7, "ok": false, "error": {"code": "confirmation_required", "detail": "…"}}
```

- **`v`** é a versão do protocolo, negociada no `hello`. Versão desconhecida → **recusa
  explícita**, nunca best-effort (mesma regra do `schema_version` no ADR-0018).
- **`id`** correlaciona; respostas fora de ordem são permitidas.
- JSON hand-rolled, sem `serde` — convenção do workspace (`MetricsEmitter::snapshot_json`,
  `ReadModel::to_json`). Reavaliar se a superfície crescer.

### Modelo de erro

Códigos **enumerados**, nunca string livre: `unauthenticated`, `unsupported_version`,
`unknown_command`, `invalid_args`, `confirmation_required`, `refused_by_policy` (ex.: WiFi ao
vivo — ADR-0005), `device_not_connected`, `engine_busy`.

### Ações irreversíveis

`device.reboot`, `device.update_firmware`, `shutdown` e **blackout** exigem **duas fases**: o
engine responde `confirmation_required` com um token de uso único e curta validade; o cliente
repete o comando com `confirm`. Toda ação irreversível é **registrada** — alinhado à trilha
Ed25519/Provenance existente.

## Isolamento do hot-path

O handler de comandos roda em **thread de controle própria**. Comandos são aplicados **no
limite de frame** — nunca dentro de `send_frame`, `apply` ou do render. O plano de controle
não pode alocar no hot-path nem tomar o lock do `scratch`.

## Degradação segura

Canal caído → **o show continua**; o engine não depende do console para tocar. Comando
malformado ou não autenticado → **rejeitado e logado**, nunca aplicado pela metade. Sem
handshake, nenhum comando é aceito.

## Gates quando isto for implementado

- Teste negativo: comando **sem autenticação é recusado**.
- Teste negativo: `v` desconhecida é **recusada**, não degradada.
- Teste negativo: ação irreversível **sem `confirm` falha**.
- `/security`: nenhum bind em `0.0.0.0`; canal de controle sempre autenticado.
- `no_alloc` do output **permanece verde** com o canal de controle ativo.
- p99 de `send_frame` **inalterado** com comandos em trânsito.

## Fora de escopo

Descoberta do daemon na rede · multiusuário e papéis · calibração ao vivo · grand master.
