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
| Configurar controladores | 🟡 | `led-hardware-profile` compila o layout, mas o WLED é configurado **à mão** (E6) e o profile tem **porta física única** (FASE C) |
| Enviar via **Ethernet** | 🔴 | os protocolos estão prontos e validados; o **meio** não — toda validação de hardware foi sobre **WiFi**, que o ADR-0005 proíbe ao vivo |
| Executar em hardware real | 🟡 | **1 nó de 5** (720 px de 6.200), e sobre WiFi |
| Validar o resultado | ✅ | replay por hash, Ed25519 com chave fixada, métricas ao vivo durante show real |

**Os dois elos ausentes são exatamente os dois gargalos do produto:** a **interface**
(bloqueada pela decisão **B2**, que é sua) e o **Ethernet** (bloqueado por recurso físico,
não por código). Enquanto qualquer um dos dois estiver aberto, **não há Golden Slice** —
há um motor forte com dois vãos no caminho do operador.

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
| **Perfil de hardware** (`led-hardware-profile`) | ✅ | descritor de capacidades, validador, presets como **dado**, compilação | **porta física única** por profile (FASE C) |
| **Read-model** (`led-readmodel`) | ✅ | snapshot read-only, JSON à mão, bind loopback-only | nenhuma UI consome |
| **Console do operador** | ⬜ | — | **a maior peça faltante** (FASE D) |
| **Observabilidade** | ✅ | Prometheus + Grafana + 5 alertas + 4 SLOs, scrape ao vivo em show real | — |
| **Segurança** | ✅ | cosign, SBOM, attestation, Ed25519 pinado, red-team com achado CRITICAL fechado | — |
| **Governança** | ✅ | **22 ADRs**, ledger de TD com gate executável (hook de pre-commit), 27 agentes, guardiões mecânicos, CI verde | 2 ADRs por decidir: **B1** (0017) e **B2** (0016) |
| **Hardware real** | 🟡 | **1 nó de 5** validado ponta-a-ponta (720 px de 6.200) | Ethernet, Falcon, FPP, 72h (FASE G) |
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
- **20 ADRs** · **12 TDs fechados com evidência auditável** · 2 `wontfix` com gatilho de revisita
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
| 0017 | blackout × heartbeat | 🔴 **adiado — decisão pendente** |
| 0018 | HardwareProfile | ✅ implementado (5 slices) |
| 0019 | calibração por-output no HAL | ✅ |
| 0020 | `WhiteMode::MinSubtract` | ✅ |
| 0021 | efeito é **função pura**, estado derivado nunca armazenado | ✅ implementado (E1, 1ª fatia) |

---

# PARTE II — O que falta

## FASE B — Decisões que estão bloqueando código 🔴

**Nada aqui é trabalho de programação. São duas escolhas.** Enquanto não forem feitas,
duas fases inteiras ficam paradas.

### B1 — ADR-0017: semântica do blackout 🔴 *decisão do usuário*

**A pergunta:** o operador aciona blackout. O heartbeat dispara em seguida. O que vai no fio?

| Opção | Consequência |
|---|---|
| **(a) O preto persiste** ← *recomendação* | O palco fica apagado até comando explícito. O invariante "nunca envia frame zerado" continua valendo para o **silêncio acidental**; o preto vira estado **comandado**. |
| (b) Último frame pré-blackout | O rig **reacende sozinho** no próximo heartbeat. Inaceitável em palco. |

**Implementação já desenhada** (não escrita): máscara no HAL reusando o ponto de
interceptação do ADR-0019, **memset** no scratch do device — não multiplicação por-byte —
e o heartbeat **empurra** (`send_frame`), não puxa.

**Desbloqueia:** botão/atalho de blackout no console (D6) — hoje **proibido** por ADR.

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

## FASE C — HardwareProfile: múltiplas portas físicas ⬜ *não bloqueado*

**Achado sustentado** da revisão externa, com o diagnóstico refinado: `PixelPhysical.format`
**já é por-pixel**, então o `CompiledLayout` já consegue expressar portas com formatos
diferentes. Quem achata é apenas o **descritor de design-time**.

| Slice | Conteúdo | Risco |
|---|---|---|
| C1 | `Port { index, pixel_count, color, calibration }` no profile — **aditivo** | baixo |
| C2 | `compile_layout` distribui por porta preservando o `pixels_per_universe` declarado | médio (é o coração do mapeamento) |
| C3 | Presets ganham portas — Falcon F16V3 tem **16** portas; hoje declara 1 | nenhum (é dado) |
| C4 | Guardião: 9º check — porta não pode vazar para o runtime | baixo |

**Não toca nenhum seam Frozen.** É a próxima peça de código que pode começar **hoje**.

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
| D6 | **Blackout**: botão + atalho + confirmação + log auditável | ⛔ bloqueado por **B1** |
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
| G1 | **Migração WiFi → Ethernet** | cabo/switch no rig | preset `esp32-poe-wled-ddp` já existe |
| G2 | **Nós 2–5** (6.200 px completos) | energizar o rig | `led-player robot_sequence.lumyx --ddp <ip>` |
| G3 | **Burn-in 72 h** → 168 h | lançar **fora da sessão** | `launchctl load ~/Library/LaunchAgents/com.lumyx.burnin.plist` |
| G4 | **Falcon / FPP** | ter o controlador | mesmo player, `--artnet`/`--ddp` |
| G5 | **Determinismo Linux/Windows** | máquina ou CI | `./scripts/determinism_probe.sh` |
| G6 | **Chaos físico** | rig + puxar o cabo | burn-in rodando + desconectar ETH |
| G7 | **RGBW `dtype 0x33` no DDP** | fita RGBW no rig | o validador já **avisa** que não foi validado |
| G8 | **sACN em hardware** | reflash do WLED | bloqueio é de **firmware**, não do LUMYX (provado) |

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
        ┌── B1 blackout ──────────────┐            (decisão sua — 1 resposta)
        │                             ▼
HOJE ───┤                          D6 blackout no console
        │
        ├── B2 stack ── D1 daemon ─ D2 IPC ─ D3 shell ─┬─ D4 preview ─ D8 app
        │  (3 medições)                                 ├─ D5 timeline
        │                                               └─ D7 layout
        │
        ├── C portas múltiplas ──────► (não bloqueado — pode começar agora)
        │
        ├── E2..E8 paridade xLights ─► (E1 efeitos: 1ª tranche FEITA — 13 de ~40)
        │
        ├── F1 ✅ ─ F2 🟡 ─ F3 player ─ F4 sync ─ F5 orçamento ─ F6 degradação
        │
        └── G1..G8 certificação ──────► (bloqueada por hardware/tempo, não por código)
```

> **Correção de 2026-08-05.** A versão anterior deste diagrama listava **E1** e **F1** como
> frentes disponíveis. **As duas já tinham sido entregues** — E1 na 1ª tranche de efeitos
> (13 de ~40, ADR-0021) e F1 no ADR-0022. Priorizar sobre o mapa antigo mandaria refazer
> trabalho concluído.

**O que pode correr hoje, sem esperar por decisão:**

1. **C** — portas físicas múltiplas (aditivo, sem decisão pendente) — **a única frente
   inteiramente livre**
2. **E1 (continuação)** — ~25 efeitos para paridade; o molde (`ComputeKernel` + ADR-0021)
   já existe, cada efeito é aditivo e testável
3. **F3/F4** — player embarcado e sync multi-traje. **Atenção:** F3 é *decisão de
   plataforma* (firmware próprio × preset WLED), explicitamente fora do escopo do ADR-0022

**Uma frente continua parada esperando você:** B1 (uma resposta sobre o blackout, ADR-0017).
**B2 fechou em 2026-08-09** e a FASE D arrancou: o console tem processo, HTTP, SSE e uma
Application Shell que comanda o daemon a sério.

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

**O motor está pronto e provado; o produto ainda não tem rosto — e o Golden Slice tem dois
vãos.**

O núcleo — determinismo, contratos, protocolos, áudio, replay, segurança, observabilidade —
está em estado que xLights e Vixen não alcançam. O que falta é quase tudo **acima** do
motor: o console que torna isso operável e os efeitos que tornam isso expressivo. A decisão
de wearable, que era a terceira lacuna, **foi tomada** (ADR-0022) e está em execução.

Medido contra o Golden Slice, sobram **dois elos rompidos**: **editar/pré-visualizar** (a
interface, travada na decisão B2) e **enviar por Ethernet** (o meio, travado em recurso
físico). Nenhum dos dois se resolve escrevendo mais motor.
