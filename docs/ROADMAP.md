# LUMYX — Roadmap Mestre

> **Objetivo final.** Uma plataforma de iluminação por pixels que (a) substitua
> xLights/Vixen para shows de larga escala com garantias que eles não dão — determinismo,
> replay assinado, observabilidade, failover — e (b) viabilize **trajes de LED para
> performance de dança** com qualidade de palco.
>
> Este documento é o mapa completo: **o que já existe com evidência**, **o que falta**, e
> **em que ordem**, com o que bloqueia o quê.
>
> Data desta revisão: **2026-09-26** (factos de estado atualizados a **2026-09-29**) · base
> `57cf21d` **== `origin/baseline/f2-f71`**, `main` = `6ac22ff` (merge do #7; `git ls-remote`, 2026-09-29);
> **mergeados:** #8 (`a922e14`), #9 (`82e5ba1`), #11 (`0f7857e`), #13 (`445d296`), #12 (`207c87f`) e #7 (`6ac22ff`); **#10 fechado** sem merge · **1143 testes** em 106 suítes (macOS; **1139** no Ubuntu — os 4 do
> `network_guard` são macOS-only) · `led-core` **1.4.0** (constante de contrato; não re-medida
> nesta revisão)
>
> **O que é novo nesta revisão:** a **PARTE VI — Plano até 100 % operável**, com a definição
> de «100 %», marcos M0–M10 **em ordem ratificada pelo dono (2026-09-26, decisões D-A e
> D-B)**, passos numerados, tipo de gate por passo e o que só o humano mede. As Partes I–V foram corrigidas **só onde estavam factualmente caducadas** (números
> de testes/TD/ADR, F7.2, D4); o histórico de decisões não foi reescrito.
>
> *Revisão anterior (2026-08-05, HEAD `5416241`, 791 testes) ficou 8 semanas atrás. A
> anterior a essa (2026-08-03) listava como disponíveis frentes que já tinham sido
> entregues. Ver PARTE III.*

---

## Como ler

| Marca | Significado |
|---|---|
| ✅ | Feito **e verificado** — há teste, medição ou artefato citável |
| 🟡 | Parcial — funciona num caminho, falta noutro; a lacuna está nomeada |
| ⏳ | Pronto para executar, **bloqueado por recurso externo** (hardware, máquina, tempo de parede) |
| 🔴 | **Bloqueado por decisão** — não é trabalho, é uma escolha que ainda não foi feita |
| ⬜ | Não iniciado |

**Regra de evidência deste documento:** todo número aqui tem origem citada (arquivo, teste
ou artefato) e a **condição** em que foi medido. Número sem condição é número inútil —
"0,55 ms/frame" só significa algo com "6.200 px, release, macOS arm64". Onde não houve
medição, está escrito *não medido* — nunca estimado e apresentado como fato.

---

## O Golden Slice — o critério que ordena tudo abaixo

**Definição registada em 2026-08-05. É o significado canónico de "Golden Slice" no LUMYX.**

> **Golden Slice = vertical slice do produto.** O **menor fluxo completo** que atravessa
> toda a plataforma, do início ao fim, **funcionando em produção**.

```
Criar ou importar um show
        ↓
Editar na timeline
        ↓
Pré-visualizar
        ↓
Configurar controladores
        ↓
Enviar via Ethernet (DDP / Art-Net / sACN)
        ↓
Executar em hardware real
        ↓
Validar o resultado
```

**O que ele não é:** não é funcionalidade isolada e não é demonstração. É um **caminho
completo do operador** através de todas as camadas do sistema.

### Por que isto muda a leitura do resto do documento

Uma camada "✅" na tabela I.1 **não** significa que o Golden Slice avançou. O que conta é se
existe um **caminho contínuo** — e hoje não existe. Estado elo a elo:

| Elo | Estado | Onde quebra |
|---|---|---|
| Criar / importar show | ✅ | import xLights com gate de conflito (2.701 achados no projeto real), `RigBuilder`, `ShowIntent` |
| Editar na timeline | 🔴 | o motor existe (`led-sequencer`); **a interface não** — é o **D5**, e a FASE D já arrancou |
| Pré-visualizar | 🔴 | só GIF offline no `led-demo`; o preview do console é o **D4**, e será WebGPU (ver anexo do ADR-0016) |
| Configurar controladores | 🟡 | `led-hardware-profile` compila o layout, mas o WLED é configurado **à mão** (E6); o profile já declara **múltiplas portas** (FASE C, entregue) |
| Enviar via **Ethernet** | ✅ | validado em hardware a **2026-08-28**: **DDP e sACN aceites** (94/0 cada). **Art-Net não aceite** neste nó — o WLED faz bind de uma só porta de entrada, e sACN/Art-Net são mutuamente exclusivos nele |
| Executar em hardware real | 🟡 | **1 nó de 5** (720 px de 6.200), agora sobre Ethernet — falta energizar o rig (G2) |
| Validar o resultado | ✅ | replay por hash, Ed25519 com chave fixada, métricas ao vivo durante show real |

**Os dois bloqueios caíram, e nenhum era de código:** o **B2** foi decidido a 2026-08-09 e o
**Ethernet** foi validado em hardware a 2026-08-28. **Mas continua a não haver Golden Slice** —
o elo *pré-visualizar* está vazio. A diferença é o tipo de vão: era **decisão e recurso**,
passou a ser **trabalho por fazer** (o D4). O caminho do operador tem um vão, e é construível.

---

# PARTE I — Onde estamos

## I.1 — Maturidade por camada

| Camada | Estado | O que está provado | Lacuna nomeada |
|---|---|---|---|
| **Contratos / seams** (`led-core`) | ✅ | 9 contratos certificados, 5 **Frozen**, SemVer com guardião mecânico e negative control | — |
| **HAL** (`led-hal`) | ✅ | mapeamento aplicado **uma vez**, zero alocação no hot-path (contador real), heartbeat, NetworkGuard, calibração por-output | fan-out sequencial (ADR-0012, adiado até 2º nó) |
| **Layout** (`led-layout`) | ✅ | MegaTree, matriz serpentina, `RigBuilder` livre de conflito por construção | editor visual não existe |
| **Protocolos** (`led-protocols`) | ✅ | sACN unicast+multicast, Art-Net ArtDmx+ArtPoll, DDP, RouterDevice | RGBW-sobre-DDP `dtype 0x33` **não validado em hardware** |
| **Engine de render** (`led-pixel-engine`) | 🟡 | triple buffer Miri-limpo, pipeline, GPU compute (wgpu), efeitos como **funções puras** (ADR-0021) com gate de pureza e de alocação | **13 efeitos** contra ~40 do xLights — faltam ~25 (E1) |
| **Sequencer** (`led-sequencer`) | ✅ | Timeline não-destrutiva, clips, keyframes, blend, TempoMap, beat-sync, `ShowIntent` | sem UI |
| **Áudio** (`led-audio`, `audio-core`) | ✅ | Hann→FFT→bandas→flux-beat→BPM→seções musicais, zero-alloc, ring SPSC Miri-limpo | — |
| **Gravação / replay** (`led-show-recorder`) | ✅ | formato `.lumyx`, manifest, hash FNV-1a, Ed25519 **com chave fixada**, `bake` por traje + leitura em fluxo | playback **embarcado** (no traje) não existe — F3 |
| **Migração xLights** (`led-xlights`) | ✅ | import + gate de conflito + auto-fix + **export bidirecional** | `.fseq` **não existe** (interop FPP) |
| **Perfil de hardware** (`led-hardware-profile`) | ✅ | descritor de capacidades, validador, presets como **dado**, compilação, **portas físicas** (ADR-0030) | — |
| **Read-model** (`led-readmodel`) | ✅ | snapshot read-only, JSON à mão, bind loopback-only | nenhuma UI consome |
| **Console do operador** | ⬜ | — | **a maior peça faltante** (FASE D) |
| **Observabilidade** | ✅ | Prometheus + Grafana + 5 alertas + 4 SLOs, scrape ao vivo em show real | — |
| **Segurança** | ✅ | cosign, SBOM, attestation, Ed25519 pinado, red-team com achado CRITICAL fechado | — |
| **Governança** | ✅ | **31 ADRs**, ledger de TD com gate executável (hook de pre-commit), 27 agentes, guardiões mecânicos, CI verde | **nenhum ADR por decidir** — **B2** (0016) fechou a 2026-08-09 e **B1** (0017) a 2026-09-01 |
| **Hardware real** | 🟡 | **1 nó de 5** ponta-a-ponta (720 px de 6.200), **em Ethernet** desde 2026-08-28 | nós 2–5, Falcon, FPP, 72h (FASE G) |
| **Trajes de dança** | 🟡 | bifurcação **decidida** (ADR-0022: playback autônomo); `bake` por traje + playback em fluxo com pacing absoluto (F2, `9b89501`) | player embarcado (F3) e sync multi-traje (F4) não existem; autenticação pré-playback é **TD-013** |

## I.2 — Parâmetros medidos

Todos com origem e condição. **Nenhum destes é estimativa.**

### Latência e hot-path

| Parâmetro | Valor | Condição | Origem |
|---|---|---|---|
| Render+send p50 | **20.651 ns** | 100k iters, 300 px, SimulatorDevice, macOS **debug** | `led-hal/tests/bench_contention.rs` |
| Render+send p99 | **69.678 ns** | idem | idem |
| Sob contenção p50 | **23.558 ns** (×1,14) | + contender em loop apertado | idem |
| Sob contenção p99 | **1.419.228 ns** (×20,37) | idem — **1,42 ms < 5 ms de orçamento** | TD-011 |
| Custo da calibração | **+133.808 ns/frame** (×1,39) | 6.200 px / 37 universos, debug | ADR-0019 |
| — em % do orçamento | **~2,7 % de 5 ms** | idem | idem |
| Frame completo (release) | **0,55 ms** | 6.200 px, pipeline completo | `capacity_bench.rs` |
| Frame completo (release) | **23,05 ms** | **248.000 px** → 40 fps em CPU pura | `docs/capacity.md` |
| Player ao vivo p50/p99 | **0,5 ms / 4,1 ms**, 0 drops | show real, debug, scrape durante playback | `--metrics` |
| Alocações no hot-path | **0** em 10k frames | contador real, DDP e HAL | `no_alloc.rs` (2 crates) |

### Escala e capacidade

| Parâmetro | Valor | Nota |
|---|---|---|
| `CompiledLayout::compile` | 1k→**0,91 ms** · 6,2k→**5,37 ms** · 25k→**46 ms** · 50k→**142 ms** · 100k→**517 ms** | **O(n²) confirmado**; roda 1× no startup. TD-012 `wontfix` com gatilho >50k px |
| Rig real | **6.200 px** / 5 controladores / 28 universos por robô | projeto do usuário |
| Teto de CPU provado | **248.000 px @ 40 fps** | 40× o rig atual; gargalo é o transporte, não o software |
| DDP por pacote | **487 px** (RGB) / **365 px** (RGBW) | MTU 1462; a fragmentação respeita fronteira de pixel |
| Throughput do sender DDP | até **1593 fps**, 0 falhas | fire-and-forget: mede `sendto`, **não** exibição |

### Elétrica (RGBW — ADR-0020)

| Modo | mA/pixel (SK6812) | 720 px | vs RGB |
|---|---|---|---|
| RGB (3 canais) | 60 | 43,2 A | — |
| `WhiteMode::Min` (aditivo) | **80** | 57,6 A | **+33 %** |
| `WhiteMode::MinSubtract` (**padrão**) | **20** | 14,4 A | **−67 %** |

**Razão 4×** entre os dois modos para branco pleno — verificada por assert, não afirmada.
Derivada de corrente nominal por die; **não medida no rig**.

### Hardware real (2026-07-20 / 07-23, ESP32 DevKit V1 + WLED 16.0.1 + 720 px)

| Parâmetro | Valor |
|---|---|
| Primeira luz (DDP) | **94/94 frames, 0 falhas**, hash `0x23b8ee876a18e5a5` |
| Mini burn-in | **74/74 passes**, 0 aborts, 0 reset, 0 leak |
| Art-Net | **validado** — WLED reporta `lm:"Art-Net"`, `live:true` |
| sACN | ❌ **bloqueado no firmware** — WLED 16.0.1 não faz bind na :5568 (provado por ICMP + sender de referência independente) |
| Burn-in WiFi | 45 passes limpos, **abort no 46** (1 falha de `sendto`, provável ENOBUFS) |
| Ping WiFi | **99 ms médio / 146 ms pico / jitter 31 ms** com RSSI −44 |

> O jitter de 31 ms com sinal forte é a **confirmação empírica** do ADR-0005: WiFi é
> proibido ao vivo. Foi pior que a estimativa original (5–50 ms).

### Migração xLights

| Parâmetro | Valor |
|---|---|
| Projeto real importado | **430 modelos**, 5 controladores, **6.200 px** |
| Conflitos de canal detectados | **2.701** |
| Modelos corrigidos pelo auto-fix | **425** → 0 conflitos, original intacto |
| Replay do show real | hash `0xd8f1479ff3645e1e` estável em todos os passes |

### Spike de UI (2026-07-30)

| Eixo | React/Vite | Leptos/WASM |
|---|---|---|
| Build | **1,64 s** | **44,98 s** (debug, wasm32) |
| Bundle | **47 kB** gzip | não empacotado (falta `trunk`) |
| axe-core | **0 violações**, 37 regras aprovadas | não medido |
| Canvas2D, 10k pontos | **3 fps** ← o achado que decide | mesma abordagem |
| Leitor de tela real / DX | **só o humano mede** | **só o humano mede** |

> **3 fps em Canvas2D com 10k pontos** significa que o preview **tem** que ser WebGPU —
> não é preferência, é requisito. Isso já está decidido pela medição.

## I.3 — Contratos e governança

- `led-core` **1.4.0** · **61 itens** de superfície pública · baseline SemVer **commitado**
- **5 seams Frozen**: `ProtocolOutput`, `DeviceDriver`, `IDevice`, `CompiledLayout`, `UniverseData`
- `ColorFormat` é **Evolving** (foi o que permitiu RGBW e vai permitir RGB+CCT)
- **31 ADRs** · **20 entradas de TD** (`scripts/audit_gate.py`, 2026-09-25): **11 fechadas**
  com evidência auditável (incl. TD-020), 1 `pending-verification` (**TD-022**), 2 `wontfix`
  com gatilho de revisita (TD-011, TD-012), **6 abertas** (TD-013, TD-014, TD-017, TD-018,
  TD-019, TD-021). Nenhuma em `diagnosed`.
- CI (run `36226221020`, PR #6, HEAD `320ff94`, **lido no log** job a job):
  `test (ubuntu-latest)` **105 suítes · 1131/0/9**, `test (macos-latest)` **105 · 1135/0/9**
  (delta 4 = `network_guard` macOS-only), clippy `--all-targets --locked -D warnings`
  **correu** nos dois (não `skipped`), `miri (led-triple)` **7/0/0, exit 0**, `contrato TS`
  verde; `windows (allow-failure)` falha por TD-021 (esperado). O agregado «success» vem do
  `allow-failure`, não de todos passarem. O Miri é gate bloqueante desde `2e603c9`.
- **19 crates** em `crates/`: 18 com `src/lib.rs` (3 deles também binários —
  `led-console-bin`, `led-daemon-bin`, `led-player`), `led-demo` só binário; `led-triple` (o triple buffer, `unsafe` isolado)
  foi extraído do `led-pixel-engine`, que ficou com **0** `unsafe`. Construções `unsafe` em
  `src/`: **8** (`led-triple` 5, `audio-core` 3); `led-hal` passou de 4 a 0 em `320ff94`.

**Gates executados nesta revisão (2026-09-25, exit lido sem pipe — KB-013):**

```
scripts/baseline_watch.sh  (cargo test --workspace)   → EXIT 0 · 105 suítes · 1135 passed · 0 failed · 9 ignored · 481 s
cargo test -p led-hal -p led-console-bin  ×3           → EXIT 0 ×3 · 228 passed, 21 suítes
cargo clippy --workspace --all-targets --locked -- -D warnings   → EXIT 0 (80 s, 0 warnings)
python3 tests/test_audit_gate.py                       → 10 passed · 0 failed
```

> **Nenhum tempo medido em 2026-09-22/24 vale** — correram sob carga 64–96 (contaminada).
> Os 481 s acima foram com load ~4; incluem parte da compilação e **não** são um baseline
> de desempenho.

### ADRs — estado

| ADR | Assunto | Status |
|---|---|---|
| 0001–0010 | replay, ArcSwap, DDP, Ed25519, WiFi, ShowIntent, seams, triple buffer, chaos, cluster | ✅ aceitos e implementados |
| 0011 | `ColorFormat` RGBW no mapper | ✅ |
| 0012 | fan-out paralelo | ✅ aceito, **implementação adiada** até 2º nó físico |
| 0013 | engine em daemon separado | ✅ aceito e **implementado** (`led-daemon-bin`, GS2) |
| 0014 | IPC e segurança UI↔engine | ✅ aceito, **UDS implementado** (GS3); auth de LAN continua vazia |
| 0015 | preview lossy fora do hot-path | ✅ aceito, ⬜ **não implementado** |
| 0016 | stack do console | ✅ **aceito (2026-08-09)** — **React + TypeScript**, com os tipos GERADOS do Rust |
| 0017 | blackout × heartbeat | ✅ **aceito (2026-09-01)**, ✅ **máscara implementada (2026-09-04, `1030a7e`)** no `OutputManager`, a jusante de `record()` — decisões 1–5, 7 e 8, cada uma com teste; ⬜ decisão **6 por implementar** (exige `PROTOCOL_V = 2`; ver ADR-0031); ⬜ **9.C por medir** (rig). O **D6** — o botão no console — continua aberto |
| 0018 | HardwareProfile | ✅ implementado (5 slices) |
| 0019 | calibração por-output no HAL | ✅ |
| 0020 | `WhiteMode::MinSubtract` | ✅ |
| 0021 | efeito é **função pura**, estado derivado nunca armazenado | ✅ implementado (E1, 1ª fatia) |
| 0022 | traje de LED: playback autónomo + sync determinístico | ✅ aceito (`docs/adr/0022-wearable-playback-autonomo-sync-deterministico.md:3`) · ⬜ **sem código neste repo** — não há crate; as únicas ocorrências de «wearable» são lições citadas em comentários (`crates/led-daemon-bin/src/loader.rs:10`) |
| 0023 | superfície de transporte do engine (8 estados) | ✅ aceito e **implementado** (`docs/adr/0023-superficie-de-transporte-do-engine.md:3` · `crates/led-daemon/src/lib.rs:71`); contrato **congelado** na GS1.6 (`docs/adr/0023-anexo-tabela-de-contrato.md:8`) |
| 0024 | fronteira de validação do `HardwareProfile` | ✅ aceite (`docs/adr/0024-fronteira-de-validacao-do-hardwareprofile.md:3`) e **implementado** — o daemon chama `validate` ao construir a saída (`crates/led-daemon-bin/src/output.rs:471`) |
| 0025 | `refresh_hz` é um **limite**, não uma recomendação | ✅ aceite (`docs/adr/0025-refresh-hz-e-a-cadencia-pedida.md:3`) e **implementado** — o tecto é lido do profile e o daemon recusa ultrapassá-lo (`crates/led-daemon-bin/src/output.rs:671`) |
| 0026 | fronteira console↔daemon (o console é **cliente** do IPC v1) | ✅ aceite (`docs/adr/0026-console-daemon-boundary.md:3`) e **implementado** — `led-console-bin` serve HTTP (`crates/led-console-bin/src/http.rs:75`) |
| 0027 | contrato TypeScript **gerado** do Rust | ✅ aceito (`docs/adr/0027-contrato-tipos-rust-typescript.md:3`) e **implementado** — o gerador é o caminho A (`crates/led-console-bin/src/contract.rs:89`) |
| 0028 | topologia da Web Platform e a fronteira de estado | ✅ aceito (`docs/adr/0028-web-platform-topology-and-state-boundary.md:3`) e **implementado** — `console-web/`, com o único `fetch` em `console-web/src/transport/api.ts:141` |
| 0029 | saída multi-controlador (N nós, um mapa) | ✅ aceito (`docs/adr/0029-saida-multi-controlador.md:3`) e **implementado** — `OutputConfig.alvos` (`crates/led-daemon-bin/src/output.rs:247`) |
| 0030 | portas físicas: a porta é subdivisão de **endereçamento** | ✅ **implementado** na FASE C (`docs/adr/0030-portas-fisicas-subdivisao-de-enderecamento.md:3`) — `Capabilities.ports` (`crates/led-hardware-profile/src/lib.rs:126`) e o daemon a pedir a repartição ao dono (`crates/led-daemon-bin/src/output.rs:210`). Fica aberta, **por decisão**, a pendência do §5 (`max_pixels % ports != 0`), com o gatilho por disparar; e as portas continuam **NÃO MEDIDO** em hardware |
| 0031 | negociação de versão no handshake (o `hello` viaja em `v:1`) | ✅ aceito (`docs/adr/0031-negociacao-de-versao-no-handshake.md:3`) · 🟢 **a metade emissora da decisão 2 está implementada** — o `accepts` é **derivado** de `SUPORTADAS` (`crates/led-daemon-bin/src/proto.rs:25`), no `hello` (`crates/led-daemon-bin/src/server.rs:306`) e na recusa por versão (`crates/led-daemon-bin/src/proto.rs:176`) · 🟡 a decisão 2 **não fecha** enquanto o daemon não **ler** o `accepts` do pedido · ⬜ **decisões 1, 3, 4, 5, 6 e 7 por implementar** — exigem estado de versão **por ligação** e uma segunda versão suportada; `SUPORTADAS` tem um só elemento e `PROTOCOL_V` continua 1 (`crates/led-daemon-bin/src/proto.rs:33`) |

---

# PARTE II — O que falta

## FASE B — Decisões que estavam bloqueando código ✅ *as duas fechadas*

**Nada aqui era trabalho de programação. Eram duas escolhas, e as duas foram feitas.**

### B1 — ADR-0017: semântica do blackout ✅ *FECHADO em 2026-09-01*

**A pergunta era:** o operador aciona blackout. O heartbeat dispara em seguida. O que vai no fio?

**A resposta desfez a premissa da pergunta.** O dilema assumia que a máscara de blackout e o
armazenamento do último frame vivem na **mesma camada** — e por isso obrigava a escolher entre
gravar preto e não gravar. Não vivem: a máscara é aplicada **a jusante** de
`heartbeat.record()`, que guarda sempre o frame **real**. O preto persiste sem nunca ser
gravado, e o invariante *"o heartbeat nunca envia um frame zerado"* continua **literalmente**
verdadeiro — quem zera é a máscara, e a máscara é comandada.

**Onde a máscara vive: no `OutputManager`, antes do fan-out** — um ponto, três protocolos.
Isto **corrige** o desenho anterior desta secção, que dizia *"máscara no HAL"*: a Emenda 1 do
ADR-0019 invalidou esse sítio, porque o `DdpOutput` contorna o HAL. Pô-la lá daria blackout em
Art-Net e sACN e **nenhum no DDP**, que é o protocolo validado em hardware.

As dez decisões — incluindo o **escape por device** como requisito normativo, o fade
instantâneo e o **fail-safe por nó** no cluster — estão em
[ADR-0017](adr/0017-blackout-intencional-vs-heartbeat.md).

> **Uma metade da decisão do cluster é requisito, não garantia.** Que o firmware do
> controlador entre em blackout ao perder o link **não está medido**. É o requisito 9.C do
> ADR, e entra na fila de validação física ao lado de G3–G7.

**Desbloqueia:** o **D6** — botão de blackout no console, com confirmação em duas fases.

### B2 — ADR-0016: stack do console ✅ *FECHADO em 2026-08-09*

**Decisão: React + TypeScript** ([ADR-0016](adr/0016-stack-console-provisorio.md)), com uma
obrigação inseparável: **o frontend não contém nenhum enum escrito à mão** que espelhe
`EstadoUi` ou `Elo` — os tipos são **gerados** do Rust e um gate reprova a CI se divergirem
([ADR-0027](adr/0027-contrato-tipos-rust-typescript.md)).

A evidência que fundamentou a decisão, e as medições posteriores que a confirmam sem a
reabrir, estão no [anexo de evidência](adr/0016-anexo-evidencia-e-matriz.md).

**O que a medição decidiu à parte da stack:** o preview **será WebGPU**. Os 3 fps são de um
Canvas2D com 10k `fillRect` — propriedade do **desenho do preview**, não de nenhuma stack.

**Desbloqueou:** a FASE D, que arrancou e está na Web Platform Phase 2 (ver abaixo).

---

## FASE C — HardwareProfile: múltiplas portas físicas ✅ *ENTREGUE em 2026-09-01*

**Achado sustentado** da revisão externa, com o diagnóstico refinado: `PixelPhysical.format`
**já é por-pixel**, então o `CompiledLayout` já consegue expressar portas com formatos
diferentes. Quem achata é apenas o **descritor de design-time**.

> **O contrato fechou em 2026-08-30 e o código chegou a 2026-09-01 —
> [ADR-0030](adr/0030-portas-fisicas-subdivisao-de-enderecamento.md), hoje `implementado`.**
> O modelo de porta que esta secção propunha foi **rejeitado por evidência**; está registado,
> na íntegra e com a razão, em *Alternativas rejeitadas* do ADR. A tabela abaixo é o contrato;
> os `§` remetem para ele.

| Slice | Conteúdo | Estado |
|---|---|---|
| C1 | `ports` — uma **contagem** — entra em `Capabilities` (§5). `color` (§2) e `calibration` (§3) **não** entram na porta; `pixel_offset`/`pixel_count` são **derivados, nunca declarados** (§4) | ✅ `led-hardware-profile/src/lib.rs:126` |
| C2 | A repartição ganha **um só dono**, o `led-hardware-profile` (§6), e o **daemon passa a consumi-lo** (§8) — era o slice de risco **alto**, por tocar o caminho validado em hardware | ✅ dono em `reparticao.rs:95`; daemon a pedir por `compile_layout_de` (`led-daemon-bin/src/output.rs:593`) |
| C3 | Presets ganham portas — Falcon F16V3 tem **16** portas | ✅ Falcon e Advatek a 16; os outros seis em `ports: 1` **não por omissão** — o repositório não determina a contagem deles |
| C4 | Guardião: 9º check — porta não pode vazar para o runtime (§7) | ✅ pela **direcção da dependência**, não por scanner textual |

**O que a FASE C não fechou, e não é dívida por esquecimento:** a pendência do §5
(`max_pixels % ports != 0`) continua **por decidir**, com o gatilho por disparar — implementar
uma política por omissão está proibido pelo critério de reversão do ADR. E as portas continuam
**NÃO MEDIDO** em hardware: nenhum controlador multi-porta foi observado, o `16` é folha de
catálogo, e os testes de bytes no fio usam apenas presets de porta única.

**Não toca nenhum seam Frozen** (§9): `led-core` fica intocado e sem bump.

---

## FASE D — Console do operador 🟡 *em curso — Web Platform fechada até `load`/`unload`*

Hoje o LUMYX é **CLI + biblioteca + um console web em construção**; xLights e Vixen são
**aplicativos**. O console é o que torna a plataforma usável por quem não escreve Rust, e
**já não está bloqueado**: B2 fechou a 2026-08-09.

**Último ponto fechado:** `load`/`unload` na UI (`1876d52`, 2026-08-14) — o browser deixou
de ser um telecomando e passou a gerir shows. A arquitetura está em
[ADR-0028](adr/0028-web-platform-topology-and-state-boundary.md) (topologia e fronteira de
verdade) e o contrato de tipos em [ADR-0027](adr/0027-contrato-tipos-rust-typescript.md).

| PR | Conteúdo | Estado · depende de |
|---|---|---|
| D1 | **Daemon**: engine headless, processo separado (ADR-0013) | ✅ **feito** (GS2) |
| D2 | **IPC**: UDS owner-only, comandos tipados e versionados, nunca `0.0.0.0` (ADR-0014) | ✅ **feito** (GS3) |
| D3 | **Shell do console**: HTTP + SSE + AppShell + design system + transporte + `load`/`unload` | ✅ **feito** |
| D4 | **Preview WebGPU**: cópia downsampled, rate-limited, **lossy por contrato** (ADR-0015) | ⬜ **desbloqueado, não iniciado.** Mecanismo decidido: **evento no canal `subscribe`** (ADR-0015 Emenda 1); o ADR-0027 Emenda 4 classifica evento novo como **aditivo**, logo **não** depende do `PROTOCOL_V=2`. O bloqueio real é que o «fora do hot-path» **não existe no daemon**. A pré-condição `led-triple` (extracção + Miri limpo) está **feita** |
| D5 | **Timeline visual**: waveform de áudio, clips, keyframes — o `led-sequencer` já tem o modelo | ⬜ depende de D3 |
| D6 | **Blackout**: botão + confirmação em duas fases + log auditável. **Sem atalho de teclado nesta fatia** (ADR-0017, decisão 10) | ⬜ **desbloqueado, não landado** — o B1 fechou a 2026-09-01. A pré-condição **escape por device** deixou de faltar: aterrou com a máscara em `1030a7e` (2026-09-04). O bloqueio actual é outro — a decisão 6 (duas fases) exige `PROTOCOL_V = 2` e `PROTOCOL_V` é 1; ver ADR-0031 — **aceite**, com a metade **emissora** da decisão 2 já implementada (o `accepts` derivado de `SUPORTADAS`), e as decisões 1 e 3–7 **por implementar**: é o estado de versão **por ligação** e a segunda versão em `SUPORTADAS` que ainda faltam, não o documento |
| D7 | **Editor de layout**: desenhar modelos, posicionar no palco | ⬜ depende de D3 |
| D8 | **Empacotamento**: app desktop com webview do SO | ⬜ depende de D3, D4 |

> **O control-plane deixou de estar vazio.** A lacuna que a especificação
> (`docs/architecture/control-protocol.md`) nomeava — *"não há o que comandar"* — fechou:
> o `ShowRuntime` (ADR-0023) dá o estado controlável, o IPC v1 dá o comando, e a UI já o
> exercita. O que resta abaixo são pendentes concretos, não ausência de fundação.

### Pendentes actuais da FASE D (Faixa A)

| # | Pendente | Onde está registado |
|---|---|---|
| 1 | **TD-014** — `console.dropped` tem contador e o [ADR-0026](adr/0026-console-daemon-boundary.md) §13 exige que a perda seja **reportada**; não há rota até ao operador | `docs/technical-debt-ledger.md` (aberto, Medium) |
| 2 | **`/api/profiles` devolve 501** — à espera de uma de duas decisões de arquitectura | changelog 2026-08-10 (F7) |
| 3 | ~~**F7.2 Ubuntu não fecha**~~ ✅ **fechado em 2026-09-08** — a causa era o **gate** (contaminação por thread), não o caminho DDP; PR #4 mergeado, `test (ubuntu-latest)` verde lido **no log** | changelog 2026-08-13d → PR #4 |
| 4 | **Confirmação de `load` não medida com leitor de ecrã** — é um segundo clique no mesmo botão, e num teclado sem foco visível é menos óbvio do que devia | changelog 2026-08-14 |
| 5 | **`path` sem histórico nem completação** — o operador escreve o caminho inteiro de cada vez | changelog 2026-08-14 |

---

## FASE E — Paridade e superação do xLights ⬜

Gap analysis honesto. Isto é o que **eles têm e nós não**.

| # | Lacuna | Estado hoje | Peso |
|---|---|---|---|
| E1 | **Biblioteca de efeitos** | 🟡 **13** — 5 base + 8 novos (`Chase`, `Twinkle`, `Fire`, `ColorWash`, `Strobe`, `Meteor`, `Lightning`, `Ripple`) sob o ADR-0021, com gate de alocação próprio. Faltam ~25 para paridade | 🔥 alto |
| E2 | **Preview 3D** | preview 2D só em `led-demo` (GIF), Z ignorado | alto |
| E3 | **Editor de layout visual** | só código e import de XML | alto |
| E4 | **Mídia mapeada em pixels** (vídeo→pixel) | não existe | médio |
| E5 | **Export `.fseq`** (interop FPP) | **não existe** — verificado, zero ocorrências no repo | médio |
| E6 | **Upload de config para o controlador** | não existe; hoje se configura o WLED à mão | médio |
| E7 | **Scheduler / playlist** | `--loop` no player; sem agendamento | baixo |
| E8 | **Faces / letras / canto sincronizado** | não existe | baixo |

**Onde já superamos:** determinismo verificável por hash, replay assinado com chave fixada,
observabilidade Prometheus, failover de cluster, chaos testing, SBOM+attestation, gate de
conflito de canais que o próprio xLights não tem (2.701 conflitos achados no projeto **deles**).

---

## FASE F — Trajes de LED para dança 🟡 *bifurcação decidida, execução em curso*

> **Atualização 2026-08-05.** O título desta fase dizia *"bifurcação não decidida"* e a
> primeira linha dizia *"ainda não tem ADR"*. **As duas caducaram:** o [ADR-0022](adr/0022-wearable-playback-autonomo-sync-deterministico.md)
> foi aceito (caminho **(a)**, playback autônomo) e o F2 já landou (`9b89501`). O conflito
> abaixo fica registado porque é o **porquê** da decisão, não uma pergunta em aberto.

### O conflito

O LUMYX hoje é **streaming**: engine → rede → controlador → pixel, frame a frame.
Um traje de dança **não tem cabo**. E o ADR-0005 proíbe WiFi ao vivo — **com medição
própria que confirma o porquê** (jitter 31 ms, e um `sendto` falhando a cada ~6 min).

Streaming sem-fio para um traje de palco é, pela nossa própria evidência, **inviável**.

### As duas saídas

| Caminho | Como funciona | Custo |
|---|---|---|
| **(a) Playback autônomo + sync** ← *recomendação* | O show é **assado** (`bake`) e gravado no controlador do traje. Cada traje toca sozinho; o sincronismo vem de um **start comum + relógio**, não de streaming. | player embarcado, formato de bake, disciplina de drift |
| (b) Streaming sem-fio dedicado | Rádio dedicado (ESP-NOW, ISM, W-DMX) em vez de WiFi | latência/jitter precisam ser **medidos**, não presumidos; risco de palco alto |

**Por que (a):** é como o estado da arte do setor funciona, elimina a dependência de rádio
durante o número, e **o LUMYX já tem 80 % das peças**: `.lumyx` é um formato de show
gravado, o replay é determinístico e verificado por hash, e o Ed25519 já garante
autenticidade. Falta o **outro lado** — tocar isso dentro do traje.

### Trabalho da fase

| # | Item | Nota |
|---|---|---|
| F1 | **ADR: wearable autônomo × streaming** | ✅ **feito.** **[ADR-0022](adr/0022-wearable-playback-autonomo-sync-deterministico.md) aceito.** 8 decisões, 10 critérios, 8 gates; **as 4 questões foram decididas** (falha apaga c/ estado declarado · sem rádio no caminho crítico · orçamento = <1 quadro, duração sai do G6 · fork replay×render) |
| F2 | **`bake`**: show → artefato que roda no controlador | 🟡 **1ª fatia feita** (`9b89501`): `bake` por traje em **fluxo** (pico de memória independe da duração), mesmo formato `.lumyx` com menos pixels, faixas de pixels como **dado**; recusa faixa vazia/degenerada/fora de alcance/**sobreposta**; teste de não-vazamento com marcador proibido. `play_streaming_unverified` com `Pacing::Absolute` (quadro superado é **descartado**, nunca empurra o erro). ⚠️ **É fundação de BANCADA**: autenticação pré-playback não existe — **TD-013**. O fork replay × render-a-bordo (Q4) continua em aberto.
| F3 | **Player embarcado** | firmware ou WLED preset — decisão de plataforma, **fora do escopo do ADR-0022** |
| F4 | **Sync multi-traje**: start comum + medição de **drift** ao longo do número | `net_time` já resolve o análogo cabeado (±10 ms medido). **Achado do scan F1:** o `led-player` hoje é *livre-corrente* — precisa de pacing por instante absoluto (D3) |
| F5 | **Orçamento wearable**: bateria, corrente, peso, calor, segurança de contato | aqui `MinSubtract` já paga: **−67 % de corrente** no branco |
| F6 | **Degradação segura**: um traje que falha não pode derrubar o número | análogo ao failover de cluster |

> **`MinSubtract` já foi a primeira entrega desta fase sem que ela existisse.** Numa fita
> de 720 px, ele é a diferença entre **57,6 A** e **14,4 A** — e num traje isso é a
> diferença entre viável e impossível de carregar nas costas.

---

## FASE G — Certificação de produção ⏳ *bloqueada por recurso externo*

Tudo aqui tem **comando pronto e ensaiado**. Nada depende de escrever código.

| # | Item | O que destrava | Comando |
|---|---|---|---|
| G1 | ✅ **Migração WiFi → Ethernet** | — | **feito em 2026-08-28**: latência e jitter medidos, ENOBUFS com causa identificada (adaptador USB) |
| G2 | **Nós 2–5** (6.200 px completos) | energizar o rig | `led-player robot_sequence.lumyx --ddp <ip>` |
| G3 | **Burn-in 72 h** → 168 h | lançar **fora da sessão** | `launchctl load ~/Library/LaunchAgents/com.lumyx.burnin.plist` |
| G4 | **Falcon / FPP** | ter o controlador | mesmo player, `--artnet`/`--ddp` |
| G5 | **Determinismo Linux/Windows** | máquina ou CI | `./scripts/determinism_probe.sh` |
| G6 | **Chaos físico** | rig + puxar o cabo | burn-in rodando + desconectar ETH |
| G7 | **RGBW `dtype 0x33` no DDP** | fita RGBW no rig | o validador já **avisa** que não foi validado |
| G8 | ✅ **sACN em hardware** | — | **feito em 2026-08-28** (94/0, `lm:"E1.31"`): o ESP32-POE faz bind na 5568. **Art-Net ficou não aceite neste nó** — bind exclusivo, não é defeito do LUMYX |

---

## FASE H — Distribuição ⬜

O que transforma "meu projeto" em "plataforma que outros usam".

| # | Item |
|---|---|
| H1 | Instalador / binários assinados por plataforma (cosign já roda) |
| H2 | Documentação de usuário (hoje a doc é de arquiteto, não de operador) |
| H3 | Licença e modelo de distribuição |
| H4 | Guia de migração xLights → LUMYX (o código já faz, falta o texto) |
| H5 | Catálogo de presets de hardware da comunidade (a tabela já é dado — cada placa é **uma linha**) |

---

# PARTE III — Caminho crítico

> **Substituída, para planeamento, pela [VI.4](#vi4--dependências-entre-marcos)** — ordem
> ratificada pelo dono a 2026-09-26 (decisões D-A e D-B, VI.0). O diagrama e a lista abaixo
> ficam como **registo histórico** da revisão de 2026-08/09 e não foram reescritos; onde
> divergirem da VI.4, vale a VI.4.

```
        ┌── B1 ✅ ────────────────────┐   (decidido 2026-09-01 — ADR-0017 aceito)
        │                             ▼
HOJE ───┤                          D6 blackout no console  ◄── desbloqueado
        │
        ├── B2 ✅ ── D1 ✅ ── D2 ✅ ── D3 ✅ ─┬─ D4 preview ─ D8 app
        │                                     ├─ D5 timeline
        │                                     └─ D7 layout
        │
        ├── C ✅ portas múltiplas ───► (entregue 2026-09-01 — ADR-0030)
        │
        ├── E2..E8 paridade xLights ─► (E1 efeitos: 1ª tranche FEITA — 13 de ~40)
        │
        ├── F1 ✅ ─ F2 🟡 ─ F3 player ─ F4 sync ─ F5 orçamento ─ F6 degradação
        │
        └── G1 ✅ G8 ✅ · G2..G7 ────► (bloqueada por hardware/tempo, não por código)
```

> **Correção de 2026-08-05.** A versão anterior deste diagrama listava **E1** e **F1** como
> frentes disponíveis. **As duas já tinham sido entregues** — E1 na 1ª tranche de efeitos
> (13 de ~40, ADR-0021) e F1 no ADR-0022. Priorizar sobre o mapa antigo mandaria refazer
> trabalho concluído.

**O que pode correr hoje, sem esperar por decisão:**

1. **D6** — blackout no console. **Desbloqueado a 2026-09-01**; a semântica está fixada pelo
   ADR-0017 e o desenho não tem lacunas. É a frente com o caminho mais curto
2. **D4 / D5 / D7** — preview, timeline e editor de layout. São **superfície** sobre domínio
   que já existe; nenhum abre arquitectura nova
3. **E1 (continuação)** — ~25 efeitos para paridade; o molde (`ComputeKernel` + ADR-0021)
   já existe, cada efeito é aditivo e testável
4. **F4** — sync multi-traje (`SharedClock` e `net_time` já existem). **F3 não**: é *decisão
   de plataforma* (firmware próprio × preset WLED), explicitamente fora do ADR-0022 e precisa
   de ADR próprio antes de qualquer código

**Nenhuma frente está parada à espera de decisão.** B2 fechou a 2026-08-09, a FASE C foi
entregue a 2026-09-01 e o B1 foi decidido no mesmo dia. O que resta são decisões **menores**,
listadas onde aparecem: TD-017 (política do universo), `/api/profiles` (IPC v2 ou catálogo no
console), o comportamento do preview sem WebGPU, o F3 e a licença.

**O que continua parado é recurso, não decisão:** o rig por energizar.

---

# PARTE IV — Riscos

| Risco | Probabilidade | Impacto | Mitigação |
|---|---|---|---|
| **Escopo do console** — D é maior que tudo que foi feito até agora | alta | alto | fatiar por PR com gate; D1–D3 entregam valor antes do resto |
| **Trajes exigem firmware embarcado** — competência diferente da do repo | alta | alto | F1 ✅ decidido (ADR-0022) **antes** de codar, como previsto. O risco **migra para F3**: a escolha firmware próprio × preset WLED continua aberta e está fora do escopo do ADR-0022 |
| **Paridade de efeitos é trabalho longo e repetitivo** | alta | médio | o `ComputeKernel` já é o molde certo; cada efeito é aditivo e testável |
| **Rig continua offline** | média | alto | tudo que era gateável sem hardware **já foi feito** — a fila G está pronta, só falta energia |
| **O `compile` O(n²)** morde acima de 50k px | baixa | médio | TD-012 com gatilho e guarda falsificável que roda sempre |
| **Windows nunca fica verde** | média | baixo | não-bloqueante por ADR-0013; não orienta o design |

---

# PARTE V — Onde estamos, em uma frase

**O motor está pronto e provado; o produto ainda não tem rosto — mas já não há nada a
decidir para lho dar.**

O núcleo — determinismo, contratos, protocolos, áudio, replay, segurança, observabilidade —
está em estado que xLights e Vixen não alcançam. O que falta é quase tudo **acima** do
motor: o console que torna isso operável e os efeitos que tornam isso expressivo. A decisão
de wearable, que era a terceira lacuna, **foi tomada** (ADR-0022) e está em execução.

**O que mudou desde a versão anterior desta secção:** os dois vãos que ela nomeava eram de
**decisão** e de **recurso**, e os dois fecharam — B2 a 2026-08-09, Ethernet a 2026-08-28,
B1 a 2026-09-01. Medido contra o Golden Slice **continua a faltar um elo**: *pré-visualizar*
não existe (D4). Mas é a primeira vez que o que falta é **só trabalho** — nenhuma frente
espera por uma escolha, e nenhuma espera por hardware que não seja energizar o rig.

**A afirmação que este documento não faz:** que o rig completo funciona. **1 nó de 5**,
720 px de 6.200. Isso é a FASE G, e depende de energia, não de código.

---

# PARTE VI — Plano até 100 % operável

> **Estado desta parte (2026-09-26):** a **ordem dos marcos** e as decisões **D-A** e **D-B**
> foram **ratificadas pelo dono** (ver VI.0). O **C8** (instalação permanente) continua
> **PROPOSTA**. A definição de «100 %» (C1–C9) foi escrita por mim a partir das Partes I–V e
> não foi ratificada critério a critério — só a inclusão dos trajes (D-B). O que estiver
> marcado 🟣 é decisão que **não** tomei.

## VI.0 — Decisões ratificadas

| # | Decisão do dono (2026-09-26) | Consequência neste plano |
|---|---|---|
| **D-A** | **O console pode vir antes da certificação no rig.** Substitui a premissa «hardware antes de features» da revisão de 2026-07-12. **Nenhum marco é «pronto para show» antes do marco do rig (M6).** | M1–M5 avançam sem esperar pelo rig; o rig é **trilha humana paralela desde já**, não bloqueante até ao M6. Tudo o que M1–M5 entregam fica, até ao M6, com o rótulo **«não pronto para show»** |
| **D-B** | **Os trajes (FASE F, F3–F6) fazem parte do «LUMYX 100 %».** | Entra o critério **C9**; o M5 (ADR do F3) e o M8 (F3–F6) passam a estar **dentro** do 100 % |

## VI.1 — O que «100 % operável» significa (critérios mensuráveis)

O roadmap anterior não definia o ponto de chegada. Só há «100 %» quando **todos** os critérios
abaixo tiverem evidência citável, cada um com o seu artefacto:

| # | Critério | Prova exigida | Quem mede |
|---|---|---|---|
| **C1** | **Golden Slice completo**: um operador, **sem escrever Rust**, cria/importa → edita → pré-visualiza → configura → envia → executa → valida | run registado no rig real, com o `.lumyx` e o hash | 🔵 humano + rig |
| **C2** | **Rig completo por cabo**: 5 nós, **6.200 px**, show de 3 min, **p99 de latência medido** | scrape de `--metrics` + hash de replay estável | 🔵 rig |
| **C3** | **Certificação de longa duração**: burn-in **≥ 72 h limpo** (alvo 168 h) | `burnin-*.jsonl` + hash em `docs/certification/` | 🔵 rig + tempo |
| **C4** | **Console do operador**: D3–D7 entregues, cada um com gate; acessibilidade **medida com leitor de ecrã real** | CI verde + relatório de a11y | 🟢 gates · 🔵 a11y/GPU |
| **C5** | **Operação segura**: blackout com confirmação em duas fases e log auditável; `--require-all`; **TD-013** (autenticação pré-playback), **TD-014** (perda de frames chega ao operador) e **TD-017** (política do universo) fechados; requisito **9.C** (firmware em blackout ao perder o link) **medido** | testes + medição no rig | 🟢 · 🔵 |
| **C6** | **Higiene de engenharia**: CI Linux + macOS + Miri + contrato TS verdes; **todo** o TD aberto ou fechado ou `wontfix` com gatilho | `audit_gate.py` + run da CI **lido no log** | 🟢 |
| **C7** | **Instalável e documentado**: binários assinados, documentação de **operador**, licença decidida | artefactos assinados (cosign) + docs | 🟠 · 🟣 |
| **C8** | **Instalação permanente** (**PROPOSTA**): scheduler/playlist por horário e dia | teste de agendamento + run longo | 🟢 · 🔵 |
| **C9** | **Trajes** (**D-B**): F3–F6 entregues; os **critérios de validação** e os **gates** do [ADR-0022](adr/0022-wearable-playback-autonomo-sync-deterministico.md) cumpridos, mais os do ADR do F3 (M5) | evidência por critério/gate do ADR-0022, medida em traje real | 🟢 · 🔵 traje |

**C8 continua PROPOSTA** — vem da descrição «instalação comercial que corre sozinha», que não
está confirmada como decisão de produto. Hoje **não existe** código de agendamento (E7 —
verificado: só há `--loop` no player).

**Fora dos critérios, de propósito:** paridade completa de efeitos (E1) e drones (M10). São
frentes **abertas sem ponto de chegada**: podem continuar depois dos 100 % sem invalidar o
C1–C9.

## VI.2 — Legenda dos passos

| Marca | Tipo de gate | Significa |
|---|---|---|
| 🟢 | **landável aqui** | prova-se com `cargo test` / `clippy` / `tsc` / `audit_gate` neste ambiente |
| 🟠 | **precisa de autorização** | push, doc versionada, alteração de ADR/CI |
| 🔵 | **precisa do humano / hardware** | rig, GPU real, leitor de ecrã, traje, tempo de parede |
| 🟣 | **decisão / ADR** | escolha que não é código |

Regras que valem para todos os passos: **exit lido sem pipe (KB-013)**; verde da CI lido **no
log**, não no ✔️; `NaoMedido ≠ Pass`; nenhum tempo medido sob carga > 6 conta; um commit por
passo, com o seu «sim». **Nenhum marco antes do M6 se declara «pronto para show» (D-A).**

## VI.3 — Marcos

### M0 — Fechar pendentes *(curto; desbloqueia o resto)*

| # | Passo | Tipo | Critério de aceitação |
|---|---|---|---|
| 0.1 | ✅ **Publicar os 4 commits** (`ce959a7…320ff94`). **Feito** — `git ls-remote` == `320ff94` (2026-09-26). | 🟠 | `git ls-remote` == HEAD local |
| 0.2 | ✅ **Ler o run da CI no log** — run `36226221020` (PR #6): ubuntu **105 · 1131/0/9**, macOS **105 · 1135/0/9**, clippy **correu** nos dois, miri **7/0/0**. Windows ❌ esperado (TD-021). | 🟢 | log lido job a job |
| 0.3 | 🟡 **TD-022 — medido, fecho por mergear.** Sonda determinística em Linux (branch descartável `probe/td-022-linux`, run `36245228354`, lido no log): `kind=BrokenPipe raw_os_error=Some(32)`, **3/3**, recusa no buffer. Falsificações F1–F3 re-executadas (2026-09-26). Conjunto aceite **não** alargado. O ledger `closed` vive no **PR #7** (draft, por mergear). | 🟢 sonda · 🟠 ledger | PR #7 mergeado com a CI lida no log |
| 0.3b | ✅ **Merge do PR #6** — feito (2026-09-26, `b86464b`, merge commit: hashes C1–C4 preservados). **Falta** ler a CI do merge na `main` no log. | 🟠 | CI do merge lida no log |
| 0.4 | **TD candidato**: o hook de pre-commit valida o **worktree**, não o índice (deu «20 OK» com índice de 19 TD). Registar como TD e corrigir. | 🟢 | teste que falha com índice ≠ worktree |
| 0.5 | **`show.gif`**: está trackeado **e** em `.gitignore` (`*.gif`) e é regenerado por `~/lumyx-e2e.sh`, logo aparece sempre como M. Decidir: `git rm --cached` ou fixar. | 🟣🟠 | worktree limpo após um e2e |
| 0.6 | **TD-018** (sintaxe do universo no `--help` do daemon). *(O TD-017 passou para o M2.)* | 🟢 | `--help` mostra a sintaxe obrigatória |
| 0.7 | **Deriva de doc** que restar em `CLAUDE.md`. **Não** reescrever o changelog histórico. | 🟠 | grep dos números contra medição |
| 0.8 | **TD candidato**: `probe_linux()` (`led-hal/src/network_guard.rs:228`) — o bloqueio de WiFi (ADR-0005) no SO do show ao vivo, usado no pré-voo do daemon — tem **zero testes**; os 4 testes que só correm em macOS (1135 vs 1131) são do parser macOS. Registar como TD. | 🟢 | teste do `probe_linux` a correr no job ubuntu |

### M1 — PROTOCOL_V = 2 *(pré-requisito do D6)*

Hoje `PROTOCOL_V = 1` e `SUPORTADAS` tem **um** elemento. O ADR-0031 está **aceite**; a metade
**emissora** da decisão 2 está feita.

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 1.1 | Estado de versão **por ligação** no daemon | 🟢 | teste: duas ligações, versões distintas |
| 1.2 | Segunda versão em `SUPORTADAS`; o daemon **lê** o `accepts` do pedido | 🟢 | teste de negociação + recusa por versão |
| 1.3 | Decisões 3–7 do ADR-0031, **uma por commit** | 🟢 | cada uma com o seu teste |
| 1.4 | Regenerar o contrato TS; gate `tsc --noEmit` | 🟢 | job `contrato TS` verde |
| 1.5 | **Falsificação:** mutar o `accepts` e ver o teste falhar | 🟢 | mutação apanhada |

### M2 — Segurança *(D6 + TD-013 / TD-014 / TD-017)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 2.1 | **D6** — botão + **confirmação em duas fases** (decisão 6 do ADR-0017; exige M1) | 🟢 | teste do fluxo de duas fases |
| 2.2 | **D6** — **log auditável** de cada blackout (quem/quando/estado) | 🟢 | entrada no log por comando |
| 2.3 | **D6** — **sem atalho de teclado** nesta fatia (decisão 10) | 🟢 | teste que prova a ausência |
| 2.4 | **TD-013**: autenticar o artefacto recortado **antes** do playback | 🟢 | teste com artefacto adulterado é recusado |
| 2.5 | **TD-014**: rota da perda (`console.dropped`) até ao operador (ADR-0026 §13 exige que seja **reportada**) | 🟢 | teste: perda → aviso visível |
| 2.6 | **TD-017**: 🟣 decidir a política do universo fora dos 15 bits e alinhar daemon e player | 🟣🟢 | os dois binários recusam/aceitam o mesmo |

*O requisito **9.C** do ADR-0017 (firmware em blackout ao perder o link) **não** está aqui: é
medição física e vive no **M6** (6.6).*

### M3 — D4: preview WebGPU *(fecha o elo vazio do Golden Slice)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 3.1 | **Tap de preview no daemon**, fora do hot-path: cópia **downsampled**, rate-limited, **lossy** (ADR-0015). Evento novo no canal `subscribe` — aditivo, **sem** PROTOCOL_V=2. | 🟢 | teste: o tap não altera o hash do frame enviado |
| 3.2 | **Gate de alocação**: o hot-path continua com **0** alocações com o tap ativo | 🟢 | `no_alloc` verde com o tap ligado |
| 3.3 | **Política de perda**: consumidor lento perde frames de preview, **nunca** atrasa o envio | 🟢 | teste com consumidor bloqueado |
| 3.4 | Tipo do evento **gerado** para TS (ADR-0027) | 🟢 | `tsc` verde |
| 3.5 | Renderer WebGPU no `console-web`: pontos instanciados, sem `fillRect` | 🟢 build · 🔵 GPU | compila; **fps medido em GPU real** |
| 3.6 | 🟣 **Comportamento sem WebGPU** (browser sem suporte): degradar, avisar ou bloquear | 🟣 | decisão registada em ADR |
| 3.7 | Medir **6.200 px** e **≥ 10k px** em GPU real | 🔵 | fps citado com a **condição** |

*O achado que fundamenta o D4: Canvas2D dá **3 fps a 10k pontos** — o preview tem de ser WebGPU.*

### M4 — D5 / D7 / profiles / a11y *(superfície sobre domínio que já existe)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 4.1 | **D5** — read-model da timeline exposto no contrato (o `led-sequencer` já tem o modelo) | 🟢 | tipos TS gerados |
| 4.2 | D5 — waveform de áudio, clips, keyframes, blend, marcadores de batida | 🟢 build · 🔵 a11y | edição não-destrutiva provada por replay |
| 4.3 | D5 — undo/redo | 🟢 | teste de ida-e-volta |
| 4.4 | **D7** — editor de layout: modelos, posicionamento (fecha o **E3**) | 🟢 · 🔵 | layout desenhado == layout compilado (`RigBuilder`, sem conflitos por construção) |
| 4.5 | `/api/profiles` devolve 501 — 🟣 escolher: IPC v2 (M1) **ou** catálogo no console | 🟣🟢 | rota deixa de devolver 501 |
| 4.6 | `path` sem histórico/completação no `load` | 🟢 | teste de UI |
| 4.7 | Confirmação de `load` **medida com leitor de ecrã** | 🔵 | relatório de a11y |
| 4.8 | Uma fatia por PR, cada uma com o gate do ADR-0028 (o único `fetch` continua em `transport/api.ts`) | 🟢 | gate de fronteira verde |

### M5 — ADR do F3 *(decisão; dentro do 100 % por D-B)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 5.1 | 🟣 **F3**: firmware próprio × preset WLED — **ADR próprio antes de qualquer código** (o ADR-0022 deixa-o explicitamente fora do seu escopo) | 🟣 | ADR aceite |
| 5.2 | 🟣 **Requisito explícito do ADR do F3:** **resolver o conflito com a invariante «sem WiFi em show ao vivo»** (ADR-0005) — os trajes **não podem ter cabo**. O ADR **tem de decidir** isto; este plano **não propõe** a solução | 🟣 | a decisão consta do ADR, com a sua justificação |

*Enquadramento que o ADR do F3 terá de respeitar ou emendar explicitamente, **citado e não
interpretado**: o ADR-0022 já fixa o sincronismo **antes** do número (D4), **nenhum rádio no
caminho crítico** (Q2), e o ADR-0005 **intacto** (critério V7, gate G7).*

### M6 — Rig *(FASE G — trilha humana paralela **desde já**; é o marco «pronto para show»)*

Nada aqui é código; tudo tem comando pronto. **Corre em paralelo de M1–M5 e não os bloqueia
(D-A)**, mas **nenhum marco é «pronto para show» antes de o M6 fechar**. Ordem interna
**obrigatória**: cada passo só vale se o anterior passou.

| # | Passo | Tipo | Comando / aceitação |
|---|---|---|---|
| 6.0 | **WLAN → cabo** nos nós 2–5 (o G1 migrou **1 nó** a 2026-08-28; o rig de 5 robôs era Art-Net por WiFi) | 🔵 | cada nó responde por Ethernet |
| 6.1 | **G2**: energizar os nós 2–5 (6.200 px) | 🔵 | `led-player robot_sequence.lumyx --ddp <ip>` por nó |
| 6.2 | Medir **portas físicas** (ADR-0030): hoje **NÃO MEDIDO** | 🔵 | controlador multi-porta observado |
| 6.3 | **G7**: RGBW `dtype 0x33` sobre DDP | 🔵 | o validador deixa de avisar |
| 6.4 | **C2**: show de **3 min**, 6.200 px, cabo, **p99 medido** | 🔵 | scrape + hash estável |
| 6.5 | **G6**: chaos físico — puxar o cabo com o show a correr | 🔵 | degradação por nó, sem derrubar o resto |
| 6.6 | Requisito **9.C** do ADR-0017 — o firmware entra em blackout ao perder o link | 🔵 | medido; **não medido hoje** |
| 6.7 | **G4**: Falcon / FPP | 🔵 | mesmo player, `--artnet`/`--ddp` |
| 6.8 | **G5**: determinismo Linux/Windows | 🟢 CI · 🔵 | `./scripts/determinism_probe.sh` |
| 6.9 | **TD-019**: o critério D custa **três** sítios, incl. o harness do burn-in de certificação | 🔵 | fecha com o burn-in |
| 6.10 | **G3**: burn-in **72 h → 168 h**, lançado **fora da sessão** | 🔵 | `launchctl load ~/Library/LaunchAgents/com.lumyx.burnin.plist` |
| 6.11 | **C1**: o operador percorre o Golden Slice inteiro | 🔵 | run registado |

### M7 — Instalação permanente e paridade *(C8 continua PROPOSTA; E1 é contínuo)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 7.1 | 🟣 Confirmar que **instalação permanente** é decisão de produto (é o que justifica o C8) | 🟣 | ADR ou nota no roadmap |
| 7.2 | **E7**: scheduler/playlist por horário e dia | 🟢 | teste com relógio injetado |
| 7.3 | **E1**: ~25 efeitos em tranches de ~5, no molde `ComputeKernel` + ADR-0021, **cada um** com o gate de alocação e de pureza | 🟢 | contagem sobe, 0 alocações |
| 7.4 | **E5**: export `.fseq` (interop FPP) — hoje **zero** ocorrências | 🟢 | round-trip com um leitor de referência |
| 7.5 | **E6**: upload de configuração para o controlador (hoje à mão) | 🟢 · 🔵 | config aplicada e lida de volta |
| 7.6 | **E2**: preview 3D (depende do M3) | 🟢 · 🔵 | Z deixa de ser ignorado |
| 7.7 | E4 (vídeo→pixel) e E8 (faces/canto) | 🟢 | sem gatilho — baixa prioridade |

### M8 — Trajes F3–F6 *(FASE F; dentro do 100 % por D-B)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 8.1 | Pré-condições: **ADR do F3 aceite** (M5) e **TD-013 fechado** (M2.4) — sem isto, nada sai da bancada | 🟣🟢 | ver 5.1–5.2 e 2.4 |
| 8.2 | **F3**: player embarcado, conforme o ADR do F3 | 🔵 | traje toca sozinho |
| 8.3 | **F4**: sync multi-traje — start comum + medição de **drift** (`SharedClock`/`net_time` existem) | 🟢 · 🔵 | drift medido durante o número |
| 8.4 | **F5**: orçamento de bateria, corrente, peso, calor | 🔵 | ver ADR-0022 |
| 8.5 | **F6**: degradação segura — um traje que falha não derruba o número | 🟢 · 🔵 | teste de falha por traje |

### M9 — Distribuição *(FASE H; fecha C7)*

| # | Passo | Tipo | Aceitação |
|---|---|---|---|
| 9.1 | **D8**: empacotamento desktop com webview do SO (depende de D3, D4) | 🟢 · 🔵 | app arranca sem dev server |
| 9.2 | Binários assinados por plataforma (cosign já corre) | 🟠 | assinatura verificável |
| 9.3 | 🟣 Licença e modelo de distribuição | 🟣 | decisão registada |
| 9.4 | Documentação de **operador** (hoje a doc é de arquiteto) | 🟠 | um operador conclui o C1 só com a doc |
| 9.5 | Guia de migração xLights → LUMYX (o código já faz; falta o texto) | 🟠 | guia testado com o projeto real |
| 9.6 | H5: catálogo de presets da comunidade (cada placa é **uma linha**) | 🟢 | preset novo sem código |

### M10 — Drones *(decisão pendente; fora do 100 %)*

Existe um repositório **separado**, `~/drone-platform` (núcleo de segurança G2: validação
contínua, atribuição húngara em `drone-formations`, `ShowIntent` determinístico, export `.skyc`
bloqueado por validação). **Não está neste repositório** e o lugar dele no roadmap do LUMYX
**não foi decidido**. G2 não pode ser afirmado «validado» (ver memória do projeto).

| # | Passo | Tipo |
|---|---|---|
| 10.1 | 🟣 Decidir: integrar na timeline comum, ou manter separado com contrato partilhado | 🟣 |
| 10.2 | Se integrar: ADR de fronteira (o `ShowIntent` é o contrato natural) | 🟣 |

## VI.4 — Dependências entre marcos

Duas setas diferentes: **══►** é dependência **técnica** (o passo não compila/não prova sem o
anterior); **──►** é **ordem ratificada** (prioridade do dono, sem dependência técnica).

```
TRILHA DE SOFTWARE (landável aqui, pela ordem ratificada)

M0 ──► M1 ══► M2 ──► M3 ──► M4 ──► M5 ──► M7 ──► M8 ──► M9
        │      │ (D6 exige PROTOCOL_V=2)   │             ▲
        │      │                           │             │
        │      └══ 2.4 TD-013 ═════════════╪═════════════╣ (8.1)
        │                                  └══ M5 ═══════╝ (ADR do F3 antes de código F3)
        └══ 4.5 /api/profiles (se a escolha for IPC v2)
M3 (D4) ══► 7.6 E2 (3D) · M3 + M4 ══► 9.1 D8

TRILHA HUMANA (paralela desde já — D-A)

M6 rig: 6.0 cabo ─ 6.1 G2 ─ 6.2 ─ 6.3 ─ 6.4 3 min p99 ─ 6.5 ─ 6.6 9.C ─ … ─ 6.10 burn-in ─ 6.11 C1

PORTÕES

«pronto para show»  ⇐  M6 fechado            (nenhum marco antes disso — D-A)
C1 (Golden Slice)   ⇐  M2 + M3 + M4 + M6
C9 (trajes)         ⇐  M5 + M8 (+ TD-013 de M2)
100 %               ⇐  C1–C9 com evidência (C8 enquanto PROPOSTA)

M10 fora do 100 %; E1 (7.3) é contínuo e não bloqueia nada.
```

**Caminho crítico até C1:** `M0 → M1 → M2 → M3 → M4` na trilha de software, e o **M6** a
fechar por cima, na trilha humana. **Caminho crítico até 100 %:** o anterior **+** `M5 → M8`
(trajes) **+** `M9` (distribuição).

**O que isto não diz:** M3 **não** depende tecnicamente de M1/M2 (o evento de preview é
aditivo); vem depois por **ordem ratificada** (segurança antes de preview), não por
compilação. Se for preciso paralelizar, é o primeiro candidato.

**O passo que mais tempo de parede custa** é o **6.10** (burn-in 168 h): convém lançá-lo
**assim que o 6.1–6.4 passem**, e continuar M1–M5 enquanto ele corre.

## VI.5 — O que este plano NÃO promete

- **Que o rig completo funcione.** Hoje há **1 nó de 5** provado (720 px de 6.200). É o M6.
- **Que algo seja «pronto para show» antes do M6** (D-A).
- **Que o preview atinja um fps.** Só o 3.7, em GPU real, o diz.
- **Que o `9.C` valha.** O firmware entrar em blackout ao perder o link é um **requisito**,
  não uma garantia (ADR-0017).
- **Como os trajes funcionam sem cabo e sem WiFi ao vivo.** É o que o ADR do F3 (5.2) tem de
  decidir; o plano não o antecipa.
- **Nenhum número de mercado.** Metas comerciais (receita, nº de instalações, prazo de
  desistência) **não constam de nenhum documento do repositório** e não entram neste plano.
- **Que E1/M10 tenham fim.** São frentes abertas.

## VI.6 — Próxima ação

*(Atualizado 2026-09-29.)* **M0.1, M0.2 e M0.3b estão feitos**; a sonda do TD-022 em Linux
**correu** (0.3). Próximos, por ordem:

1. ✅ **PR #8 mergeado** (fix do TD-020 stale; `a922e14`, 2026-09-28) e ✅ **PR #9 mergeado**
   (debt gate na CI; `82e5ba1`, 2026-09-28).
2. ✅ Mergeados a 2026-09-29: **#11** (o hook julga o índice; `0f7857e`), **#13** (TD-014 →
   `GET /api/dropped`, ADR-0026 §13-bis; `445d296`), **#12** (audit_gate mostra open/wontfix;
   `207c87f`) e **#7** (TD-022 closed; `6ac22ff`). **#10** fechado sem merge (controlo negativo).
   CI da `main` em `6ac22ff`: debt gate `13 OK · 5 open · 2 wontfix`, exit 0.
3. Registar **TD-023..026** no ledger a partir da `main` atualizada (rascunho já decidido:
   no_alloc do `led-hal` com contador global; `speed_factor` com relógio de parede; hook sobre
   o worktree; `audit_gate` ignora o returncode do `git log`). O TD-027 (Miri do `audio-core`
   no e2e) continua sem evidência.
4. **M1** (PROTOCOL_V=2) — muda o protocolo IPC: exige autorização antes de começar.

Em paralelo e desde já, na trilha humana: **6.0** (WLAN → cabo nos nós 2–5).
