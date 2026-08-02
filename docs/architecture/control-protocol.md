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

### O que a UI vai precisar e o engine **NÃO TEM**

| O console vai querer | Existe? | Realidade |
|---|---|---|
| play / pause / stop do show | ❌ | `led-player` toca um arquivo **linearmente do início ao fim**; os `stop()` do codebase são desligamento de thread, não transporte |
| seek / scrub na timeline | ❌ | não há noção de posição de reprodução em runtime |
| carregar/trocar show sem reiniciar | ❌ | o caminho do `.lumyx` é argumento de startup |
| mudar calibração ao vivo | ❌ | `Hal::with_calibration` é **construtor**; não há setter em runtime |
| grand master / intensidade global | ❌ | não existe |
| blackout | ⛔ | **bloqueado** pelo [ADR-0017](../adr/0017-blackout-intencional-vs-heartbeat.md) |

**Consequência para o roadmap:** o console **read-only** (PRs 05–09: saúde, discovery, métricas)
é implementável hoje. Um console **de controle** exige antes uma **superfície de transporte no
engine** — que é trabalho de backend, não de UI, e ainda não foi projetada. Isso deve virar
ADR próprio antes de qualquer tela de transporte.

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

`device.reboot`, `device.update_firmware` e (futuramente) blackout exigem **duas fases**: o
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

Blackout (ADR-0017, adiado) · a **superfície de transporte do engine** (play/pause/seek — não
existe; precisa de ADR próprio) · descoberta do daemon na rede · multiusuário e papéis.
