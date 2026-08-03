# LUMYX — Roadmap Mestre

> **Objetivo final.** Uma plataforma de iluminação por pixels que (a) substitua
> xLights/Vixen para shows de larga escala com garantias que eles não dão — determinismo,
> replay assinado, observabilidade, failover — e (b) viabilize **trajes de LED para
> performance de dança** com qualidade de palco.
>
> Este documento é o mapa completo: **o que já existe com evidência**, **o que falta**, e
> **em que ordem**, com o que bloqueia o quê.
>
> Data desta revisão: **2026-08-03** · HEAD `80c2a6c` · `led-core` **1.4.0**

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

# PARTE I — Onde estamos

## I.1 — Maturidade por camada

| Camada | Estado | O que está provado | Lacuna nomeada |
|---|---|---|---|
| **Contratos / seams** (`led-core`) | ✅ | 9 contratos certificados, 5 **Frozen**, SemVer com guardião mecânico e negative control | — |
| **HAL** (`led-hal`) | ✅ | mapeamento aplicado **uma vez**, zero alocação no hot-path (contador real), heartbeat, NetworkGuard, calibração por-output | fan-out sequencial (ADR-0012, adiado até 2º nó) |
| **Layout** (`led-layout`) | ✅ | MegaTree, matriz serpentina, `RigBuilder` livre de conflito por construção | editor visual não existe |
| **Protocolos** (`led-protocols`) | ✅ | sACN unicast+multicast, Art-Net ArtDmx+ArtPoll, DDP, RouterDevice | RGBW-sobre-DDP `dtype 0x33` **não validado em hardware** |
| **Engine de render** (`led-pixel-engine`) | 🟡 | triple buffer Miri-limpo, pipeline, GPU compute (wgpu) | **5 efeitos visuais** contra ~40 do xLights |
| **Sequencer** (`led-sequencer`) | ✅ | Timeline não-destrutiva, clips, keyframes, blend, TempoMap, beat-sync, `ShowIntent` | sem UI |
| **Áudio** (`led-audio`, `audio-core`) | ✅ | Hann→FFT→bandas→flux-beat→BPM→seções musicais, zero-alloc, ring SPSC Miri-limpo | — |
| **Gravação / replay** (`led-show-recorder`) | ✅ | formato `.lumyx`, manifest, hash FNV-1a, Ed25519 **com chave fixada** | sem playback embarcado (ver FASE F) |
| **Migração xLights** (`led-xlights`) | ✅ | import + gate de conflito + auto-fix + **export bidirecional** | `.fseq` **não existe** (interop FPP) |
| **Perfil de hardware** (`led-hardware-profile`) | ✅ | descritor de capacidades, validador, presets como **dado**, compilação | **porta física única** por profile (FASE C) |
| **Read-model** (`led-readmodel`) | ✅ | snapshot read-only, JSON à mão, bind loopback-only | nenhuma UI consome |
| **Console do operador** | ⬜ | — | **a maior peça faltante** (FASE D) |
| **Observabilidade** | ✅ | Prometheus + Grafana + 5 alertas + 4 SLOs, scrape ao vivo em show real | — |
| **Segurança** | ✅ | cosign, SBOM, attestation, Ed25519 pinado, red-team com achado CRITICAL fechado | — |
| **Governança** | ✅ | 20 ADRs, ledger de TD com gate executável, 27 agentes, guardiões mecânicos, CI verde | — |
| **Hardware real** | 🟡 | **1 nó de 5** validado ponta-a-ponta (720 px de 6.200) | Ethernet, Falcon, FPP, 72h (FASE G) |
| **Trajes de dança** | ⬜ | — | **bifurcação arquitetural não decidida** (FASE F) |

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
| 0013 | engine em daemon separado | ✅ aceito, ⬜ **não implementado** |
| 0014 | IPC e segurança UI↔engine | ✅ aceito, ⬜ **não implementado** |
| 0015 | preview lossy fora do hot-path | ✅ aceito, ⬜ **não implementado** |
| 0016 | stack do console | 🔴 **provisório** — depende de medição humana |
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

### B2 — ADR-0016: fechar a stack do console 🔴 *medição humana*

O agente já mediu tudo que era mensurável sem humano. Faltam **3 eixos** que exigem sua máquina:

1. **Leitor de tela real** — VoiceOver/NVDA: a live-region anuncia a mudança de status?
2. **Teclado interativo** — navegar a tela inteira, foco visível, sem trap?
3. **DX subjetiva** — qual ecossistema você quer manter pelos próximos anos?

**O que a medição já decidiu por si:** o preview **será WebGPU** (3 fps em Canvas2D).
**O que falta decidir:** Leptos (Rust puro) × React/TS (a11y provada, 27× mais rápido de buildar).

**Desbloqueia:** toda a FASE D.

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

## FASE D — Console do operador ⬜ *bloqueado por B2*

A maior peça faltante. Hoje o LUMYX é **CLI + biblioteca**; xLights e Vixen são
**aplicativos**. Sem console, a plataforma não é usável por quem não escreve Rust.

| PR | Conteúdo | Depende de |
|---|---|---|
| D1 | **Daemon**: engine headless, processo separado (ADR-0013) | — |
| D2 | **IPC**: UDS owner-only, comandos tipados e versionados, nunca `0.0.0.0` (ADR-0014) | D1 |
| D3 | **Shell do console**: polling do `led-readmodel`, saúde por controlador | B2, D2 |
| D4 | **Preview WebGPU**: cópia downsampled, rate-limited, **lossy por contrato** (ADR-0015) | B2 |
| D5 | **Timeline visual**: waveform de áudio, clips, keyframes — o `led-sequencer` já tem o modelo | D3 |
| D6 | **Blackout**: botão + atalho + confirmação + log auditável | **B1** |
| D7 | **Editor de layout**: desenhar modelos, posicionar no palco | D3 |
| D8 | **Empacotamento**: app desktop com webview do SO | D3, D4 |

> **Onde o control-plane ainda está vazio.** A especificação existe
> (`docs/architecture/control-protocol.md`) e expôs a lacuna honestamente: hoje **não há o
> que comandar** — o engine não tem um estado de show controlável em runtime. D1 é onde
> isso nasce.

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

## FASE F — Trajes de LED para dança ⬜ 🔴 *bifurcação não decidida*

**Este é o item que mais precisa de uma decisão arquitetural, e ainda não tem ADR.**

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
| F1 | **ADR: wearable autônomo × streaming** | a decisão que abre a fase |
| F2 | **`bake`**: show → artefato que roda no controlador | reusa `.lumyx` + manifest |
| F3 | **Player embarcado** | firmware ou WLED preset — decisão de plataforma |
| F4 | **Sync multi-traje**: start comum + medição de **drift** ao longo do número | `net_time` já resolve o análogo cabeado (±10 ms medido) |
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
        ├── E1..E8 paridade xLights ──► (E1 efeitos não depende de nada)
        │
        ├── F1 ADR wearable ─ F2 bake ─ F3 player ─ F4 sync ─ F5 orçamento ─ F6 degradação
        │
        └── G1..G8 certificação ──────► (bloqueada por hardware/tempo, não por código)
```

**Três frentes podem correr em paralelo hoje mesmo:**

1. **C** — portas múltiplas (aditivo, sem decisão pendente)
2. **E1** — biblioteca de efeitos (a maior lacuna de paridade, zero dependência)
3. **F1** — o ADR do wearable (é escrita e decisão, não implementação)

**Duas frentes estão paradas esperando você:** B1 (uma resposta) e B2 (três medições).

---

# PARTE IV — Riscos

| Risco | Probabilidade | Impacto | Mitigação |
|---|---|---|---|
| **Escopo do console** — D é maior que tudo que foi feito até agora | alta | alto | fatiar por PR com gate; D1–D3 entregam valor antes do resto |
| **Trajes exigem firmware embarcado** — competência diferente da do repo | alta | alto | F1 decidir **antes** de codar; considerar WLED preset em vez de firmware próprio |
| **Paridade de efeitos é trabalho longo e repetitivo** | alta | médio | o `ComputeKernel` já é o molde certo; cada efeito é aditivo e testável |
| **Rig continua offline** | média | alto | tudo que era gateável sem hardware **já foi feito** — a fila G está pronta, só falta energia |
| **O `compile` O(n²)** morde acima de 50k px | baixa | médio | TD-012 com gatilho e guarda falsificável que roda sempre |
| **Windows nunca fica verde** | média | baixo | não-bloqueante por ADR-0013; não orienta o design |

---

# PARTE V — Onde estamos, em uma frase

**O motor está pronto e provado; o produto ainda não tem rosto.**

O núcleo — determinismo, contratos, protocolos, áudio, replay, segurança, observabilidade —
está em estado que xLights e Vixen não alcançam. O que falta é quase tudo **acima** do
motor: o console que torna isso operável, os efeitos que tornam isso expressivo, e a
decisão de wearable que torna isso vestível.
