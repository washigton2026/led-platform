# LUMYX — Roadmap Mestre

> **Objetivo final.** Uma plataforma de iluminação por pixels que (a) substitua
> xLights/Vixen para shows de larga escala com garantias que eles não dão — determinismo,
> replay assinado, observabilidade, failover — e (b) viabilize **trajes de LED para
> performance de dança** com qualidade de palco.
>
> Este documento é o mapa completo: **o que já existe com evidência**, **o que falta**, e
> **em que ordem**, com o que bloqueia o quê.
>
> Data desta revisão: **2026-08-05** · HEAD `5416241` · `led-core` **1.4.0** · **791 testes**
>
> *Revisão anterior (2026-08-03, HEAD `80c2a6c`) ficou 3 commits atrás e listava como
> disponíveis frentes que já tinham sido entregues. Ver PARTE III.*

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
- **31 ADRs** · **17 entradas de TD**: **10 fechadas** com evidência auditável, 2 `wontfix` com
  gatilho de revisita, **5 abertas** (TD-013, TD-014, TD-017, TD-018, TD-019)
- CI: **Linux + macOS bloqueantes, verdes** em `80c2a6c`; Windows não-bloqueante

**Gates executados nesta revisão (2026-08-03, não citados de memória):**

```
cargo test --workspace --locked --no-fail-fast
  → 64 suítes · 771 passed · 0 failed · 8 ignored · exit 0
cargo clippy --workspace --all-targets --locked -- -D warnings
  → exit 0
```

*(746 → 771 nesta sessão: +25 testes da 1ª fatia do E1, incl. o gate de alocação do render.)*

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
| 0031 | negociação de versão no handshake (o `hello` viaja em `v:1`) | ✅ aceito (`docs/adr/0031-negociacao-de-versao-no-handshake.md:3`) · ⬜ **não implementado** — `PROTOCOL_V` continua 1 (`crates/led-daemon-bin/src/proto.rs:17`) |

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
| D4 | **Preview WebGPU**: cópia downsampled, rate-limited, **lossy por contrato** (ADR-0015) | ⬜ depende de D3 |
| D5 | **Timeline visual**: waveform de áudio, clips, keyframes — o `led-sequencer` já tem o modelo | ⬜ depende de D3 |
| D6 | **Blackout**: botão + confirmação em duas fases + log auditável. **Sem atalho de teclado nesta fatia** (ADR-0017, decisão 10) | ⬜ **desbloqueado, não landado** — o B1 fechou a 2026-09-01. A pré-condição **escape por device** deixou de faltar: aterrou com a máscara em `1030a7e` (2026-09-04). O bloqueio actual é outro — a decisão 6 (duas fases) exige `PROTOCOL_V = 2` e `PROTOCOL_V` é 1; ver ADR-0031, aceite e não implementado |
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
| 3 | **F7.2 Ubuntu não fecha** — falta a próxima falha para o instrumento produzir o `N` de alocações; o log da CI devolve HTTP 403 sem autenticação | changelog 2026-08-13d |
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
