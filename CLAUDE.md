# CLAUDE.md — LUMYX codebase guide

Reference implementation of the LUMYX LED pixel platform. Architecture is specified in the
`led-strip-platform` skill suite (`~/led-strip-platform-skill/`); this repo builds it
**slice by slice, foundation first**, each slice proving a contract in tested code.

## Definition of Done — read before closing any task

Work closes only when **all gates in [`LUMYX_GOSL.md`](./LUMYX_GOSL.md) pass** — compliance,
perf, protocol, seam — **and this `CLAUDE.md` is updated**. Updating `CLAUDE.md` to match
your change is part of finishing the task, not an afterthought.

LUMYX_GOSL also defines the **Hardware Rules** (WiFi forbidden live; heartbeat never zeros;
2.4 s max gap) and the standard commands **`/seam`** (contract audit), **`/security`**,
**`/phase-gate`**, **`/rollback`** (on an invariant violation, revert the whole file — never
patch inline — and report invariant/file/line), and **`/changelog`** (append a session
entry to the `## Session changelog` below at the end of every session).

## Build & test

```sh
cargo test --workspace                  # all suites (930 tests)
cargo build --workspace --all-targets   # must be warning-free
cargo +nightly miri test -p led-pixel-engine --lib   # lock-free unsafe under Miri
~/lumyx-e2e.sh                          # full cross-platform E2E validation
~/lumyx-e2e.sh --miri                   # + Miri on all unsafe crates
```

## Crate map (dependency DAG: everything depends on `led-core`, never the reverse)

| Crate | Owns | Sub-skill |
|---|---|---|
| `led-core` | seam types, `ProtocolOutput`/`DeviceDriver`/`IDevice`, `CompiledLayout` | master §3 |
| `led-hal` | `Hal` (sole `ProtocolOutput`), `SimulatorDevice`, `Heartbeat`, `Core` | led-hal |
| `led-layout` | `PixelLogical`/`Layout`, prop generators, `LayoutMapper` | led-layout |
| `led-protocols` | `SacnDevice` (E1.31, unicast + per-universe multicast) + ArtPoll source-conflict detection | led-protocols |
| `led-pixel-engine` | `Effect`s, HSV/gamma, lock-free triple buffer, render→send `Pipeline`, audio-reactive bridge, GPU-style compute kernels (`Plasma` + WGSL) | led-pixel-engine |
| `led-sequencer` | non-destructive `Timeline`/`Track`/`Clip`/keyframes + `TempoMap` beat-sync; **a `Timeline` is an `Effect`** | led-sequencer |
| `led-audio` | Hann-windowed FFT, band energy, spectral-flux beat detection → `led-core::AudioFeatures` (Phase-1 contract) | led-audio |
| `led-demo` *(bin)* | renders a show to `show.gif` (matrix + sequencer + Plasma + beat-sync); uses the `gif` crate | — |
| `audio-core` | **leaf, outside this DAG** — CPAL capture → Hann window → rustfft → its own `AudioFeatures` v1.0 (lumyx-system-architect §3/§11), published via `tokio::sync::watch` | lumyx-system-architect |
| `led-daemon-bin` | **NEW** — o **processo** (GS2): laço, pacer injetável, journal JSONL, loader `.lumyx`. Bin `led-daemon`. **Sem saída** — nenhum frame deixa o processo | — |
| `led-daemon` | **NEW** — a superfície de transporte do engine (ADR-0023): `State` (8 estados), `Command`, `Event`, `ShowRuntime`. **Zero dependências** — nem `led-core`; o pré-voo chega como dado | — |
| `led-bridge` | **integration seam** — the only crate that imports both `audio-core` (v1) and `led-pixel-engine` (v0). Owns: `adapt`/`adapt_into` (v1→v0 adapter), `BridgeHandle` (watch→AudioShare thread), `SimLoop` (hardware-free end-to-end live loop) | — |

Data flow: `led-layout` compiles the mapping → `led-sequencer` composes effects over time
(a `Timeline` *is* an `Effect`) → `led-pixel-engine` renders `LogicalFrame`s → (lock-free
triple buffer) → `Hal` applies the mapping **once** → `DeviceDriver` fan-out →
`SimulatorDevice` / `SacnDevice`. `Heartbeat` runs on its own thread.

Audio→light (Phase-1, wired): `led-audio` analyzes samples → `led-core::AudioFeatures` →
`led-pixel-engine`'s `AudioShare` (written by the audio thread) → reactive effects
(`BandPulse`, `BeatFlash`) read it on the render thread. `led-pixel-engine` consumes
`AudioFeatures` from `led-core`, so it does **not** depend on `led-audio` (only the
app/test wires them together).

Audio intelligence (`audio-core`, now wired via `led-bridge`): a separate, richer realtime
pipeline — own `AudioFeatures` (adds `peak`, `onset`, `bpm`, `spectral_centroid`,
`spectral_rolloff`, `spectral_flux`, `musical_section`; `spectrum` is `[f32; 512]` not
`Vec<f32>` for a `Copy`, alloc-free struct). **Contract divergence resolved at the
`led-bridge` boundary** (Cycle 3): `led_bridge::adapt_into` maps v1→v0 fields
(`bass_energy→bass`, `mid_energy→mid`, etc.), zero-alloc after warmup. `BridgeHandle`
spawns a thread that polls `watch::Receiver<V1>` and calls `AudioShare::publish` at the
analysis rate (~5ms/hop). `SimLoop` provides a hardware-free end-to-end test of the full
pipeline: SineGen → Analyzer → adapt → AudioShare → BandPulse/BeatFlash → pixels.

## Invariants that bite (enforced by tests — don't regress)

- **One mapping, applied once, at the HAL.** Nothing above the HAL names a universe/channel.
- **Core holds only `Arc<dyn ProtocolOutput>`** — never a device/socket.
- **Heartbeat resends the last valid frame, never zeros**; max gap to any device **2.4 s**
  (Warning 2.0 s, Critical 2.4 s). **WiFi is forbidden for live shows** (cabled only).
- **No allocation on the hot path** (`led-hal/tests/no_alloc.rs`, counting allocator).
- **Render and send never share a mutable buffer** — the `triple` buffer (Miri-clean,
  incl. many-seeds). The permutation invariant of its 3 slots is the whole safety argument.
- **Per-universe wrapping sequence** in sACN; one universe per datagram; per-universe
  multicast group (239.255.hi.lo) — and **one sender per universe** (ArtPoll detects a
  conflict and names the other IP before starting). Multicast needs IGMP on the path (`/security`).
- **Hann window before every FFT** (structural: `magnitude_spectrum` is the only path);
  **`sample_rate` explicit**, never hardcoded; spectral-flux beat with a slow-EMA threshold.
- `audio-core` (separate leaf, see crate map): same Hann/sample_rate/spectral-flux
  invariants, plus its own zero-alloc proof (`audio-core/tests/no_alloc.rs` — `Analyzer`
  hot path + `watch::send`, `AudioFeatures` is `Copy`) and an SPSC ring buffer Miri-clean
  across scheduler seeds (`audio-core/src/ring_buffer.rs`).

## Status (keep current)

```
cargo test --workspace                  # all suites (930 tests)
```

15 lib crates + `led-demo` binary + `led-bridge` integration crate + `led-show-recorder` · **930 tests green** · zero warnings.

Miri clean: `ring_buffer` (5, SPSC unsafe), `triple` buffer (24 seeds), `led-bridge/adapter` (6, 1M iter).
Governance: `scripts/audit_gate.py` (KB-012) — all 9 closed TDs pass evidence gate. `tests/test_audit_gate.py` 9/9. `lumyx-e2e.sh` Phase 5b + Phase 7 (Engineering Council gates C1–C11) run on every CI pass.

| Crate | Status |
|---|---|
| `led-core` | seam types (stable) |
| `led-hal` | HAL + mapping + heartbeat + NetworkGuard (integrated into `Hal::new`/`with_guard`) |
| `led-layout` | MegaTree + matrix-serpentine generators + LayoutMapper |
| `led-protocols` | sACN (unicast + multicast) + ArtPoll + DDP + RouterDevice (sACN/DDP fan-out by universe) |
| `led-pixel-engine` | effects (**13**: 5 base + biblioteca `Chase`/`Twinkle`/`Fire`/`ColorWash`/`Strobe`/`Meteor`/`Lightning`/`Ripple`, ADR-0021) + `noise` sem estado + triple buffer + pipeline + reactive bridge + GPU compute (wgpu 22.1.0, `gpu` feature) |
| `led-sequencer` | Timeline/Track/Clip/Keyframe + TempoMap + LiveTempoMap (real-time beat accumulator) |
| `led-audio` | Hann FFT + band energy + spectral-flux beat |
| `led-bridge` | adapt v1→v0 + BridgeHandle + SimLoop |
| `audio-core` | CPAL → SPSC ring → Hann FFT → bands/beat/BPM/harmonic + **SectionDetector** (musical section detection: Intro/Verse/Chorus/Build/Bridge/Drop/Outro) |
| `led-show-recorder` | **NEW** — `.lumyx` binary format: write/read `LogicalFrame` + `AudioSnapshot` streams; `pixel_hash` for regression replay comparison |
| `led-readmodel` | read-only snapshot the operator UI polls: `ReadModel` (DeviceStatus + HealthStatus + MetricsView + discovery) + loopback-only serve (ADR-0013/0014) |
| `led-hardware-profile` | **NEW** — design-time capability descriptor (ADR-0018): schema, validator, `const` preset table, `HardwareRegistry`, compile → `CompiledLayout` + `DriverConfig`. **+`Transport` (GS4.3): MTU declarado, fragmentação DERIVADA dele.** Leaf: depends only on `led-core` |
| `led-daemon` | **NEW** — máquina de estados do transporte (ADR-0023, contrato **congelado** na GS1.6). Matriz exaustiva 8×10 = 80 pares; `PositionChanged` carrega `cause`; `Transitioned` só quando o estado muda; `no_show_loaded` por guarda única |
| `led-daemon-bin` | **NEW** — processo daemon (GS2) + **IPC UDS owner-only (GS3)** + **camada de saída (GS4.1/4.2)**: `OutputManager` (DDP/Art-Net/sACN), `FrameSource` e `Stage` — **ligados ao laço**, com pré-voo real (WifiBlockGuard + ArtPoll) e heartbeat conduzido pelo tick. Protocolo v1, `ledctl`, um só aplicador. Carrega `.lumyx` em **fluxo**, tica em cadência absoluta, emite JSONL, encerra limpo. Pacer injetável ⇒ laço testável sem relógio de parede |
| `led-demo` | show.gif renderer |

**TD-004 CLOSED** (2026-06-26): wgpu 22.1.0 — Metal headless no longer hangs. Real GPU executor implemented (`crates/led-pixel-engine/src/gpu_executor.rs`): `GpuContext::try_init()` + `GpuPlasmaExecutor` (pre-allocated buffers, per-frame dispatch, readback). 3 GPU tests pass (init_does_not_hang, parity_with_cpu, deterministic). Paridade CPU/GPU validada com tolerance ≤ 1 LSB per channel.

## Conventions

- Std-only where possible; add a dependency only with a reason. `audio-core` is the first
  crate with real external dependencies (`cpal`, `rustfft`, `tokio` sync) — justified by
  its CPAL/FFT/watch-channel pipeline contract; it remains a leaf so this doesn't ripple
  into the rest of the workspace.
- New seam type or change → edit `led-core` in one place, update both sides + this file.
  (`audio-core`'s `AudioFeatures` is a separate, self-owned contract — see crate map.)
- A new `unsafe` block must come with a test that exercises it (and Miri if concurrent).

## Session changelog

Newest first. One entry per session (`/changelog`): Done · Invariants verified · Pending · Decisions.

> As **decisões estruturais** (seams, invariantes, escolhas difíceis de reverter)
> estão registradas como ADRs em [`docs/adr/`](./docs/adr/README.md) no formato
> MADR. Uma decisão nova de peso ganha um ADR; correções e features aditivas
> continuam aqui no changelog.

### 2026-08-07d — GS4.4: `--profile` obrigatório, e o MTU deixa de ser decorativo

**Done.** O caminho da CLI deixou de adivinhar hardware. `--profile <preset>` é **obrigatório sempre que há `--output`**, e o `OutputConfig::parse` — que preenchia cor e universos com omissões — **foi removido**. `led-daemon` e `led-core` continuam intocados.

**Por que remover em vez de manter com omissões melhores.** As omissões eram `RgbOrder::Rgb` e 170 px/universo. Estavam **erradas** para o rig real, cujos WLED são GRB. Um valor errado por omissão é pior que a ausência de valor: parece configuração, e ninguém o confere. Sem `--profile`, o daemon agora **recusa arrancar** com exit 2 e uma mensagem que aponta o `--list-profiles`.

**O segundo defeito, que o teste de MTU encontrou.** A fatia anterior declarou `Transport::mtu_bytes` e derivou dele a fragmentação — mas **ninguém honrava a derivação no fio**. O `DdpDevice` fragmentava pelos seus próprios 487 fixos, e o teste apanhou-o de imediato: *"MTU 576: previsto 5, no fio 2"*. Um MTU declarado que o fio ignora é pior que não o declarar — dá a impressão de configuração onde há uma constante. Corrigido com `DdpDevice::set_max_pixels` + `DdpOutput::with_limits`, e o teto **nunca sobe** acima do que o buffer comporta: um profile com MTU maior que a rede real produziria datagramas que se perdem.

**O `universes_equiv` também deixou de ser 170 escrito à mão** — o DDP não tem universos, e esse número existe só para o `universe_count()` do `ProtocolOutput`; passa a vir do profile em vez de ser assumido.

**O esquema não é uma segunda fonte de protocolo.** `--output` passou a aceitar `IP[:porta]`, e `ddp://IP` continua a funcionar — mas apenas se **concordar** com o preset. Discordar é erro, não é uma escolha: `artnet://` sobre um preset DDP devolve *"contradiz o profile"*. O operador pode escrever o que já escrevia sem que exista um segundo sítio de onde o protocolo possa vir.

**A porta continua a não ser configuração física, e isso é uma decisão.** 4048/6454/5568 são **identidade dos protocolos** (IANA/spec) — mudam quando o protocolo muda, e o protocolo vem do profile. Pô-las no `HardwareProfile` seria declarar como propriedade do nó algo que é do protocolo.

**Bytes medidos, não deduzidos** (`tests/wled_driver.rs`, 16 testes). RGB/GRB/BGR produzem **três resultados distintos** (`[200,100,50]` · `[100,200,50]` · `[50,100,200]`) e o teste falha se dois coincidirem. RGBW põe 4 canais no fio com o branco do ADR-0020 — subtraído, não somado; um teste que só contasse canais passaria com o modo aditivo antigo, que consumia 4× mais corrente. MTU 576/1000/1500 → 5/3/2 datagramas, **e nenhum datagrama excede o MTU**. `first_universe` 0/1/7/100 sai consecutivo no fio.

**Gate falsificado 3×** (KB-012). (1) `RgbOrder::Rgb` fixo → **6 testes** reprovaram. (2) 170 px/universo fixo → o teste de MTU reprovou. (3) fragmentação fixa em 487, ignorando o MTU → *"MTU 576: previsto 5, no fio 2"*. Produção restaurada e verde nos três.

**Erro meu nesta rodada.** A asserção de monotonia do teste de MTU estava **invertida** — escrevi `previsto > anterior` quando MTU maior fragmenta **menos**. O gate real já tinha passado; foi a minha própria verificação que estava errada, e só a distingui da falha verdadeira depois de comparar o ficheiro com o backup byte-a-byte.

**A auditoria do PASSO 6 é um teste, não um `grep` de uma vez.** `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida` lê `output.rs`, `stage.rs` e `run.rs` com `include_str!` e reprova se `RgbOrder::…)`, `170`, `487`, `1462` ou `1500` aparecerem **fora de comentário**. Um `grep` prova o estado de um instante; isto prova-o em cada `cargo test`.

**Invariants verified.** **930 testes** no workspace, clippy `-D warnings` exit 0, `led-daemon` e `led-core` intocados, **nenhum teste removido**. Executado a sério: `--list-profiles` lista os 8 presets com protocolo, cor, universos, MTU e heartbeat; `--output` sem `--profile` → **exit 2** com a mensagem certa.

**Pending.** `--output` aceita **um** alvo — multi-controlador continua a ser outra fatia, e é o que separa 720 px de 6.200. A `Calibration` (gamma/brightness) do profile continua **sem ser ligada** ao HAL pelo daemon: está declarada desde o ADR-0019 e o caminho existe (`Hal::with_calibration`), mas o `OutputManager` não a passa. É a mesma classe de defeito que o `RgbOrder` era — campo declarado que ninguém honra — e nomeio-a agora em vez de a deixar para outro teste a encontrar. GS4.5–GS4.7 seguem bloqueados pelo rig.

### 2026-08-07c — GS4.3: o `HardwareProfile` passa a mandar na saída (e apanha um defeito de cor)

**Objeção ao PASSO 1, e o que fiz em vez dele.** O sprint pedia "criar HardwareProfile". Ele **já existia** desde o ADR-0018 (2026-07-29b): identity, protocolo, universos, pixels, cor, limites. Criar um segundo seria exatamente o caminho paralelo que o próprio sprint proíbe. **Estendi o que havia** com o que faltava — MTU, fragmentação e heartbeat.

**A fragmentação não é declarada: é derivada.** `Transport { mtu_bytes, heartbeat_ms }` dá só o MTU; `pixels_per_datagram` calcula o resto a partir dele, do protocolo e do formato de cor. Escrever "487 px por datagrama" ao lado de "MTU 1500" seria a mesma verdade duas vezes, e a segunda apodreceria em silêncio no dia em que a primeira mudasse. Há um teste que confirma que a derivação **reproduz** o `DDP_MAX_PIXELS = 487` já validado no rig (1500 − 20 IP − 8 UDP − 10 DDP = 1462; 1462/3 = 487). A concordância entre as duas fontes é **provada**, não assumida.

E a derivação sabe distinguir o que prende cada protocolo: DDP é preso pelo **MTU**; Art-Net e sACN são presos pelo **universo** (512 canais), que é muito menor. Dizer que o MTU limita o Art-Net seria descrever o protocolo errado.

**O defeito que a integração encontrou — e que teria aparecido no palco.** O `OutputManager` construía o layout com `RgbOrder::Rgb` **fixo no código**, ignorando o `ColorFormat::Rgb(Grb)` que os presets WLED declaram desde o ADR-0018. Falsifiquei-o repondo o código antigo: vermelho puro saía `[255, 0, 0]` onde um nó GRB exige `[0, 255, 0]`. **Vermelho teria acendido verde na fita real.** O profile já sabia a resposta certa; faltava alguém consultá-la.

**Integração sem segundo caminho.** `OutputConfig::from_profile` e `OutputConfig::parse` produzem o **mesmo tipo**, e o `OutputManager` continua com um único construtor — o que muda é a *procedência* dos campos, não o caminho dos bytes. Há um teste que afirma que os dois coincidem quando os dados coincidem. Endereço e primeiro universo **não** vêm do profile: são da instância, não do tipo de hardware (ADR-0018).

**Configurações duplicadas eliminadas.** O `HEARTBEAT_MS` do `stage.rs` deixou de ser um número solto e passa a derivar de `led_protocols::HEARTBEAT_MS`, com um teste que falha se as fontes divergirem. Um profile com heartbeat fora do teto de 2400 ms do `LUMYX_GOSL` **não abre saída nenhuma**; um show maior do que o nó declara suportar é **recusado na construção**, não descoberto no palco com metade da fita apagada.

**Descoberta sobre sockets reais, sem mocks** (`tests/discovery.rs`). Um controlador de mentira responde na loopback com um `ArtPollReply` construído pelo próprio `led-protocols`, e a presença é decidida sobre os bytes que voltaram. Com controle negativo: silêncio é ausência, **a resposta de um nó não mascara o silêncio de outro**, e lixo no fio não vira um controlador descoberto.

**Sobre "DDP discovery": não existe, e não o inventei.** A especificação do DDP não define descoberta. Um alvo DDP descobre-se pelo ArtPoll do mesmo nó (o WLED responde independentemente do protocolo de saída — precedente de 2026-07-12) ou por HTTP. Está escrito na doc do módulo `inventory`, em vez de eu escrever um protocolo que nenhum controlador fala.

**Inventário com três categorias, não duas.** `Inventory` separa *presente*, *ausente* e **não sondado**. Manter a terceira é o que impede "não sei" de ser arredondado para "sim".

**Invariants verified.** `led-daemon` e `led-core` **intocados**. **924 testes** no workspace, clippy `-D warnings` exit 0. Nenhum teste removido. `led-hardware-profile` continua leaf (o teste do MTU compara com o literal 487 **de propósito** — importar o `led-protocols` fá-lo-ia deixar de ser leaf, e o objetivo é comparar duas fontes independentes, não colar uma na outra).

**Runbook reescrito** ([gs4-hardware-ethernet.md](./docs/runbooks/gs4-hardware-ethernet.md)): 9 etapas, cada uma com **Objetivo · Procedimento · Critério de aceite · Evidência esperada · Resultado**. **Nenhuma marcada como concluída** — todos os campos de resultado vazios. A etapa 3 traz o controle negativo obrigatório (cabo desligado → exit 1) e o aviso do KB-013 sobre ler exit codes sem pipe.

**Bloqueado por hardware, explicitamente.** O que nenhum teste desta fatia prova: que um WLED **aceita** estes bytes e acende os pixels certos. Tudo o que está provado descreve o que **sai** do daemon; nada descreve o que **entra** no controlador. O runbook abre com essa distinção e lista o que já está provado sem rig, para não se repetir trabalho.

**Pending.** `--profile <preset>` ainda não está na CLI do daemon: `from_profile` existe e está testado, mas o binário só aceita `--output proto://host`, que usa as omissões (RGB, 170 px/universo). **Enquanto isso, o daemon continua a enviar RGB para nós GRB** — o defeito está corrigido no caminho do profile, não no caminho da CLI. É a primeira coisa a fazer, e não a dou como feita. `--output` continua a aceitar **um** alvo.

### 2026-08-07b — GS4.2 (integração): a saída entra no laço e a vacuidade do pré-voo acaba

**Done.** O `--output` deixou de ser uma peça ao lado. `run` e `run_with_control` chamam agora `Stage::on_tick`, o pré-voo consulta rede e controladores a sério, e o heartbeat mantém o palco vivo. `led-daemon` continua **intocado** (`git diff -- crates/led-daemon/` vazio): o contrato congelado na GS1.6 atravessou mais uma fatia sem mudar.

**Um só caminho, e é uma decisão, não uma preferência.** [`stage.rs`](./crates/led-daemon-bin/src/stage.rs) é o **único** sítio do daemon que põe bytes no fio, e os dois laços chamam a mesma função. O heartbeat **não corre numa thread própria** — é conduzido pelo tick: uma segunda thread a enviar seria exatamente o caminho paralelo a evitar, e faria o determinismo do laço passar a depender do escalonador. Para o fechar por construção, `OutputManager` passou a **ser** um `ProtocolOutput`, e o `Heartbeat` do `led-hal` usa-o diretamente — mesmo socket, mesmas estatísticas.

**A vacuidade desapareceu sozinha, como estava escrito que desapareceria.** O changelog do GS2 prometia: *"quando o GS4 ligar a saída, os dois passam a ser verificações reais e a vacuidade desaparece sozinha"*. [`preflight.rs`](./crates/led-daemon-bin/src/preflight.rs) cumpre-o — `network_ok` vem do `WifiBlockGuard` (ADR-0005) e `devices_present` da descoberta ArtPoll (a mesma do `--require-all`, RT-003).

**Provado contra alvo real, não só em teste.** `led-daemon striptest.lumyx --output ddp://192.168.2.156`:

```
network_refused  · WiFi ATIVO em en0 — ADR-0005 proibe show ao vivo
devices_missing  · SEM resposta de 192.168.2.156 — palco escuro se o show comecar
arm_refused      · preflight_failed
shutdown         · NeverStarted · ticks=0 · skipped=0        (exit 1)
```

**Os dois gates dispararam ao mesmo tempo, um por razão diferente**, e o daemon não tocou. E contra loopback, o caminho feliz: 40 ticks → **80 datagramas DDP** (720 px fragmentam em 2 por frame), exit 0.

**Sondas injetadas.** `preflight` recebe `NetworkGuard` e `DevicePresence` como **dados** — a mesma disciplina do validador do ADR-0018. É o que torna a *lógica* do pré-voo falsificável sem rede, sem WiFi e sem hardware, que é precisamente a parte que não se pode testar num rig que não existe.

**A exceção do loopback, e porque não é um bypass.** Um alvo `127.0.0.1` não atravessa interface nenhuma, por isso o gate do ADR-0005 não se lhe aplica — mesmo raciocínio da vacuidade do GS2, aplicado a um caso concreto. Não é uma porta dos fundos: um show apontado ao loopback não chega a rig nenhum, logo não há nada que a regra pudesse salvar. E a sonda de presença **recusa-se a inventar um rig** ali: emite `devices_unverified`, nunca `devices_checked`. O controle negativo `num_alvo_de_rede_o_wifi_ativo_reprova_mesmo` fica vermelho se o ramo alastrar.

**Transporte não apaga o palco, agora em código executável.** A decisão 3 do ADR-0023 e o invariante do heartbeat do `LUMYX_GOSL` passaram a ser a mesma linha: em `Paused`/`Stopped`/`Finished`/`Ready` sai o **último quadro válido** a cada 800 ms. Testes afirmam que o keep-alive reenvia o byte real (não zeros), que o maior intervalo fica **abaixo dos 2400 ms**, e que **antes do primeiro quadro nada sai** — nunca se fabrica um preto para um palco que não tocou.

**Invariants verified.** `led-daemon` e `led-core` intocados. **84 testes** em `led-daemon-bin` (61 lib + 4 bin + 4 e2e-output + 3 e2e + 14 IPC + 2 pipeline), clippy `-D warnings` exit 0.

**Gate falsificado 2×** (KB-012). (1) Removida a chamada a `tick_do_palco` do laço: `o_daemon_envia_frames_reais_em_ddp_artnet_e_sacn` reprovou com *"NENHUM datagrama saiu do daemon — a saída não está ligada"*. (2) Trocado `if cfg.addr.ip().is_loopback()` por `if true` (o ramo do loopback a alastrar, que **apagaria o gate do WiFi**): 4 testes de pré-voo reprovaram, incluindo o controle negativo. Produção restaurada e verde nos dois casos.

**Erro de envio não derruba o laço**, e também não inunda o journal: a primeira falha é registada como `output_error`, a contagem completa vive em `OutputStats`. Falhar em silêncio é que é proibido.

**Pending.** Um erro de saída **não muda o estado** da máquina — o show continua a avançar com o fio partido, e só o journal e as estatísticas o dizem. Passar isso a `Fault` exigiria uma política de quantas falhas consecutivas contam, e essa política não está decidida: fica nomeado, não escondido. `--output` aceita **um** alvo; multi-controlador continua a ser outra fatia. GS4.3–GS4.7 seguem bloqueados pelo rig (ver runbook).

**Decisions.** `Config` deixou de ser `Copy` (ganhou `output: Option<String>`) — é sempre passada por referência, não custa nada. Saída impossível é **`NeverStarted`**, não um aviso: um show que não alcança o rig não deve fingir que toca. O palco é reaberto a cada `load` por IPC, porque é o **show** que dimensiona a saída — um show de 720 px não pode sair por uma saída de 300.

### 2026-08-07 — GS4.1/GS4.2: a camada de saída e o primeiro frame no fio

**Done.** O daemon ganhou uma **saída**. Dois módulos novos em `led-daemon-bin`: `output.rs` (`OutputManager`, `OutputConfig::parse("proto://host[:porta]")`, três protocolos) e `source.rs` (`FrameSource`: `.lumyx` → o quadro da posição do transporte). `led-daemon` **intocado** — o contrato congelado na GS1.6 não mudou uma linha.

**Nenhuma segunda implementação de protocolo.** `output.rs` **não serializa pacotes**: DDP reusa `led_player::DdpOutput`, Art-Net e sACN reusam `led_protocols::{ArtNetDevice, SacnDevice}` por trás do `Hal` — o mesmo caminho validado em hardware (94/94 frames, 2026-07-20). Um segundo serializador seria uma segunda coisa para divergir do fio que já funciona. O daemon fala com `send()` e mais nada: trocar `ddp://` por `artnet://` não toca uma linha de `run.rs`.

**Em fluxo, com um cursor.** `robot_sequence.lumyx` são 73 MB; `FrameSource` mantém **um** quadro em memória e o cursor só anda para a frente. Seek para trás **reabre o ficheiro** em vez de guardar tudo para conseguir recuar — custa I/O num evento raro (o operador salta) e mantém a memória constante no caso comum (o show a correr). É a lição da F2 do wearable aplicada ao transporte.

**A prova é UDP real, não mock.** `tests/pipeline.rs` escreve um `.lumyx`, passa-o pelo mesmo loader do daemon, percorre três posições e **lê os datagramas de um socket** — nos três protocolos, 3 frames / 0 erros cada. Um segundo teste faz o mesmo com seek para trás. E `a_configuracao_escolhe_caminhos_realmente_diferentes` afirma que Art-Net começa por `Art-Net`, sACN contém `ASC` e o DDP difere de ambos: sem isso, três configurações a chamar o mesmo código passariam iguais — o falso-verde do KB-012.

**Erro engolido é indistinguível de sucesso.** `OutputStats{frames_sent, errors}` existe para que a diferença seja observável, e `erro_de_envio_e_contado_e_devolvido` afirma que `frames + errors == 1` **sempre** — um envio nunca desaparece da contabilidade.

**Invariants verified.** **70 testes** em `led-daemon-bin` (26 GS2 + 31 GS3 + 13 GS4), clippy `--workspace --all-targets -D warnings` exit 0.

**Pending — e é o que impede fechar o GS4.** (a) O `--output` ainda **não está ligado ao laço** do binário `led-daemon`: `OutputManager` e `FrameSource` existem e o pipeline entre eles está provado, mas o `run_with_control` ainda não os invoca. O `--help` continua a dizer "este processo não tem saída" e **continua verdadeiro**. (b) Enquanto isso, o pré-voo continua **vacuosamente** verdadeiro — a promessa do GS2 (`network_ok` do `WifiBlockGuard`, `devices_present` do `discover_controllers`) só se cumpre quando a saída entrar no laço. Nenhuma das duas foi dada como feita.

**GS4.3–GS4.7 — BLOQUEADO por hardware, e não afirmado.** Verifiquei antes de escrever código: os cinco nós (`192.168.2.156–160`) **não responderam a ping**. Não há ESP32-POE, switch nem cabos na rede. Em vez de carimbar critérios, escrevi [`docs/runbooks/gs4-hardware-ethernet.md`](./docs/runbooks/gs4-hardware-ethernet.md) no formato de 2026-07-20 — 8 etapas, cada uma com o campo de evidência por preencher (⏳, nunca ✅), o aviso do ABL a 850 mA, e a nota de que o sACN, se falhar, é o firmware do WLED 16.0.1 e não o LUMYX.

**Decisions.** `OutputConfig` recebe a porta **opcional** e cai no padrão do protocolo — escrever `:4048` à mão em cada invocação é uma oportunidade a mais de errar um número já conhecido. CID fixo `LUMYX-DAEMON-001` no sACN: um receptor E1.31 distingue fontes por CID, e dois senders com o mesmo CID seriam indistinguíveis no diagnóstico. `FrameSource::black()` está documentado como **não** sendo blackout — é conteúdo de um quadro, não máscara de saída; o ADR-0017 continua bloqueado e nada aqui o contorna.

### 2026-08-05e — GS3: IPC sobre UDS owner-only + `ledctl`

**Done.** O daemon passou a ser controlável. Protocolo v1 em `docs/architecture/ipc-protocol-v1.md`; implementação em `json.rs` (parser), `proto.rs` (tipos), `server.rs` (UDS) e o cliente `ledctl`. **`led-daemon` intocado** — `git diff -- crates/led-daemon/` vazio: o contrato congelado na GS1.6 atravessou o fio sem mudar.

**A decisão de arquitetura: um só aplicador.** As threads de ligação **nunca tocam** o `ShowRuntime` — analisam, validam e **enfileiram**; o laço aplica no limite do tick. Não é só conformidade com o `control-protocol.md`: é o que faz o determinismo do ADR-0023 **sobreviver à chegada da concorrência**. E `status` não passa pela fila (lê um snapshot publicado), então **consultar nunca compete com comandar**.

**Três escolhas de segurança que não são estéticas.** (a) Socket `0o600` — e **sem TCP**, `0.0.0.0` não é sequer representável, o que é mais forte que verificá-lo em runtime. (b) **Limite de 64 KiB por linha** — sem ele, um cliente que nunca envie `\n` faz o daemon crescer sem limite. (c) **Limite de profundidade no parser JSON** — `[[[[[…` recursivo estouraria a pilha; um cliente derrubaria o daemon com uma linha de texto.

**`shutdown` em duas fases**, embora o enunciado não pedisse. A spec exige duas fases para ações irreversíveis e já define `confirmation_required`; hoje o daemon não tem saída, mas no GS4 terá — e **acrescentar confirmação depois de existirem clientes custa versão de protocolo**. Token de uso único; não é segredo criptográfico (o socket já é owner-only), existe contra o **engano**.

**O gate do pré-voo ficou visível no fio.** `load` sem `assume_integrity` deixa em `loaded`, e o `play` seguinte devolve **`not_armed`** — em vez de um arm implícito que esconderia a decisão do ADR-0023 do cliente.

**Invariants verified.** **870 testes** (839 + 31), clippy `-D warnings` exit 0. Os códigos de recusa do runtime vão **inalterados** para o fio (`no_show_loaded` significa o mesmo dos dois lados — foi para isto que se congelou o contrato). O `id` sobrevive a qualquer erro analisável: é extraído **antes** de validar o resto, para o cliente nunca ficar à espera de uma resposta que não vem.

**Prova real com os dois binários**, não só testes: `ping`/`version`/`status` → ok; `load` sem integridade → `loaded` e `play` → `not_armed` (exit 1); com `--assume-integrity` → `ready` → `playing`, `status` a 1 s = `position_ms:998`, `duration_ms:8100`, `ticks:52`; `seek 4000` → 4000; `pause` → `paused` em 4074; `shutdown --yes` → `confirmation_required` e depois `shutting_down`, exit 0, e o daemon encerrou com `ShutdownRequested · ticks=56 · skipped=5`.

**Erro meu nesta rodada.** Nos testes extraí o token de confirmação com `split('"').nth(1)`, que devolve `confirm` — o **rótulo**, não o valor (o `ledctl` usava `nth(3)`, correto). Um teste de duas fases com o token errado testa a **rejeição**, não a aceitação. Substituí por uma função `token_de()` que procura a marca `"confirm":"`, com o porquê no comentário. Também levei ~10 min a suspeitar de um travamento que era **tempo de compilação**: isolei teste a teste e todos passavam.

**Pending.** Sem TCP/token/mTLS (same-host por agora; a LAN é do ADR-0014 e espera por appliance). Sem `SIGINT` ainda — mas agora há `shutdown` remoto, que era a via prevista. `ClientRegistry` está declarado e vazio (identidade por ligação é do GS4).

**Decisions.** JSON **parser próprio** (o repo já emite à mão e já tem precedente de parser em `led-xlights`) — nenhuma dependência nova no workspace. Eventos assíncronos **não têm `id`**: é assim que o cliente os distingue de uma resposta, sem campo extra. Subscritor morto é **podado**, senão o `Sender` acumula para sempre.

### 2026-08-05d — GS2: o processo daemon (`led-daemon-bin`)

**Done.** A máquina de estados ganhou um processo. Crate novo `led-daemon-bin` (lib + bin `led-daemon`), com `led-daemon` **intocado** — o contrato congelado na GS1.6 não mudou uma linha, e o crate da máquina continua **sem dependências**.

**Três decisões que podiam virar mentira, e como as resolvi:**

1. **Pré-voo sem rede nem dispositivos.** O GS2 **não tem saída**: nenhum frame deixa o processo. Logo `network_ok` e `devices_present` são **vacuosamente** verdadeiros — um show que não envia nada não pode enviar por WiFi nem perder um controlador. É logicamente correto, não um atalho, e a vacuidade **desaparece sozinha** quando o GS4 ligar a saída. O journal regista-a em cada arranque (`preflight_vacuous`).
2. **Integridade.** `pixel_hash` recebe `&[ShowRecord]` — exige o show inteiro em RAM (`robot_sequence.lumyx` são 73 MB). Hash em fluxo não existe. Então o daemon **não verifica**: exige `--assume-integrity`, e o journal diz `AFIRMADA pelo operador, NAO verificada`. `Integrity` é um enum, não um `bool`, precisamente para que "assumido" e "verificado" não fiquem indistinguíveis. **Sem a flag, o pré-voo reprova e o daemon não toca.**
3. **"Sem busy-loop" testável.** `Pacer` é **injetável**. Testar com relógio de parede seria instável sob carga (TD-003). Com pacer virtual, "dormiu a cada iteração" vira asserção determinística — e os prazos são **absolutos** (`20,40,60,80,100`), então o período não deriva com o custo do trabalho. **Nenhum teste deste crate dorme.**

**Mais duas escolhas.** O `.lumyx` é percorrido **em fluxo**, um quadro de cada vez — o pico de memória não pode depender da duração (lição da F2 do wearable); e o loader **não confia** no `frame_count` do cabeçalho, que vem a zero em escritores não-*seekable* (há teste que afirma essa premissa). A serialização JSON vive no **bin**, não no `led-daemon`: o formato do fio é contrato do GS3 e não deve ser congelado antes de existir cliente.

**Invariants verified.** **839 testes** (813 + 26), clippy `-D warnings` exit 0, `led-core` e `led-daemon` intocados. Tick superado é **saltado**, nunca acumulado como dívida (mesma regra do `Pacing::Absolute` do player).

**Executado de verdade, não só testado.** `./target/release/led-daemon striptest.lumyx --assume-integrity --max-ticks 5 --log /tmp/daemon.jsonl` → 14 linhas JSONL, `MaxTicks · ticks=5 · skipped=0`, exit 0. Fim de show real: `ReachedEnd · ticks=17`, `position_ms=8100` = duração exata. Encerramento por stdin: `ShutdownRequested`. Exit codes lidos **sem pipe** (KB-013): 1 sem integridade, 0 com, 1 para ficheiro ausente, 2 para CLI inválida.

**Erro meu nesta rodada.** Na 1.ª verificação usei `timeout` (não existe no macOS — a mesma família do KB-013c, `grep -P`) e li o exit code **através de um pipe**, medindo o `tail` em vez do daemon. Refiz sem pipe; foi assim que confirmei os quatro códigos.

**Pending / gap nomeado.** **Não há tratamento de `SIGINT`/`SIGTERM`** — exigiria uma dependência de sinais, e o `shutdown` por IPC é entrega do GS3. Ctrl-C termina o processo mas **abruptamente**: sem linha final de estado nem flush. Está no `--help` e na doc do módulo, não escondido. Também pendente: verificação de integridade a sério (precisa de hash em fluxo no `led-show-recorder`).

**Decisions.** EOF no stdin **não** encerra, só a linha `shutdown` — um daemon supervisionado corre com stdin fechado, e fazer o EOF encerrar mataria o processo com `/dev/null` na entrada. Falha ao escrever o ficheiro de log **não derruba o daemon**: é reportada em stdout e o laço continua (disco cheio não pode parar um show).

### 2026-08-05c — GS1.6: fechamento do contrato do daemon (F1–F4)

**Done.** As quatro divergências da auditoria GS1.5 foram fechadas. Contrato **congelado** para o GS2/GS3. Cada uma foi resolvida pela opção que elimina a **classe** do problema, não a instância — que é a diferença entre corrigir `pause` e impedir que o próximo comando repita o erro do `pause`.

| # | Decisão | Por que esta opção |
|---|---|---|
| **F1** | `Tick` aceite em **todos** os estados, incl. `Error` | A absorção de `Error` é sobre **transições**, e `Tick` fora de `Playing` não faz nenhuma. O laço de cadência do daemon fica livre de estado |
| **F2** | `PositionChanged { ms, cause }` · `PositionCause { Advanced, Sought, Reset }` | **3 causas para 4 origens, deliberadamente**: `pause` e `tick` são ambos `Advanced` — pausar **avança** até ao instante da pausa. Uma 4ª variante descreveria o *comando*, não a *causa* |
| **F3** | `requires_show()` + **guarda única** em `apply` | Era a verificação por-handler que deixou `pause` divergir. Com guarda única a classe deixa de ser possível |
| **F4** | `transition()` devolve `Vec` e só emite se `from != to` | `Transitioned` passa a significar "o estado mudou" **por construção**, para toda transição futura |

**Invariants verified.** `led-core` intocado. **813 testes** (808 + 5), clippy `-D warnings` exit 0, build 0 warnings. Tabela dos 80 pares **regenerada** — o diff mostra exatamente 4 classes de mudança e nada mais. Os três sinais do gerador passam agora: `PositionChanged` carrega causa · **nenhuma** auto-transição emite `Transitioned` · `Transitioned` continua inequívoco (injetividade de `(from→to) → comando` computada, não afirmada).

**Gate falsificado 4×, um bug por correção** (KB-012): (F1) recusar `Tick` em `Error`; (F2) trocar a causa de `seek` para `Advanced`; (F3) `pause` a saltar a guarda; (F4) `transition` a emitir sempre. **As quatro reprovaram.** Produção restaurada e verde.

**Reforço estrutural dos gates:** F2/F4 viraram invariantes verificadas em **todos** os 80 pares (nenhum `Transitioned` com `from == to`; toda `cause` casa com o comando), e F3 virou um teste que percorre **todos** os comandos que exigem show — não só o `pause`. Testar a regra em vez das instâncias é o que impede a reincidência.

**Pending / honestidade.** Numa das execuções de `cargo test --workspace` **um** teste do `led-bridge` falhou; isolado, os 38 passam, e duas execuções seguintes do workspace deram 813/0. **Não capturei qual era** — registo isto como instabilidade observada, não como diagnóstico. O `led-bridge` não depende do `led-daemon`, mas acrescentar um crate ao workspace aumenta a carga paralela, e este repo já tem precedente documentado desta classe (2026-07-09: "3 testes de latência flaky em debug sob carga paralela → budgets condicionais"). Se reaparecer, é candidato a TD.

**Decisions.** Campo `cause` em vez de eventos separados: mantém **um evento por conceito** ("a posição mudou") e uma causa nova é aditiva. Evento `Rearmed` descartado — o `Ok` vazio já confirma o re-armar, e seria funcionalidade nova para um caso já coberto.

### 2026-08-05b — GS1.5: auditoria de contrato do daemon (tabela dos 80 pares)

**Done.** Antes de congelar o contrato para o IPC, auditá-lo. [Anexo do ADR-0023](./docs/adr/0023-anexo-tabela-de-contrato.md) com a **tabela completa dos 80 pares** — `estado → comando → resultado → eventos → próximo estado` — **gerada executando a máquina de produção** (`cargo run -p led-daemon --example contract_table`), não escrita à mão. Uma tabela manual está correta no instante em que se escreve e apodrece no commit seguinte.

**Máquina de estados intocada:** `git diff -- crates/led-daemon/src` vazio. O único código novo é o gerador (ferramenta de documentação; não corre na CI, não afirma veredito).

**As 5 propriedades — 4 passam, 1 falha:**

| Propriedade | Veredito |
|---|---|
| Comandos com semântica única | ✅ com ressalva — `Play` de `Ready`/`Paused`/`Stopped` **não** é sobrecarga: a semântica é uma só, "avançar a partir da posição corrente" |
| Eventos com origem única | ❌ **falha** — `PositionChanged` tem **4 origens** |
| Sem tempo implícito | ✅ — `grep Instant\|SystemTime\|now()\|elapsed()` devolve **só um comentário** |
| Nenhum estado inalcançável | ✅ — os 8 têm caminho de entrada, provado por construção |
| Nenhum estado terminal por acidente | ✅ — `Error` é absorvente **por decisão**, com 2 saídas |

**Quatro divergências encontradas (F1–F4), nenhuma é bug de estado:**

- **F1 🔴** — a doc de `Command::Tick` diz *"aceite em qualquer estado... o daemon não deve ter de saber o estado"*, mas `Tick` é **recusado em `Error`**. Um daemon a ticar em cadência fixa — exatamente o que a doc descreve — recebe um fluxo de recusas. A razão declarada e o comportamento contradizem-se.
- **F2 🔴** — `PositionChanged` sai de `seek`, `pause`, `stop` e `tick`. Um consumidor **não distingue** avanço contínuo de salto do operador — que é exatamente a distinção de que uma timeline de console precisa.
- **F3 🟡** — em `Idle`, `pause` devolve `not_applicable` mas `stop`/`play`/`seek` devolvem `no_show_loaded`. **Mesma causa raiz, dois códigos**, num modelo de erro que é consumido por máquina.
- **F4 🟡** — `ready + arm` emite `Transitioned{from == to}`: evento de mudança onde nada mudou. É a **única** auto-transição que emite.

**Verificado, não deduzido:** o gerador computa a injetividade de `(from→to) → comando` e confirma que **`Transitioned` é inequívoco** apesar de 9 origens — o consumidor deduz a causa do par, sem campo extra. Foi assim que se separou o evento que está bem do que não está.

**Recomendação registada: não congelar ainda.** F1 e F2 são a superfície que o IPC vai serializar; enquanto o contrato só existe em Rust, mudá-los é uma edição — depois de existir no fio com `v` negociado, é versão de protocolo e migração de cliente.

**Pending.** Decisão do responsável sobre F1–F4. Se optar por congelar como está, F1 e F2 devem entrar no ledger de dívida com gatilho no GS3.

**Decisions.** Fragilidade registada sem correção: `cmd_play` tem `State::Error => unreachable!()`, de facto inalcançável, mas a ausência de pânico depende de uma guarda **noutra função** — um refactor que mova a guarda transforma-a em pânico em produção.

### 2026-08-05 — GS1: superfície de transporte do engine (ADR-0023) + `led-daemon`

**Done.** O `control-protocol.md` mediu a lacuna e foi explícito: *"a superfície de transporte do engine (play/pause/seek — **não existe**; precisa de **ADR próprio**)"*. Esta fatia fecha isso — ADR primeiro, código contra ele.

[ADR-0023](./docs/adr/0023-superficie-de-transporte-do-engine.md) + crate novo **`led-daemon`**, com **zero dependências** (nem `led-core`): a máquina não toca frames, pixels nem dispositivos, e o pré-voo chega como **dado**. É o que garante, por construção, que ela não alcança o hot-path.

**As decisões que valem a pena registar:**

1. **`Loaded` e `Ready` são estados distintos.** Carregar não é estar pronto. `Ready` é onde os gates que já existem passam a ter lugar no ciclo de vida: `--verify <hash>`, `Hal::check_network()` (ADR-0005) e discovery `--require-all`. Fundir os dois apagaria a distinção que impede um show de começar sobre WiFi ou com um controlador ausente.
2. **O tempo é injetado** (`apply(cmd, now_ms)`) — a máquina nunca lê o relógio. Determinismo por construção, e **nenhum teste dorme** (a lição do TD-003). Relógio retrógrado é **clampado**, não pânico (precedente do `SharedClock`).
3. **`Stop`/`Pause` NÃO apagam o palco.** Transporte é avanço do tempo; apagar é **saída**. Em `Paused`/`Stopped`/`Finished` o heartbeat continua a reenviar o último frame válido. É a mesma separação que Eos e MagicQ fazem (ver [anexo do ADR-0017](./docs/adr/0017-anexo-analise-e-proposta.md)), e **não existe comando aqui capaz de zerar saída** — há um teste que falha se alguém acrescentar `blackout`/`dbo`/`grand_master`/`intensity` à lista de comandos.
4. **`Play` a partir de `Finished` é recusado.** Rebobinar implicitamente é a classe de surpresa que faz um show **recomeçar no palco** com um toque acidental. O caminho é `Stop`/`Seek` e depois `Play`.
5. **Recusa nunca muda o estado** — `Result<Vec<Event>, Rejected>`, com códigos enumerados (nunca string livre, como o modelo de erro do `control-protocol.md` exige).

**Invariants verified.** `led-core` **intocado** — zero bump, nenhum seam `Frozen` tocado. **Matriz exaustiva de 8 estados × 10 comandos = 80 pares**, cada um com destino ou código de recusa declarado; a tabela **é a especificação executável do ADR-0023**. Invariantes estruturais verificadas **depois de cada** aplicação (`Idle` sem show, `Error` sempre com falha, posição nunca excede a duração). Workspace **808 testes**, clippy `-D warnings` exit 0.

**O gate foi falsificado, não presumido** (KB-012). Plantei `Play` aceite a partir de `Finished` e confirmei que a matriz **reprova** com diagnóstico exato — `(Finished, play): esperava recusa: [Transitioned { from: Finished, to: Playing }]` — e que o controle negativo dispara junto. Restaurado e verde.

**Pending.** O *processo* daemon e o IPC (GS2/ADR-0014) — esta fatia é a máquina, não o servidor. Nenhuma fonte de frames está ligada: `ShowDescriptor` é descritor, e quem carrega o `.lumyx` será o daemon. O `to_json()` ainda não é consumido pelo `led-readmodel`.

**Decisions.** Crate **sem dependências** em vez de depender de `led-core` — não precisa, e não depender é uma garantia mais forte que uma convenção. `State::as_str()` escrito à mão em vez de derivado de `Debug`: é superfície observável pelo control-plane e `Debug` pode mudar num refactor. Sem comando `Resume` — `Play` a partir de `Paused` já é retomar, e um comando a mais é uma linha a mais na matriz sem semântica nova.

### 2026-08-03 — E1 (1ª fatia): biblioteca de efeitos + ADR-0021 (efeito é função pura)

**Done.** O roadmap (`docs/ROADMAP.md`, escrito nesta sessão) mediu a maior lacuna de paridade com o xLights: **5 efeitos contra ~40**. Esta fatia leva a **13** e, mais importante, estabelece a regra que rege os ~25 restantes.

**O conflito que a fatia resolveu.** Bibliotecas de LED (FastLED, WLED, xLights) guardam **estado por-pixel entre frames** — mapa de calor do fogo, `random()` avançando, posição acumulada. É o padrão do setor e é **incompatível** com o LUMYX: um efeito com estado renderiza diferente na segunda passada do mesmo `time_ms`, e todo o replay verificado por hash (+ assinatura Ed25519 + burn-in) deixa de valer. A assinatura já tinha decidido — `Effect::render(&self, …)` recebe `&self`; guardar estado exigiria **mudar o contrato**, não escrever um efeito.

[ADR-0021](./docs/adr/0021-efeitos-funcoes-puras-estado-derivado.md), 3 regras: (1) função pura de `(time_ms, position, index)`; (2) aleatoriedade é **hash de coordenadas**, nunca fluxo; (3) parâmetro espacial é **taxa por metro**, nunca coordenada normalizada — o efeito não conhece o tamanho do rig, e onde a extensão é indispensável ela é **parâmetro declarado** (`Meteor::span_m`), na disciplina "injeção de dado" do ADR-0018.

1. **`led-pixel-engine/src/noise.rs`** — `mix64`/`hash01`/`value_noise`/`fbm`. `mix64` compartilha as constantes do finalizador do SplitMix64 já presente em `show_intent.rs:143` e `chaos.rs:83`, **e isso está documentado**: não é terceira cópia — aqueles são geradores **com estado** (`fn(&mut u64)`), este é **função pura** (`fn(u64)`).
2. **`led-pixel-engine/src/library.rs`** — 8 efeitos: `Chase`, `Twinkle`, `Fire`, `ColorWash`, `Strobe`, `Meteor`, `Lightning`, `Ripple`.
3. **`led-pixel-engine/tests/no_alloc.rs` (novo)** — contraparte de render do gate que o `led-hal` já tinha no envio. Alocador contador, **2.000 frames × 512 px por efeito**, os 11 efeitos. **Com controle negativo** (KB-012): um efeito que aloca de propósito **tem que ser pego** — senão o gate não estaria provando nada.
4. **Show real do usuário destravado** (`led-demo/examples/robot_sequence.rs`): `Lightning → Pulse 6 Hz` e `Meteors → Rainbow` eram aproximações **documentadas como tais**. Agora são `Lightning` e `Meteor` nativos. O `x_span` do rig é medido **do layout, no exemplo** — não pelo efeito (regra 3).
5. **`led-demo/examples/effect_gallery.rs` (novo)** → `effect_gallery.gif`: 12 faixas, 200 frames. Efeito é coisa que se **vê**; teste de unidade prova geometria e pureza, não aparência.

**Invariants verified.** `led-core` **intocado** — `Effect` vive no `led-pixel-engine` e **não** está em `certified_contracts()` (`contract_version.rs:74-83`): zero bump, zero seam Frozen tocado. Gate de pureza executável (`every_effect_is_a_pure_function_of_time`: renderiza `t`, depois `t+1`, depois `t` — estado interno faria a 3ª passada divergir da 1ª). Gate de alocação verde com controle negativo. Controle negativo de `NaN`/`∞` em posições e em ruído — a classe do **BUG-3** (`smoothstep(NaN)` propagando até posição de drone).

**Erros meus nesta rodada.** (a) O 1º teste do `Ripple` falhou e a **falha era do teste**: tentei isolar a atenuação com comprimento de onda enorme, mas isso fez o termo da crista variar junto e dominar — isolamento correto é comparar pixels na **mesma fase**. (b) Escrevi `0xL1` e `0xC0_ME7A` como sementes: `L`, `M` e `R` **não são dígitos hexadecimais**. Duas vezes.

**Pending.** ~25 efeitos para paridade. **`robot_sequence.lumyx` NÃO foi regenerado**: o hash `0xd8f1479ff3645e1e` é parâmetro de verificação do runbook de palco (`docs/runbooks/show-startup.md:65`, `--verify`), e regenerar o mudaria — decisão do operador, não do agente. Efeitos com difusão real (`Fire2012`, fluidos, `Life`) continuam fora: exigiriam um `StatefulEffect` com contrato próprio e **ADR próprio**.

**Decisions.** `Fire` é ruído fractal, **não** `Fire2012` — visualmente próximo, algoritmicamente diferente, e isso está na doc do tipo em vez de escondido. `Strobe` expõe `SEIZURE_RISK_HZ`/`is_in_seizure_risk_band` e **não clampa em silêncio**: precedente do ADR-0018 (o componente declara, a camada com contexto decide) — estroboscópio que muda de frequência sozinho no palco é pior que parâmetro documentado.

### 2026-08-02 — ADR-0020: RGBW subtrativo (achado com consequência física)

**Done.** Revisão externa apontou que o RGBW podia estar aumentando a corrente; a verificação no código **confirmou**.

`led-core/src/types.rs:97-103` escrevia os bytes RGB **intactos** e **acrescentava** o branco — a própria doc dizia "the RGB bytes are left unchanged (simple, non-destructive)". Branco pleno virava `[255,255,255,255]`: **quatro dies no máximo**.

| Modo | por pixel (SK6812) | 720 px | vs fita RGB |
|---|---|---|---|
| RGB (3 canais) | 60 mA | 43,2 A | — |
| `Min` (era o padrão) | **80 mA** | **57,6 A** | **+33 %** |
| `MinSubtract` (novo padrão) | 20 mA | 14,4 A | −67 % |

**Razão de 4x** entre os dois modos para branco pleno. Além da corrente, o die branco **somava** luz ao branco RGB — a saída ficava mais brilhante que a cor lógica pedida, fazendo o `brightness` do `Calibration` mentir sobre a intensidade real.

1. **`WhiteMode::MinSubtract`** — `W = min(r,g,b)` **subtraído** dos três canais (satura em zero), o comportamento colorimétrico padrão: o neutro sai pelo die dedicado (mais eficiente, melhor CRI) e só o excedente de cor permanece no RGB.
2. **`Min` preservado byte-a-byte** — há uso legítimo do branco aditivo; a doc agora **avisa** sobre a corrente em vez de omitir.
3. **`residual_rgb()`** separa "qual é o branco" de "o que sobra no RGB", e o `write` aplica a **ordem de canais ao resíduo** — ordenar antes de subtrair produziria bytes errados.
4. **Presets RGBW passaram a `MinSubtract`** (padrão seguro; nenhum foi validado em hardware ainda).

**Invariants verified.** `ColorFormat` é `Evolving` → variante nova é **aditiva**: bump MINOR 1.3.0 → 1.4.0, **zero contrato Frozen tocado**. SemVer Guardian: "superfície mudou COM bump — intencional". 9 testes novos, incluindo o **gate elétrico verificado, não afirmado**: `assert_eq!(additive/subtractive, 4)` para branco pleno; mais saturação (nunca dá a volta), cor saturada intocada, e ordem de canais aplicada ao resíduo.

**Pending.** Trocar o padrão **mudou bytes no fio**: 1 teste que fixava o comportamento aditivo foi atualizado para a semântica subtrativa. Nenhum preset RGBW foi validado em hardware — a razão de 4x é derivada de corrente nominal por die, não medida no rig.

**Decisions.** Adicionar variante em vez de mudar `Min` — mudar silenciosamente os bytes de quem já usa seria pior que ter duas semânticas documentadas. Isto **não é proteção elétrica**: o limite real continua sendo a fonte e o ABL; `MinSubtract` reduz a corrente de branco, mas não autoriza aumentar carga.

### 2026-07-30 — FASE A: RGBW pixel-nativo no DDP (A2) + últimas dívidas da FASE 1 (A3)

**Done.**

1. **A2 — RGBW pixel-nativo no DDP.** Até aqui RGBW só saía por sACN/Art-Net (byte-transparentes); o caminho DDP era RGB-only. Novo `build_ddp_packet_format` delega cada pixel a `ColorFormat::write` (ADR-0011) — **nenhuma segunda derivação de branco no projeto**. `max_pixels_per_packet(format)` corrige a fragmentação: RGB cabe 487 px/pacote, **RGBW cabe 365** (1462/4). `DdpDevice::with_format` e `DdpOutput::with_format` propagam o formato, e `--profile` + `--ddp` passa a honrá-lo. Hardware novo continuou sendo **uma linha**: preset `esp32-poe-wled-rgbw-ddp` adicionado sem tocar em código.

   **Dois cuidados com o caminho validado em hardware:** (a) `DDP_DTYPE_RGB8 = 0x01` **não foi alterado** — descobri que ele não segue a codificação publicada do DDP (que daria `0x13`), mas é o valor que o WLED aceitou no rig real (94/94 frames, 2026-07-20); a discrepância ficou documentada e alterar exige re-validação; (b) `DDP_DTYPE_RGBW8 = 0x33` segue a spec mas está marcado **não validado em hardware**, e o validador emite `RgbwOverDdpDataType`. Teste prova retrocompatibilidade **byte-a-byte**: RGB pelo builder novo é idêntico ao antigo.

2. **A3/L1 — `verify_manifest` depreciado.** Fecha uma nota do próprio ADR-0004. A função prova integridade, não autenticidade (um atacante re-assina com a própria chave e ela retorna `Ok`). Só tinha chamadores em testes; estes ganharam `#![allow(deprecated)]` **com justificativa** — inclusive a prova de red-team que motivou a depreciação. O caminho de fronteira de confiança continua sendo `verify_manifest_pinned`.

3. **A3/M6 — `CompiledLayout::compile` medido.** Crescimento **quadrático confirmado** (1k→0,91 ms · 6,2k→5,37 ms · 25k→46 ms · 50k→142 ms · 100k→517 ms; a 2× pixels o tempo cresce ~3,1–3,6×). Mas `compile` roda **uma vez no startup, nunca no hot path**, e no rig real (6.200 px) custa **5,4 ms**. Registrado como **TD-012 `wontfix`** com gatilho (rigs > ~50k px) e uma **guarda falsificável que roda sempre**: 6.200 px deve compilar em < 1 s.

**Invariants verified.** `led-core` intocado — SemVer Guardian: "superfície de seam inalterada (v1.3.0, 61 itens)". Audit gate: 0 Critical, 0 Warning, 10 OK. Caminho RGB do DDP byte-idêntico ao validado em hardware.

**Pending.** O data type RGBW do DDP precisa de validação em rig com fita RGBW. Depois desta fase o trabalho gateável sem recurso externo se esgota: o que resta depende do spike da ADR-0016 (a11y/GPU/DX), da migração WiFi→Ethernet, da decisão do ADR-0017 (blackout) e do burn-in 72h.

**Decisions.** Não alterar `DDP_DTYPE_RGB8` sem re-validar no rig — evidência de hardware vale mais que conformidade com a spec quando as duas divergem. M6 e M1 receberam o mesmo tratamento: medir, documentar, adiar com gatilho — em vez de otimizar por suspeita.

### 2026-07-29c — H5: calibração por-output no HAL (ADR-0019)

**Done.** O `Calibration{gamma,brightness}` declarado por nó no ADR-0018 passa a ter efeito real. [ADR-0019](./docs/adr/0019-calibracao-por-output-no-hal.md).

**O achado que corrigiu o próprio H5.** O H5 dizia "gamma/brightness estão no lugar errado (globais no engine)". Verificando: (a) **`Gamma` era código morto** — LUT de 256 entradas em `color.rs:35` sem nenhum consumidor de produção, nem reexportada; (b) **`color::scale` não é calibração** — seus usos (`reactive.rs:142,189`) são **intensidade de efeito** (energia de áudio, decay do flash), corretos onde estão e **não movidos**. O problema real não era mover nada: era um campo declarado que ninguém honrava.

**Colocação:** no **HAL, por device, entre o `apply` e o fan-out** — o `scratch` de cada device já é contíguo (`device_range`). Assim **nenhum contrato Frozen muda** (`led-core` intocado, zero bump), a ramificação é **por device e não por pixel**, o custo é **zero** sem calibração, e o `led-hal` recebe `f32` — **não** ganha dependência de `led-hardware-profile` (o app é quem cabla).

**Bug pego antes do commit:** calibrar o `scratch` in-place causaria **escurecimento cumulativo** — `CompiledLayout::apply` só escreve os alvos cobertos por `frame.pixels` (guarda `.get(id)` para frames curtos), então alvos não cobertos seriam re-corrigidos a cada frame. Solução: **buffer calibrado separado**, dimensionado no startup. Provado por `calibration_does_not_compound_across_frames_on_a_short_frame` (50 frames curtos, gamma 2.2).

**Hazard no gate `no_alloc`:** o gate acusou 7 alocações. Em vez de afrouxar, diagnóstico (10k vs +20k frames): **0 e 0** rodando isolado → o contador é `static` **global do processo** e o `cargo test` roda em threads paralelas, contaminando a janela de medição. Corrigido com `ALLOC_GATE` serializando os testes do arquivo; o hazard ficou documentado.

**Invariants verified.** `no_alloc` verde **com calibração ativa** (zero alocação no hot path). SemVer Guardian: "superfície de seam inalterada (v1.3.0, 61 itens)". Efeitos reativos intocados. **Custo medido, não estimado** (6.200 px / 37 universos, debug): sem calibração 338.775 ns/frame, com 472.583 ns/frame → **+133.808 ns (×1,39), ~2,7% do orçamento de 5 ms**; release é substancialmente menor.

**Pending.** Ligar automaticamente `HardwareProfile.calibration` → `Hal::with_calibration` no app (hoje é manual, por design — o HAL não depende do profile). Calibração é por **device**, não por strip — um nó com chips diferentes precisaria de granularidade maior. White balance / temperatura de cor fora de escopo.

**Decisions.** Gamma e brightness são **dobrados numa única LUT** de 256 entradas no startup: o hot path faz uma leitura indexada por canal, sem `powf` nem ponto flutuante. Calibração é correção óptica, **nunca proteção elétrica** (isso é a fonte e o ABL do controlador).

### 2026-07-29b — `HardwareProfile` completo (ADR-0018): slices 1–5, guardião, presets, E2E

**Done.** O ponto de expansão de hardware do LUMYX, em 5 slices, com [ADR-0018](./docs/adr/0018-hardwareprofile-capacidades-design-time.md) e arquitetura em [`docs/architecture/hardware-profile.md`](./docs/architecture/hardware-profile.md).

1. **ADR-0018 + `HardwareProfileGuardian`** (`6aeb9f0`): profile é **descritor declarativo de capacidades**, nunca enum de produtos. `OutputInterface` **declara**, `DeviceDriver` **executa** (nome escolhido em vez de `Connection` porque `DeviceStatus.connected` já é conectividade de runtime). `RuntimeState` fica **fora** (reusa `DeviceStatus` + `led-readmodel`). `HardwareRegistry` separa registro de descrição. `schema_version` + `firmware_version`. Guardião com **8 checks executáveis**.
2. **Slice 1 — schema** (`65e604b`): crate leaf `led-hardware-profile` (dep única: `led-core`). `Identity`/`Capabilities`/`Limits`/`Power`/`Calibration`. `Capabilities` só declarativo/booleano; limites de pixel **só** em `Limits`. Cor reusa `ColorFormat`/`WhiteMode` (ADR-0011) — nenhuma segunda representação de RGBW.
3. **Slice 2 — validador** (`a203f7c`): `validate(profile, &Available{interfaces, protocols})`. **Injeção de dados**: detectar "driver inexistente" exigiria conhecer os drivers, o que quebraria o leaf — quem os conhece passa a lista como dado. Erros: schema desconhecida, interface/protocolo sem driver, pixels que não cabem no universo (usa `led_core::UNIVERSE_SIZE`, não 510 hardcoded), limites zerados, `Power`/`Calibration` inválidos (inclui `NaN`). Avisos: **RGBW sobre DDP** (o cabeçalho DDP fixa data type RGB8) e **WiFi** (ADR-0005 — o validador declara, o `NetworkGuard` bloqueia). Achados **acumulam**.
4. **Slices 3+4 — presets + registry** (`32f6b38`): `presets.rs` é uma **tabela**, `const PRESETS: &[PresetRow]`, com **zero `fn`/`impl`/ramificação** — como todo campo é literal ou variante, é `const` genuína. ESP32, ESP32-POE, Falcon, Advatek, Raspberry Pi, SK6812-RGBW e Custom são **linhas**, nunca variantes. Conversão linha→profile vive no `registry` (por isso a tabela fica sem `fn`). Landaram juntas porque o check 6 do guardião bloqueia preset sem teste de validação. Testes afirmam que os **avisos são exatamente os que os ADRs preveem**.
5. **Slice 5 — compilação** (`e40cb24`): `compile_layout` + `driver_config`. **Não precisou de crate novo nem tocar o `led-hal`**: `CompiledLayout` está no `led-core` e a cadeia termina em *Driver Configuration* (dado), não em *Driver* (I/O) — o crate segue leaf e o profile nunca chega ao caminho de render. Honra o `pixels_per_universe` **declarado** mesmo abaixo do máximo teórico. **Recusa** em vez de mapear errado.
6. **E2E + docs** (este commit): `integration-tests/tests/hardware_profile_e2e.rs` percorre **preset → validate → CompiledLayout → DriverConfig → Hal → SimulatorDevice** e verifica os **bytes** no dispositivo (ordem GRB do preset; 4 canais do preset RGBW com branco derivado no mapper; empacotamento declarado sobrevivendo até o fio) + controle negativo (sem driver, o fluxo para na validação). Vive no `integration-tests` porque o `SimulatorDevice` está no `led-hal` — assim o E2E existe sem o profile ganhar dependência de HAL.

**Invariants verified.** `led-core` **intocado** — confirmado pelo próprio SemVer Guardian ("superfície de seam inalterada, v1.3.0, 61 itens"). `cargo tree`: o crate depende só de `led-core`. Profile **ausente** de `led-hal` e do engine (guardião check 8). Zero dependência nova no workspace. Gates completos com `--locked` verdes a cada slice. Quando `CompiledLayout` (Frozen, sem `Debug`) impediu `unwrap_err()` nos testes, a saída foi casar o padrão — **não** adicionar `Debug` ao seam.

**Pending.** Drivers **SPI/PWM/ESP-NOW** (declaráveis, sem implementação — ADR próprio). Mover `gamma`/`brightness` do engine para por-output (achado H5). RGB+CCT / 5 canais. `profile_version` separado de `schema_version` — adiado por falta de consumidor (decidir se o catálogo precisar de revisão própria). Spike da ADR-0016 aguarda medição humana (a11y/GPU/DX). ADR-0017 (blackout) adiado.

**Decisions.** Preset é um **tipo** de hardware; `device_id`/`address`/`first_universe` são da **instância** — por isso `Identity` não tem endereço. Presets são dado e **novo hardware é uma linha**; `DeviceDriver` novo só quando houver transporte físico novo de verdade. Os números dos presets são pontos de partida por família, **não medições** — ajustar por instalação.

### 2026-07-29 — Fundação de UI: ADRs 0013–0017, CI do zero, `led-readmodel`, spike de stack, M1 medido

**Done.**
1. **ADRs 0013–0017** (`docs/adr/`, commit `4337cb7`): 0013 engine headless em daemon separado + UI cliente (output não compartilha processo de falha); 0014 IPC/segurança (UDS owner-only same-host, token/mTLS por interface na LAN, **nunca 0.0.0.0**, comandos tipados/versionados); 0015 preview por cópia downsampled/rate-limited/**lossy fora do hot-path** (UI nunca lê o triple buffer nem faz backpressure); 0016 stack do console **PROVISÓRIO** (web DOM+WebGPU, Leptos preferido) pendente de spike; **0017 blackout intencional × invariante do heartbeat — ADIADO** (nenhum botão/atalho/API de blackout antes deste ADR ser aceito).
2. **CI criada do zero** (`.github/workflows/ci.yml`, `91a92e3`): build+test+`clippy -D warnings` em **Linux e macOS (bloqueantes)**, Windows `continue-on-error` (suporta, não conduz — ADR-0013). Linux precisa de `libasound2-dev` (cpal→ALSA); roda **sem** a feature `gpu`. **Primeira CI verde do repo** após `35e77d7`.
3. **`sacn_multicast` na CI** (`35e77d7`): o teste estourava com `No route to host` nos runners (sem rota de multicast). O teste já pulava em falha de bind/join/recv; estendido para pular também na falha de **send**. Não era bug de código.
4. **`led-readmodel`** (crate leaf novo, `a3a8852` + `9f8a14c` + `f9274c3`): `ReadModel` read-only que a UI vai pollar — `DeviceStatus` + `HealthStatus` + contadores do `MetricsEmitter` + `DiscoveryResult`; `to_json()` hand-rolled (std-only, convenção do workspace — **sem serde**); `serve_readmodel` **loopback-only** (recusa bind não-loopback: `/security`); `ReadModel::assemble(...)` monta o snapshot das **fontes reais** do engine. Ausências são honestas (`devices:[]`, `discovery:null`), nunca fabricadas. `MetricsView` carrega só o que o emitter expõe publicamente (frames/drops/beats/p50/p99).
5. **Spike de stack** (`spike/`, `5450cfc`): protótipos descartáveis React/Vite e Leptos/WASM com a mesma tela acessível + preview 10k pts, `exclude`ídos do workspace/CI. React builda (vite 1.64s, 47 KB gzip); Leptos exige `trunk`+target wasm (não builda no ambiente do agente). `spike/README.md` traz o checklist de medição da ADR-0016 — **a11y/fps de GPU/DX são medidos pelo humano, não carimbados pelo agente**.
6. **M1 medido → TD-011 `wontfix`** (`8b1f217`): bench de medição `led-hal/tests/bench_contention.rs` (`#[ignore]`, **zero mudança de produção**) quantifica a contenção do `Mutex` de scratch em `send_frame`. 100k iters/300px/SimulatorDevice/dev macOS: render sozinho p50 20.651 ns / p99 69.678 ns; render+contender p50 23.558 ns / p99 1.419.228 ns → **x1,14 / x20,37**. **RESOLVIDO, otimização adiada**: pior caso 1,42 ms < 5 ms de orçamento; a cauda foi medida com contender em loop apertado, não a cadência real do heartbeat (~1 Hz vs ~44 Hz do render). Gatilhos de revisita em `docs/technical-debt-ledger.md` TD-011.

**Invariants verified.** Nenhum contrato canônico/seam Frozen alterado; hot-path render/send intocado (o read-model é management-plane read-only; o bench só exercita a API pública). Gate workspace **verde via CI** em `f9274c3` (Linux+macOS). `clippy -D warnings` = 0. Bench é `#[ignore]` (não pesa na CI: `1 ignored`, 0.00s). `/security`: read-model recusa bind não-loopback.

**Pending.** **Spike da ADR-0016 aguarda medição humana** (a11y/GPU/DX) → decidir Leptos vs React → promover 0016 → desbloqueia PR-05 (scaffold da shell). **ADR-0017 (blackout) adiado** — bloqueia qualquer blackout na UI. Windows segue vermelho na CI (não-bloqueante, não é alvo atual). Achados abertos da FASE 1: M2 (skill-Constituição sem camada HAL), M5 (PixelLogical não é seam — decisão de contrato), M6 (compile O(n²) não medido), L1 (deprecar `verify_manifest`), RGBW pixel-nativo no DDP.

**Decisions.** UI = **processo separado** (isolamento de falha é a única garantia real de que a UI não derruba o output). Read-model **hand-rolled sem serde** (o workspace já emite JSON à mão; evita dependência durável). Preview **lossy por contrato** — regra de isolamento, não otimização. Spike **excluído do workspace** para nunca entrar no baseline/CI. M1 **medido antes de otimizar** — e a medição disse para não otimizar.

### 2026-07-26 — Evolução pós-FASE 1: gate de clippy verde + M4 (router status real)

**Done.**
1. **Gate de clippy destravado** (Gate 2 do GOSL): `cargo clippy --workspace --all-targets -- -D warnings` → **exit 0**. Corrigido 1 erro deny (`led-protocols/src/artnet.rs` `*seq >= 255` → `== 255`, `absurd_extreme_comparisons` — lógica idêntica, wrap 1..=255 skip-0) + ~13 warnings de estilo em 15 arquivos (`checked_div`, `needless_range_loop`, `type` alias p/ `type_complexity`, const-assert, doc-comments `//!`, `unused_mut`, `redundant_pattern_matching`). Era **drift de toolchain** (rust-1.96.0): o "clippy=0" anterior foi registrado com um clippy mais antigo.
2. **M4** (`led-protocols/src/router.rs`): `RouterDevice::status()` reportava `frames_sent:0` hardcoded. Agora conta via `AtomicU64` incrementado em `send_physical` (+ teste `router_status_reports_real_frames_sent`).

**Invariants verified.** Sem mudança de comportamento: crates afetados **416/0**, led-protocols **75/0** (incl. teste M4). Gate de clippy **exit 0**. Seams Frozen intactos (só estilo + um `AtomicU64` de observabilidade no Router, fora do hot-path de serialização).

**Pending.** M5 (PixelLogical não-seam) — decisão de contrato, adiada p/ ADR próprio. Demais achados da FASE 1: M1 (Mutex hot-path HAL), M2 (skill-Constituição sem HAL), M6 (compile O(n²)), L1 (deprecar `verify_manifest`).

**Decisions.** Clippy tratado como cleanup mecânico, zero impacto de runtime — o erro deny era benigno (`>=255` num `u8` é sempre `==255`). M4 usa `AtomicU64`/`Relaxed` (status é lido off-path).

### 2026-07-25 — Revisão Constitucional HardwareProfile: C2 (alloc DDP) + H4 (fantasmas GOSL) + C1/RGBW (`ColorFormat`, ADR-0011)

**Done.**
1. **C2 — alloc no hot-path do `DdpBackend`** (`led-protocols/src/router.rs`): reconstruía `Vec<PixelColor>` + `Vec<u8>` por universo/frame (sob o fan-out do HAL). Corrigido: buffer `Box<[u8; 10+DDP_MAX_PAYLOAD]>` pré-alocado atrás do `Mutex` + novo `ddp::build_ddp_packet_bytes` (memcpy do payload já mapeado). Novo `led-protocols/tests/no_alloc.rs`: **0 alocações em 10k frames DDP**. `SacnBackend` (stack) e `DdpDevice` (já pré-alocado) estavam corretos.
2. **H4 — contratos-fantasma** (`LUMYX_GOSL.md`): Gate 4 e `/seam` citavam `LayoutIntent`/`SharedContext` como seams §3; **não existem no código**. Removidos.
3. **C1/RGBW — `ColorFormat` (ADR-0011)**: `led-core` ganha `ColorFormat{Rgb,Rgbw}` + `WhiteMode{None,Min}`; `PixelPhysical.order: RgbOrder` → `format: ColorFormat`; `CompiledLayout::apply` escreve `format.channels()` bytes; novo `linear_format` **aditivo** — `linear` (Frozen) intacto, delegando. Branco derivado da cor lógica RGB em `apply` (fronteira L↔P; espaço lógico segue RGB).
4. **DDP 4-canais** (`led-protocols/src/router.rs`): `DdpBackend` ganha `stride` (canais/pixel); `new` = RGB (3), `with_channels(dest, offset, 4)` = RGBW. Trunca padding DMX e fragmenta em fronteira de pixel por `stride` (o antigo `% 3` corromperia RGBW por meio pixel). Teste `ddp_backend_rgbw_sends_whole_4byte_pixels`. **C1 fecha ponta-a-ponta**: RGBW por sACN/Art-Net (byte-transparentes) e DDP (stride-aware).
5. **H1/H2 → ADR-0012 (decisão, impl. adiada)**: FASE 2 revisou a própria conclusão — os "3 serializadores sACN" chamam o mesmo `packet::build_data_packet` (serialização já é fonte única); são 3 *modelos de entrega*. **H1 rebaixado a LOW**. **H2** (fan-out sequencial do HAL, `hal.rs:115`) confirmado mas **adiado até 2º nó físico** (precedente ADR-0010, anti-overengineering a 1 nó).

**Invariants verified.** `PixelColor` lógico intocado (L↔P preservado). Assinaturas Frozen (UniverseData/CompiledLayout/DeviceDriver/ProtocolOutput/IDevice) inalteradas. **SemVer-guardian: superfície mudou COM bump (led-core 1.2.0→1.3.0) — PASS**; DAG acíclico. Prova de bytes RGBW (`mapping.rs` tests): GRBW = `[g,r,b,min(r,g,b)]`/pixel; RGB byte-idêntico ao anterior; `linear == linear_format(Rgb)`. Crates afetados: **183 testes verdes** (led-core/layout/xlights/player/hal). led-protocols 74+14 verdes (incl. DDP RGBW stride). `cargo build --workspace --all-targets`: **0 warnings**.

**Pending.** Fan-out paralelo do HAL (H2) **adiado até 2º nó físico** (ADR-0012). Resíduo LOW: CID/priority hardcoded no `SacnBackend`. Caminho DDP **pixel-nativo** (`DdpDevice`/player) para RGBW pende do player construir frames RGBW (o caminho universo→`DdpBackend` já cobre). Alinhar a skill Constituição (§1/§6 sem camada HAL) — arquivo de plugin fora do repo.

**Decisions.** RGBW resolvido como formato **por-pixel no mapper** (ADR-0011), sem tocar seam Frozen — aditivo, MINOR bump. `WhiteMode::Min` (W=min(r,g,b)) como derivação padrão; `None` para strip usada como RGB puro. `ColorFormat` é **Evolving** (RGB+CCT futuro). Unificação de saída documentada e **adiada** (ADR-0012) em vez de re-arquitetar o hot-path a 1 nó — mesma disciplina do ADR-0010.

### 2026-07-23 — Production Certification (WiFi): Art-Net validado em HW, sACN causa-raiz BLOQUEADO (firmware), burn-in WiFi, fase ESP32 DevKit V1 ENCERRADA

**Done (MODO EXECUTION — só validação, nenhuma feature; único arquivo tocado: `crates/led-player/examples/sacn_send.rs`, runner de teste parametrizado universo + `--multicast`).**

1. **Art-Net → VALIDADO EM HARDWARE.** `led-player striptest.lumyx --artnet 192.168.2.156 --first-universe 0` (universo 0 alinha com WLED `dmx.uni:0`; `1` desloca ~170px). played **94/0** em 3 runs; WLED `/json/info` reporta `lm:"Art-Net"`, `lip:192.168.2.32`, `live:true` ~50/56; **visual R→G→B→cometa confirmado pelo operador**. O `lm` lido do WLED é evidência de aceitação mais forte que tcpdump.

2. **sACN → BLOQUEADO (lado WLED, não LUMYX).** Investigação completa sem assumir culpa: (a) pacote E1.31 do LUMYX dissecado byte-a-byte por loopback — **todos os 14 campos corretos** (PID `ASC-E1.17`, vectors root/framing/DMP, PDU flags+len, start code, count 513); (b) **sender E1.31 de referência independente** (Python, byte-perfeito) falha igual — `live:true` 0/26 em unicast uni0/uni1 + multicast; (c) **probe de porta UDP:** `:5568` devolve **ICMP port-unreachable idêntico a porta não-usada**, enquanto `:6454` e `:4048` têm listener. **Causa raiz objetiva: WLED 16.0.1 não faz bind do receiver E1.31 na 5568.** LUMYX exonerado; `packet.rs` provado correto por 2 caminhos independentes.

3. **Burn-in WiFi (DDP, striptest, monitor de saúde 5s):** 45 passes limpos (94/0), **abort no pass 46 por 1 falha de `sendto`** (provável ENOBUFS WiFi). ESP32 **impecável**: 0 reset (uptime 2616→3044s monotônico), 0 leak (freeheap ±60B), rssi −40/−51. Falha é do **transporte WiFi**, não do ESP32 nem do player.

4. **Sweep de throughput do sender (DDP):** speed 1→max = 11/21/40/76/**1593 fps**, **0 falhas em todos** (played 282/282). Sender não é o gargalo. Ressalva de honestidade: fire-and-forget sem ACK — `played` mede só sucesso do `sendto`, **não** exibição no WLED; teto de entrega real não é mensurável sem instrumentar o firmware.

5. **Auditoria de HW:** `hw.led.ins` = strip[0] pin2/720px/GRB/start0 (real) + strip[1] pin17/380px + strip[2] pin18/460px (config-fantasma sem HW) → count 1560 vs 720 físico. `dmx.mode:4` (Multi RGB), `live.en:true`, unicast. ABL clampando (pwr 1680mA vs maxpwr 850mA). `wifi.sleep:true`.

**Invariants verified.** Nenhum código de produto alterado (só um example de teste). Replay determinístico: hash `0x23b8ee876a18e5a5` idêntico em todos os runs (Art-Net, sACN, DDP, burn-in). Regra WiFi-proibido reconfirmada empiricamente (1 transiente de envio/~6min + jitter documentado).

**Pending (dependem OBRIGATORIAMENTE de Ethernet — próxima fase).** Confiabilidade de entrega contínua; burn-in 72h sem abort de transporte; latência/jitter sub-ms de entrega real. **Exceção:** sACN em HW é bloqueio de *firmware* (porta 5568), não de meio físico — destrava só reflashando/reconfigurando o receiver E1.31 do WLED.

**Decisions.** Fase "ESP32 DevKit V1 + WLED + WiFi + 720 LEDs" **formalmente encerrada**: toda validação técnica não-redundante possível sobre WiFi foi extraída. `live:true`+`lm` do `/json/info` do WLED adotado como evidência de aceitação (mais forte que tcpdump). Probe ICMP port-unreachable adotado como prova de listener ausente. tcpdump não rodado (requer sudo/senha — ação do operador).

### 2026-07-20 — PRIMEIRA LUZ FÍSICA: validação de bancada 1 nó (ETAPAS 1–10 PASS), striptest, certificado

**Done.** O rig deixou de estar 100% offline. 1 controlador de bancada energizado e validado ponta-a-ponta pela skill "LUMYX Live Hardware Validator" (10 etapas, uma por vez, só avança com evidência observada). Hardware: ESP32/WLED **16.0.1**, fita WS2812B **720px**, GPIO2/GRB, bateria 12V → DC/DC TOBSUN 5V/10A, cap 1000µF + resistor 330Ω, ABL 850mA, **IP 192.168.2.156** (= robô led 1).

1. **PRIMEIRA LUZ LUMYX (ETAPA 7)** — `led-player striptest.lumyx --ddp 192.168.2.156`: **94/94 frames, 0 falhas**, hash `0x23b8ee876a18e5a5`; WLED `live:true` de 192.168.2.32; visual R→G→B→cometa confirmado pelo operador. Primeira vez que o pipeline software→fio→pixel real fecha em hardware (antes tudo era ⚠ NÃO VALIDADO).

2. **`make_striptest.rs`** (`crates/led-show-recorder/examples/`) — gera um `.lumyx` de bring-up: sólidos R/G/B + cometa branco (caça pixel morto), valor 64 (corrente baixa, ABL de backup). 720px/94 frames. Build warning-free.

3. **Metrics ao vivo (ETAPA 8)** — `/metrics` durante DDP real: frames_total 1→222, **0 drops**, p50 128µs/p99 8.2ms (latência de ENVIO do player; entrega real é o ping, downstream).

4. **Mini burn-in (ETAPA 9)** — **74/74 passes DDP, 0 falhas, 0 aborts**, hash único; WLED pós-burn uptime 2388→3296s (**sem reset**), freeheap estável (**sem leak**), rssi −37.

5. **Relatório (ETAPA 10)** — `docs/certification/HARDWARE-VALIDATION-2026-07-20.md`: placar + evidência + escopo honesto do que NÃO está validado.

**Invariants verified.** Replay determinístico em **77 passes** contra hardware real (hash idêntico em todos). led-core e código-produto intocados (só um example novo + docs). ABL 850mA segura a corrente da fita enrolada.

**Achado de rede (evidência real, não hipótese).** Ping do Mac ao WLED: 0% perda mas **99ms avg / 146ms pico / jitter 31ms** com RSSI −44 (sinal forte) → power-save do WiFi do ESP32, não sinal fraco. Confirma empiricamente [ADR-0005](./docs/adr/0005-wifi-proibido-producao.md) (WiFi proibido ao vivo), pior que a estimativa de 5–50ms.

**Pending.** Só 1 de 5 nós / 720 de 6.200px. Caminho **Ethernet cabeado** NÃO validado (bancada foi WiFi — e o WiFi provou o jitter). Show musical real (foi striptest sintético, não `robot_sequence.lumyx`). Burn-in 72h; chaos físico; multi-controlador DDP em hardware.

**Decisions.** Validação de bancada sobre WiFi conta como prova do **pipeline** (software→DDP→WLED→LED), não do link de show ao vivo (esse exige Ethernet). DDP é fire-and-forget: 0 aborts no burn-in prova estabilidade do `led-player`, não continuidade do WLED (sem ACK — a continuidade sob jitter é observação visual).

### 2026-07-11 — Certification Program, rodada 1: burn-in 1h PASS (30/30), ledger de certificação, alertas Prometheus, probe de determinismo

**Done.**

1. **Burn-in 1h: PASS** — `burnin-20260711-083048.jsonl`: 30 passes consecutivos do `robot_sequence.lumyx` (show real, 6.200px), 0 aborts, hash estável em todos. Janela de 72h (software) na fila (classificador bloqueou o disparo — retentar).

2. **Ledger de Certificação** (`docs/certification/CERTIFICATION.md`): todos os critérios do programa com status + evidência verificável (regra: sem artefato = ⏳). Estado: Segurança/Observabilidade/Governança/Determinismo-referência ✅; hardware e Linux/Windows aguardando recurso externo.

3. **Alertas Prometheus** (`docs/observability/alerts.yml`): 5 regras espelhando os SLOs — fast/slow burn de entrega (multi-window), p99>5ms, show stalled, exporter down; montadas no compose (`rule_files`).

4. **Probe de determinismo** (`scripts/determinism_probe.sh`): 1 comando em qualquer plataforma (Linux/WSL/macOS) → artefato de evidência com veredicto MATCH/DIVERGENCE. Docker/colima indisponíveis nesta máquina — Linux fica como execução pendente com ferramenta pronta.

**Invariants verified.** Burn-in re-verifica hash por pass (30/30 idênticos). Ledger sem critério ✅ sem evidência.

**Pending.** Burn-in 72h (disparo bloqueado pelo classificador — retentar); rig offline (re-verificado 2026-07-11); probe Linux/Windows aguarda máquina; chaos físico literal.

**Decisions.** Burn-in de software conta como pré-gate; aprovação plena dos critérios de burn-in exige repetição contra hardware. ARM certificado via plataforma de referência (Apple Silicon = arm64).

### 2026-07-12b — Discovery pré-show: fecha o footgun de palco escuro (RT-003)

**Done.** Após a revisão suprema recomendar SIMPLIFICAR (focar no 1x real, não escala), o usuário escolheu discovery pré-show — a melhoria de ROI real no rig atual.

1. **`led-protocols` discovery** (network-architect): `presence(&[Ipv4Addr], &[ArtPollReply]) -> DiscoveryResult` (lógica pura, particiona esperados em responded/missing; reply de IP errado nunca mascara um ausente) + `discover_controllers(expected, timeout)` (ArtPoll broadcast + coleta, espelha `poll_conflicts`). 4 testes (14 no módulo artnet) incl. negativo `negative_control_rogue_reply_cannot_mask_a_missing_controller`.

2. **Player `--discover` / `--require-all`**: roda ArtPoll antes do 1º frame contra o alvo `--artnet`/`--ddp`. `--discover` avisa; `--require-all` aborta (exit 1) se algum controlador esperado silenciar. **Provado contra o rig real offline**: `⚠ SEM resposta` → `ABORT exit 1`. Opt-in — run de simulador inalterado.

3. **RT-003 → MITIGADO** (era PARCIAL): o footgun "controlador ausente = palco escuro sem erro" agora é pego antes do show. GAP residual menor: `--first-universe` numérico errado (baixo risco, rastreado).

**Invariants verified.** Cadeia Builder→Guardian: `lumyx_builder.sh check preshow-discovery` APROVADA, 0 regressões. Seam led-core intocado (SemVer 1.2.0). Feature aditiva (sem flag = comportamento idêntico).

**Decisions.** Discovery via ArtPoll (não DDP query) — um WLED no IP certo responde ArtPoll independente do protocolo de saída, cobrindo mais casos. Opt-in por flag (precisa de porta :6454 livre e pode não ter permissão em todo ambiente).

### 2026-07-12 — Red Team (4º time): achado CRITICAL de autenticidade de assinatura encontrado, provado e fechado end-to-end

**Done.**

1. **RT-001 (CRITICAL) — verificação de assinatura confiava na chave embutida**: o Red Team (security) auditou `signing.rs` e achou que `verify_manifest` valida a assinatura contra a pubkey do **próprio blob** — um atacante re-assina o tamper com a própria chave, embute a própria pubkey, e passa. Prova de exploit: `redteam_resigned_tamper_defeats_unpinned_verify` (passa, documentando o buraco).

2. **Correção — verificação com chave fixada**: `verify_manifest_pinned(signed, &trusted_key)` + variante `UntrustedKey` (rejeita se a chave embutida ≠ a chave pré-confiada, que viaja out-of-band). `verify_manifest` mantido com doc ⚠️ (uso local só). Ligado no consumidor real: `led-player --verify-key <hex>` carrega o sidecar `.sig`, confere que cobre o show e verifica com chave fixada. Produtor: `led-show-recorder --example sign_show`. **Provado e2e no CLI**: estúdio assina (pubkey `d04a…`) → palco verifica OK; atacante re-assina (pubkey `34b4…`) → palco verifica com chave do estúdio → `SIG VERIFY FAILED` exit 1. 12 testes de signing (incl. negativo `pinned_verify_rejects_resigned_tamper`).

3. **`scripts/lumyx_red_team.sh`** (harness do 4º time): 5 probes adversariais (security/reliability/architecture/product/chaos) que verificam que cada mitigação CRITICAL/HIGH não regride (ex.: pinned test passa + player ainda usa o caminho fixado). Ledger consolidado em `docs/red-team/findings.md` (RT-001…RT-005, estruturado + histórico de execuções). Exit 0 = 0 achados abertos.

4. **+6 agentes RED-TEAM** (`.claude/agents/`): lumyx-red-team + security/reliability/architecture/product/chaos-red-team (Sonnet). **Total: 27 agentes nos 4 times** (Builder/Validator/RedTeam=Sonnet, Guardian=Haiku — política de modelo do usuário respeitada).

5. **Ledger de certificação atualizado** (`docs/certification/CERTIFICATION.md`): RT-001 fechado; seção "o que falta" — 100% dos bloqueadores restantes são recurso externo (hardware/CI), não software.

**Invariants verified.** Guardian 0 regressões após a mudança de segurança (aditiva em led-show-recorder, seam led-core intocado → SemVer 1.2.0 preservado). Burn-in 24+ passes/0 aborts.

**Pending.** Hardware (rig offline). Burn-in 72h persistente (launchd, fora de sessão). Linux/Windows determinism probe. GAP menor de chaos: reorder/dup de pacote no fio (RT-004).

**Decisions.** `verify_manifest` não foi removido (compat + uso local legítimo) mas documentado como integridade-só; o caminho de fronteira de confiança é o pinned. Red team é adversarial por design: 0 achados = falha; o valor é o RT-001 real que foi encontrado, não os testes que passam.

### 2026-07-11b — Times completos Builder/Validator/Guardian + feature xlights-export pela cadeia inteira

**Done.**

1. **`led-xlights::export`** (feature construída VIA o fluxo builder, workbook em `docs/features/xlights-export.md`): `export_rgbeffects` (XML compatível, entity-escaped) + `rig_to_xmodels` (`RigPlan`→endereçamento xLights `!ctrl:abs`, grupos por instância). Migração agora é bidirecional: rig criado no LUMYX abre no xLights. 4 testes: roundtrip campo-a-campo, gate no próprio output ("nunca emitimos o que recusaríamos"), escaping (`R&B "x" <robô>`), **negative control** (export adulterado com canal duplicado É pego pelo gate). Dep nova justificada: led-xlights→led-layout (ponte bidirecional; sem ciclo, led-core intocado). Workspace: **647 testes**.

2. **`scripts/lumyx_builder.sh`** (harness do time de construção): `new <slug>` scaffolda workbook com as 6 seções obrigatórias (Motivação·Design·Implementação·Testes·Rollback·Evidência); `check <slug>` valida completude (seção vazia = bloqueio; "Teste negativo:" obrigatório — KB-012; evidência exige bloco de comando) e entrega ao Guardian. Demonstrado: `check xlights-export` → **APROVADA**.

3. **`scripts/lumyx_validator.sh`** (harness do time de validação, perfil Sonnet): 5 validadores com saída PASS/FAIL/Risco/Evidência — Test Architect (workspace+e2e), Chaos (udp_chaos+failover), Observability (**scrape AO VIVO durante playback real** + alerts.yml parse + dashboard JSON), Cluster (two_node 6/6 + net_time 5/5), Production (burn-in jsonl + ping hardware + binário release). SKIP ≠ FAIL: ausência de hardware é risco nomeado. Resultado: **PASS, 11/1 SKIP/0 FAIL, exit 0**. O validator pegou 2 bugs no próprio harness na 1ª rodada (server de métricas sobe após o hash do show em debug → retry; `grep -c || echo 0` imprime duplo zero) — corrigidos.

4. **+6 agentes VALIDATOR** (`.claude/agents/`): lumyx-validator + test-architect, chaos-engineer, observability-engineer, cluster-engineer, production-engineer (Sonnet). Total: **21 agentes** nos 3 times.

5. **Burn-in em curso**: `burnin-20260711-194408.jsonl` acumulando (15+ passes, 0 aborts na última checagem).

**Invariants verified.** Cadeia inteira verde na mesma feature: BUILDER check → VALIDATOR PASS → GUARDIAN 0 regressões. 647 testes.

**Pending.** Hardware (SKIP nomeado no validator). Linux/Windows determinism probe. Burn-in 72h persistente (launchd).

**Decisions.** Validador ao vivo > validador de unit-test (o scrape mid-show é obrigatório, não opcional). Exit code lido SEM pipe (KB-013 — o próprio invocador tinha violado).

### 2026-07-11 — Certificação: times de agentes Builder/Guardian + gate mecânico anti-regressão, alertas Prometheus, ledger de certificação, burn-in 72h

**Done.**

1. **LUMYX-GUARDIAN mecânico** (`scripts/lumyx_guardian.sh`): 6 guardiões em ~8,6s (perfil Haiku — rápido, roda em toda alteração antes do e2e pesado). SemVer (snapshot da superfície pública de led-core vs baseline commitado `.lumyx-guardian/led-core-api.txt`; diff sem bump de `LED_CORE_CONTRACT_VERSION` = BLOCK), Dependency (DAG acíclico + C1 + led-core sink), Replay (vetores determinismo + ReplayManifest), Performance (bench_latency), Security (cargo audit + Ed25519), Governance (audit_gate + C10). **Negative control provado**: injetar item de seam sem bump → o guardião bloqueia. **2 bugs de shell corrigidos no processo**: versão saía "v-1" porque `grep -oE` pegava lixo → `sed -E`; e a função `head()` sombreava o comando `head` (KB-013 família — funções de shell não podem sombrear coreutils).

2. **15 definições de subagente** (`.claude/agents/`): LUMYX-BUILDER (Sonnet, criar recursos) + 7 architects (rust/dsp/network/realtime/product/drone/security), cada um com a saída obrigatória Motivação·Design·Implementação·Testes(negativo)·Rollback·Evidência; LUMYX-GUARDIAN (Haiku, impedir regressões) + 6 guardians que espelham o gate mecânico. Frontmatter validado (name/model/description, modelos corretos por time).

3. **Alertas Prometheus** (`docs/observability/alerts.yml`): 5 regras espelhando os SLOs (fast/slow burn de entrega, p99>5ms, show stalled, exporter down) montadas no compose. Fecha "Alertas" da certificação.

4. **Ledger de certificação** (`docs/certification/CERTIFICATION.md`): tabela evidência-por-critério do LUMYX Production Certification Program. Certificado hoje: Segurança (cosign+SBOM+attest+Ed25519), Observabilidade, Governança, Determinismo (referência arm64), Confiabilidade lógica. Aguardando recurso: hardware, Linux/Windows (probe `determinism_probe.sh` de 1 comando pronto), chaos físico literal.

5. **Burn-in**: 1h software concluído (30 passes, 0 aborts) → 72h iniciado (`robot_sequence.lumyx`, 6.200px).

**Invariants verified.** Guardian exit 0 (12 checks); led-core 17 provenance tests; baseline SemVer commitado (não ignorado — corrigido). Nenhuma mudança de código-produto (só scripts + docs + agentes + baseline).

**Pending.** Hardware físico (rig offline). Linux/Windows determinism (sem Docker nesta máquina — probe pronto). Burn-in 72h/168h em curso/pendente. Chaos físico literal (cabo).

**Decisions.** Guardian é separado do e2e (rápido/toda-alteração vs pesado/release). Baseline SemVer é COMMITADO (contrato, não estado efêmero). Builders = Sonnet (raciocínio de construção), Guardians = Haiku (verificação repetitiva barata) conforme especificado.

### 2026-07-09f — Missão Production-Ready, rodada 5: cosign fechado, E2E 15 fases 708/0, KB-013/KB-014 (2 classes de falha de gate encontradas e corrigidas)

**Done.**

1. **Cosign integrado (Segurança 100%)**: cosign v3.1.1 instalado; `release_sign.sh` gera chave local não-interativa (`COSIGN_PASSWORD`), assina binários (`*.cosign.bundle`), atesta o SBOM CycloneDX (`*.sbom.bundle`) e **verifica no próprio pipeline**. Executado: 2 binários assinados+atestados+verificados + sidecars Ed25519.

2. **E2E completo executado e consertado — 708 testes, 0 falhas, exit 0** (`~/lumyx-e2e.sh`, 15 fases incl. projeto real). No caminho, 3 bugs de infraestrutura de gate encontrados por rodar a coisa de verdade:
   - **KB-013a**: `set -e` matava o script na 1ª falha (contrato é contar e reportar) e o pipe do chamador mascarava o exit → falso-verde. Fix: `set -u`, morte proibida.
   - **KB-013b**: `pipefail` + `grep -q` → SIGPIPE (141) no cargo após o match → **16 gates falso-vermelhos**. Fix: pipefail removido (nenhum pipeline o exigia).
   - **KB-013c**: `grep -P` não existe no BSD grep do macOS (C6/C11 nunca extraíam) → `sed -E` portável.
   - **KB-014**: `"hash":0x...` (hex sem aspas) = JSON inválido em **3 emissores** (`ReplayManifest::to_json`, `ShowInfo::to_json`, `Provenance::to_json`) — pego pelo gate P10d que faz `json.load` real. Fix: hex sempre string; teste endurecido para `"hash":"0x`.
   - Gates P13/P13b tinham contagens exatas hardcoded (18/6) que quebravam ao adicionar testes → verificação robusta (ok + sem FAILED).

3. **Burn-in 1h** (simulador, show real `robot_sequence.lumyx`) rodando em background via `nohup` — evidência de estabilidade pré-hardware.

**Invariants verified.** E2E 15/15 fases PASS; workspace 643 + drone 65 = 708 verdes; JSON de todos os emissores parseável; assinaturas cosign verificadas.

**Pending.** Hardware físico (5 IPs offline re-verificados). Resultado do burn-in 1h (em execução). Miri 8-thread (inalterado).

**Decisions.** Chave cosign local em `release/` (gitignored) até existir CI com OIDC. KB-013/KB-014 registrados como permanentes no knowledge-base.

### 2026-07-09e — Missão Production-Ready, rodada 4: sequência criativa do usuário no LUMYX, capacidade 248k px provada, grupos, .xsq import, métricas no player

**Done.**

1. **Sequência real do usuário tocada pelo LUMYX** (`led-demo/examples/robot_sequence.rs`): `parse_sequence` leu `__.xsq` (15 spans: Lightning→Life→Meteors cascateando r1→r5 + finale em `robôs T`), `pixels_for_group` resolveu grupos aninhados → 3.925 frames × 6.200 px renderizados com efeitos mapeados (Lightning→Pulse branco 6Hz, Life→Plasma, Meteors→Rainbow) → `robot_sequence.lumyx` replay VERIFIED (`0xd8f1479ff3645e1e`) + GIF do trecho da cascata. Preview confirma targeting: em t=0 só o robô 1 acende (fidelidade ao design).

2. **Capacidade provada** (`capacity_bench.rs` + `docs/capacity.md`): pipeline completo em release — 6.200 px = 0,55ms/frame (45× folga); **248.000 px = 23,05ms — 40fps OK em CPU pura** (40× o rig atual). Gargalo é o transporte ESP32/WiFi, não o software; plano de capacidade documentado (DDP → ETH → RouterDevice multi-controlador).

3. **`led-xlights` +3 APIs**: `pixels_for_group` (grupos aninhados, guard de ciclo), `parse_sequence`/`parse_sequence_file` (.xsq → `EffectSpan{element,effect,start,end}` + media + duration; efeitos de timing-track excluídos), `examples/sequence_report.rs` (CLI). +4 testes (22 no crate).

4. **Player `--metrics PORT`**: `MetricsEmitter` + `serve_metrics` no produto; `play_instrumented` mede latência por frame e drops em QUALQUER saída (Hal, DDP). **Validado ao vivo**: scrape no meio do show real → p50=0,5ms, p99=4,1ms (dentro do SLO de 5ms em debug), 0 drops.

**Invariants verified.** Workspace verde; 0 warnings; replay das duas gravações do projeto real bate hash; targeting por grupo comprovado visualmente e por teste.

**Pending.** Hardware físico (inalterado). Fidelidade visual dos efeitos xLights (Meteors/Lightning/Life reais vs mapeados — falta motor de efeitos equivalente; timing/targeting já exatos). EffectDB settings não importados (slice futura).

**Decisions.** Mapeamento de efeitos é explícito e documentado no example (tabela) — honesto sobre o que é aproximação. GIF de excerpt (cascata 0–31s @10fps) em vez do show inteiro (tamanho).

### 2026-07-09d — Missão Production-Ready, rodada 3: projeto real de 5 robôs rodando no LUMYX end-to-end, DDP no player, posições de pixels do xLights

**Done.**

1. **Projeto real no LUMYX** (`led-demo/examples/robot_show.rs`): o show de 5 robôs do usuário (430 modelos, 6.200 px, 5 controladores WLED) importado do `xlights_rgbeffects.LUMYX-FIXED.xml` → gate 0 conflitos → `CompiledLayout` → Plasma renderizado nas **posições de mundo reais** dos robôs → Hal → 5 SimulatorDevices (240 frames cada) → `robot_show.lumyx` → **replay VERIFIED** (`0xda96e3fe9abe65f4`) → `robot_show.gif` (preview 2D: os 5 robôs humanoides visíveis e animados). Prova da migração xLights→LUMYX completa.

2. **`XModel::pixel_positions()`** (`led-xlights`): endpoints X2/Y2/Z2 importados; interpolação linear start→end por pixel (guard divisão-por-zero em strip de 1 px). Ordem das posições = ordem de `assignments()` — 1 posição por pixel físico. +1 teste.

3. **DDP no player**: `DdpOutput` (adapter `ProtocolOutput` pixel-nativo sobre `DdpDevice::send_pixels`, 487 px/datagrama ≈ 3× menos pacotes que ArtNet p/ WLED) + flag `--ddp IP[:4048]`. Teste: 600 px × 3 frames = exatamente 6 fragmentos DDP válidos no loopback.

4. **lumyx-e2e.sh Phase 15**: pipeline do projeto real como gate (skip gracioso se a pasta não existir).

**Invariants verified.** Workspace verde; 0 warnings; posição↔assignment 1:1 comprovada por assert no exemplo; replay do projeto real bate hash.

**Pending.** Hardware físico (rig offline). GIF preview usa projeção XY simples (Z ignorado — os robôs são planos). Modelo "cabeça doida" órfão aparece no canto do preview (herança do template original no layout do usuário — inofensivo).

**Decisions.** Preview 2D como example do led-demo (gif dep já existe lá). DDP bypassa o Hal (pixel-nativo, sem mapa de universos) — correto para WLED-single-target; multi-controlador DDP fica para RouterDevice.

### 2026-07-09c — Missão Production-Ready, rodada 2: chaos de rede real, net_time (PTP-style), burn-in, determinismo cross-platform, release signing

**Done.**

1. **`UdpChaosProxy`** (`integration-tests/src/lib.rs` + `tests/udp_chaos.rs`): proxy UDP com fault injection determinística (SplitMix64) entre sockets reais — o equivalente CI de puxar o cabo. 5 testes: baseline 100/100, 30% de perda no fio degrada sem parar o stream (sender segue mandando), outage total → heal → 100% de novo, latência injetada observável, mesmo seed = mesmos drops.

2. **`net_time`** (`led-hal/src/net_time.rs`): two-way time transfer (matemática NTP/PTP delay req-resp) — `TimeServer` no líder, `measure_offset`/`best_of` (gating por delay), `sync_to` calibra o `SharedClock` do follower. 5 testes: offset injetado ±500/−300ms medido a ±10ms; pós-sync dentro do budget de 5ms; robusto a pacotes malformados. + `docs/ptp-investigation.md` (conclusão: ±1ms por software basta para o alvo; PTP HW só se sub-ms for exigido — troca da fonte do clock, sem mudança de arquitetura).

3. **Burn-in**: `led-player --loop N` (0=infinito) re-verifica o hash do manifest a cada pass e aborta na primeira falha de integridade/entrega (o abort É o sinal). Smoke: 5 passes no show real, hash estável. `scripts/burnin.sh <horas> <show> [ip]` para as janelas de 72h/168h contra hardware.

4. **Vetores de determinismo** (`integration-tests/tests/determinism_vector.rs`): goldens gravados na plataforma de referência (macOS arm64) — intent_hash (só inteiros: OBRIGATÓRIO igual em toda plataforma) e Plasma render hash (f32 trig: instrumento de MEDIÇÃO de divergência libm cross-platform). 3 testes.

5. **Release signing** (`scripts/release_sign.sh` + `led-show-recorder/examples/sign_file.rs`): build release → SBOM → assinatura Ed25519 de cada artefato (sidecar .sig/.pub) → verify self-check. Rodado: 3 artefatos assinados e verificados. cosign plugável (install bloqueado pela rede: ghcr.io reset — script detecta e instrui). `release/` no .gitignore (seed privada nunca commitada).

6. **Grafana provisionado**: `docs/observability/docker-compose.yml` (Prometheus + Grafana com dashboard e datasource pré-provisionados, `docker compose up -d`).

7. **lumyx-e2e.sh Phase 14** (P14a–d): wire chaos, net_time, determinism vectors, release-signing check.

8. Rig físico verificado: 5 IPs offline no momento (192.168.2.156–160) — smoke de hardware permanece bloqueado só pela alimentação do rig.

**Invariants verified.** Workspace verde (ver contagem); 0 warnings; chaos determinístico por seed nos dois níveis (in-process e wire); líder nunca ajusta o próprio clock; goldens de determinismo pinados com data+plataforma.

**Pending.** Hardware real (rig desligado); burn-in 72h (harness pronto — é só rodar `scripts/burnin.sh 72 show.lumyx <ip>`); cosign binário (rede); chaos físico literal (runbook = puxar cabo com burn-in rodando); Grafana compose não executado (docker não testado aqui).

**Decisions.** Chaos proxy vive em `integration-tests` (infra de teste, não produção). `net_time` re-sincroniza por troca de música, nunca por frame. Golden de render f32 é instrumento de medição, não gate duro em plataformas não-referência.

### 2026-07-09b — Missão Production-Ready, rodada 1: importador xLights + auto-fix, RigBuilder, Show Player, Ed25519, Prometheus, SBOM

**Done.**

1. **`led-xlights`** (crate novo, leaf, std-only): parser XML mínimo (elementos+atributos+entities) + `parse_networks`/`parse_rgbeffects` + resolução `!controller:canal` → (universo, canal) + **gate de conflitos** (`assignments()` retorna `Err` enquanto houver overlap) + **auto-fix** (`propose_fix` reempacota contíguo preservando ordem; `apply_fixes_to_xml` reescreve só os StartChannel, byte-a-byte no resto). 18 testes. **Validado no projeto real do usuário**: 430 modelos, 5 controladores, 6.200 px, **2.701 conflitos detectados** → fix de 425 modelos verificado (0 conflitos) → `xlights_rgbeffects.LUMYX-FIXED.xml` (original intacto).

2. **`RigBuilder`** (`led-layout/src/rig.rs`): `RigTemplate`/`build_rig` — N instâncias de um template com endereçamento **livre de conflito por construção** (1 device/instância, universos de 1, 170px/universo sem straddle, `verify_no_overlap` prova). 7 testes incl. escala real (86 strips × 5 robôs = 8.600 px).

3. **`led-player`** (crate novo, bin+lib): reproduz `.lumyx` para qualquer `ProtocolOutput` — pacing pelos timestamps gravados (`Speed::Factor`/`Max`), `--info` (timeline: frames/px/duração/beats/hash), `--verify <hash>` (gate de integridade, exit 1 em mismatch), `--artnet IP --first-universe N` (hardware real). `linear_assignments` (universos de 1, estilo WLED). 6 testes. Validado no `show.lumyx` real (120 frames, hash OK, verify bom/ruim → exit 0/1).

4. **Ed25519** (`led-show-recorder/src/signing.rs`, dep `ed25519-dalek` justificada): `ShowSigner` (seed 32B ou `/dev/urandom`), assinatura de `ReplayManifest` (bytes canônicos versionados) e de snapshots, sidecar `.sig` roundtrip. Determinístico (mesma chave+manifest ⇒ mesma assinatura). 9 testes (tamper agg/frame-hash, wrong key, malformed).

5. **Prometheus** (`led-hal/src/prometheus.rs`): `prometheus_text()` (exposition 0.0.4: `lumyx_frames_total`, `lumyx_drops_total`, `lumyx_beats_total`, `lumyx_frame_latency_seconds` quantiles) + `serve_metrics()` (HTTP std-only, GET /metrics, 404 no resto). 4 testes incl. endpoint loopback. **BUG corrigido no histogram** (`metrics.rs::percentile`): target=0 com poucas amostras casava no bucket 0 vazio → p50 de 1 amostra de 1s reportava 1µs; fix `ceil().max(1)` (classe KB-012).

6. **SBOM + supply chain**: `scripts/generate_sbom.py` (CycloneDX 1.5 de `cargo metadata --locked`; 158 componentes, purl+licença) → `docs/sbom/sbom.cdx.json`; `docs/supply-chain.md` (política, gates, ameaças, próximo passo cosign).

7. **Observabilidade formal**: `docs/observability/SLO.md` (4 SLOs + error budgets + burn-rate queries) e `grafana-lumyx.json` (dashboard 5 painéis).

8. **lumyx-e2e.sh Phase 13** (P13a–f): importer, player, ArtNet, signing, Prometheus, RigBuilder como gates.

**Invariants verified.** Workspace completo verde (ver contagem no status); 0 warnings; gate de import é bloqueante (Err, não warning); assinatura determinística preserva reprodutibilidade; scrape nunca toca hot path.

**Pending.** Hardware real (smoke ArtNet no WLED da bancada — precisa do rig ligado); burn-in 72h; cosign nos releases; PTP investigação; chaos físico; Grafana provisionado num Prometheus real; link z.ai (navegador desconectado).

**Decisions.** `led-xlights` std-only com parser próprio (XML é machine-generated; evita dep). Auto-fix nunca sobrescreve o original. `ed25519-dalek` sem feature de RNG (seed via SO). Player carrega o show inteiro em memória (shows são minutos; permite verificar manifest antes do 1º frame).

### 2026-07-09 — ArtNet ArtDmx output + debug-aware perf budgets + estudo do rig real (5 robôs WLED)

**Done.**

1. **`ArtNetDevice`** (`led-protocols/src/artnet.rs`): saída ArtDmx real — `build_art_dmx`/`parse_art_dmx` (OpCode 0x5000 LE, ProtVer 14, SubUni/Net, length big-endian par), `ArtNetDevice: DeviceDriver` unicast com sequência **por universo** wrapping 1..=255 (0 nunca emitido — desabilitaria checagem no receptor), buffer pré-alocado (zero alloc hot path), payload ímpar padded. 7 testes novos (wire offsets exatos, padding, roundtrip 512 slots, garbage rejection, loopback UDP 2 universos, wrap per-universe 300 frames, oversize refused). Motivação: o rig real do usuário (5 robôs, WLED ESP32, ArtNet, universos 1–149) não podia ser acionado — LUMYX só tinha ArtPoll.

2. **Perf budgets debug-aware** (`led-bridge/tests/e2e_pipeline.rs`, `audio-core/src/harmonics.rs`): 3 testes de latência flaky em debug sob carga paralela → budgets condicionais `cfg!(debug_assertions)` (release mantém os budgets originais: avg 5ms, p99 20ms, 10k class 50ms).

3. **Estudo do rig real** (`~/Desktop/meu show robô/`): 5 controladores WLED ESP32 (192.168.2.156–160), ArtNet **WiFi**, 28 universos alocados/robô, 430 modelos Single Line (~20px), 6.200 px total, grupos hierárquicos por parte do corpo. Problemas: **35 grupos de StartChannel duplicados** (strips sobrepostas), 12 scripts Python manuais para clonar o rig, 3 crash reports do xLights. WiFi viola a Hardware Rule — migração para cabo/ETH é pré-requisito de show ao vivo.

**Invariants verified.** 574+ testes verdes (69 em led-protocols); 0 warnings; seq per-universe (ArtNet igual sACN); MTU-safe (530 bytes); gates C1–C11 intactos.

**Pending.** Importador xLights (rgbeffects+networks → CompiledLayout com gate de conflito de canais); RigBuilder (instanciar N robôs com auto-endereçamento); link z.ai não legível (página JS; navegador desconectado). Missão Production-Ready iniciada.

**Decisions.** ArtNet universe = port-address 15-bit direto (numeração igual xLights/WLED). Sequência 1..=255 skip 0 (spec: 0 desliga verificação). `ArtNetDevice` unicast-only por ora (broadcast ArtDmx satura a rede com muitos universos).

### 2026-06-27 — 10-item roadmap execution: musical_section bridge, show.lumyx demo, SectionClip, SharedClock, pipeline tests, InstrumentClassifier, AutoGpuPlasma, MetricsEmitter, lumyx-skills packaged

**Done.**

1. **`MusicalSection` em `led-core::AudioFeatures`** — adicionado `MusicalSection` enum + campo `musical_section: Option<MusicalSection>` ao v0 contract. `AudioFeatures` agora `Default`. Bridge `led-bridge/adapter.rs`: `map_section()` mapeia v1→v0 (8 variantes), `adapt()` e `adapt_into()` incluem `musical_section`. 4 novos testes de adapter (None, Some todos variants, adapt_into). `AudioScalars` em `led-pixel-engine/reactive.rs` também recebeu o campo + `publish()` o propaga.

2. **`led-demo` gera `show.lumyx`** — além do `show.gif`, `led-demo` agora escreve `show.lumyx` via `ShowWriter`. Após gerar, re-lê o arquivo e valida `pixel_hash(replayed) == pixel_hash(rerendered)` — proves deterministic replay inline. Hash atual: `0x7fea06a789edbbdb ✅`.

3. **`SectionClip`** (`crates/led-sequencer/src/section_clip.rs`) — `Effect` que roteia para sub-efeitos por seção musical atual via `AudioShare::scalars().musical_section`. HashMap O(1), default effect para None/não-mapeado, Builder pattern `with_section()`. `SectionReceiver` convenience wrapper. 7 testes (None→default, mapped, unmapped→default, switch, all 8 sections, deterministic, receiver).

4. **`SharedClock`** (`crates/led-hal/src/shared_clock.rs`) — clock monotônico com offset i64 para sync multi-node. `AtomicI64 offset` + `AtomicU64 last_now` (monotonicity guard). `calibrate_offset(ref_ts, local_ts)` helper. `Send+Sync` via atomics. 13 testes (monotonicity, offset +/−, reset, threads, calibration, two-node scenario).

5. **`MockCaptureSource::run_pipeline_sync()`** — método novo que executa o loop completo ring→Analyzer→watch sync e retorna o último `AudioFeatures`. 6 novos testes: returns_some, returns_none_on_empty, sample_rate_travels, timestamp_monotone, beat_on_impulse, musical_section_some_after_warmup.

6. **`InstrumentClassifier`** (`crates/audio-core/src/instrument.rs`) — classificação heurística por frame: Kick/Snare/HiHat/Bass/Melody/Chord/Noise/Silence/Unknown. Zero deps, zero alloc, determinístico. Usa band fractions + harmonic_ratio + f0_hz + beat flag. 8 testes (silence, 440Hz melody, 80Hz bass, impulse percussive, white noise, deterministic, silence priority, all 9 classes).

7. **`AutoGpuPlasma`** (`crates/led-pixel-engine/src/auto_gpu.rs`) — seleciona CPU vs GPU automaticamente por pixel count vs threshold (default 50k). `cpu_only()` force-CPU. `with_threshold()` override. GPU tentado silenciosamente, fallback CPU sem panic. 9 testes (below threshold, above threshold, cpu_only determinism, known value, parity, threshold accessor).

8. **`MetricsEmitter`** (`crates/led-hal/src/metrics.rs`) — observabilidade estruturada. p50/p99 via histogram HDR-lite 64 buckets (log2). Contadores `frame_count`, `drop_count`, `beat_count`, `hop_count`, `hb_gap_us`. `snapshot_json()` emite JSON uma linha. `reset()`. `Send+Sync`. 12 testes (contadores, percentis, JSON shape, reset, concurrent, hb_gap, send_sync).

9. **`lumyx-skills/dist/`** — 18 skills do Conselho de Engenharia empacotados com o empacotador oficial. 18/18 ✅.

**Invariants verified.** Audit gate: 0 Critical, 0 Warning, 8 OK. `cargo build --workspace --all-targets` → 0 errors.

**Pending.** Miri reactive 8-thread (resource). 2 tokio Type-B sleeps. `InstrumentClass` não ainda integrado ao `AudioFeatures` (campo extra) — disponível como análise standalone. `lumyx_to_skyc` (LED→drone annotation) — complexidade alta, depende de mapeamento LED pixel→drone, adiado. `MetricsEmitter` não integrado ao `Hal::send_frame()` hot-path (usuário deve chamar explicitamente — preserva zero-overhead quando não usado).

### 2026-06-28 — 10-item corrected roadmap: Provenance, Contracts, Metrics hot-path, InstrumentClass, ShowIntent, SharedClock, SyncedCluster, ChaosHarness, Replay Distribuído, Observabilidade, DroneBridge

**Done.**

1. **Provenance End-to-End** (`led-core/src/provenance.rs`): `Provenance` struct com `FrameSource` enum (Audio/Timeline/ShowIntent/Heartbeat/Simulator), `compute_pixel_hash` (FNV-1a 64-bit), `to_json()`. Integrado ao `LogicalFrame` como `provenance: Option<Provenance>` — `LogicalFrame::new()` permanece backward-compat (None), `LogicalFrame::with_provenance()` novo. 15 testes.

2. **Certificação de Contratos Canônicos** (`led-core/src/contract_version.rs`): SemVer policy documentada; `ContractRecord` + `ContractStability` (Frozen/Stable/Evolving); `certified_contracts()` = 9 contratos; versões `HAL_CONTRACT_VERSION="1.0.0"`, `LOGICAL_FRAME_VERSION="1.1.0"`, `AUDIO_FEATURES_V0_VERSION="1.2.0"`. Testes: backward-compat de `LogicalFrame::new()`, MusicalSection exhaustiva, `ProtocolOutput` object-safe. 11 testes.

3. **MetricsEmitter no hot-path + benchmark de latência** (`led-hal/src/hal.rs` + `tests/bench_latency.rs`): `Hal::with_metrics(Arc<MetricsEmitter>)` integrado — mede `Instant::now()` antes do send, `record_frame(latency_us)` depois. Zero overhead quando `None`. Benchmark: 500 frames × 512px, avg_us < 10ms debug / 500µs release; p99 < 10× avg; escala linear validada até 10k pixels. 4 testes.

4. **InstrumentClass no AudioFeatures** (`audio-core/src/instrument.rs` + `contracts.rs` + `analyzer.rs`): `InstrumentClass` enum (Kick/Snare/HiHat/Bass/Melody/Chord/Noise/Silence/Unknown). `InstrumentClassifier::classify()` — puro, determinístico, zero alloc. Integrado no `Analyzer::process_hop` → `instrument_class: Option<InstrumentClass>` em `AudioFeatures` v1 (audio-core). 8 testes.

5. **ShowIntent Generator (Deterministic Composer)** (`led-sequencer/src/show_intent.rs`): `ShowStyle` (Beat/Ambient/Drop/Narrative), `ShowIntent` (validado: energy[0,1], bpm[20,300], duration>0, pixels>0) + `intent_hash` FNV-1a. `ShowIntentGenerator::from_audio()` (seção → estilo, puro, sem LLM) + `build_timeline()` (SplitMix64 PRNG determinístico → Pulse/Rainbow + beat-flash overlay). 16 testes.

6. **SharedClock** (`led-hal/src/shared_clock.rs`): clock monotônico com offset signed para sync multi-nó; `calibrate_offset(ref_ts, local_ts)`; Send+Sync via AtomicU64. 13 testes.

7. **SyncedCluster** (`led-hal/src/cluster_sync.rs`): `SegmentHealth` (Healthy/Degraded/Failed, thresholds 3/10 falhas), `SyncedCluster: ProtocolOutput` com hot-join, failover, rejoin, drift detection, cache de last_frame. 11 testes.

8. **ChaosHarness** (`led-hal/src/chaos.rs`): `FaultConfig` (packet_loss_pct, latency_us, crash_after_frames, seed), `ChaosHarness<P: ProtocolOutput>` determinístico (SplitMix64), `run_experiment()`, enable/disable dinâmico. 11 testes.

9. **Replay Determinístico Distribuído** (`led-show-recorder/src/replay.rs`): `ReplayManifest` (frame_count, aggregate_hash, frame_hashes[], pixel_count), `verify_replay()`, `record_and_manifest()`, `cross_node_verify()`. Stress: 1000 frames, hash idêntico em dois nós simulados. 13 testes.

10. **Observabilidade Distribuída** (`led-hal/src/observability.rs`): `Span` + `ActiveSpan` + `SpanCollector` (circular buffer, p99 por nome, JSON export), `AlertRule`/`AlertEngine` (P99ExceedsUs/DropRatePct/HeartbeatGapMs/LowFrameCount), `ObservabilityReport` com `to_json()`. `AlertEngine::lumyx_standard()` = 3 regras GOSL. 10 testes.

11. **Drone Timeline Bridge** (`led-show-recorder/src/drone_bridge.rs`): `SectionEvent` → `DroneFormationHint` (Grid/Ring/Burst/Descend/Rise/Wave/Hold) mapeado de `MusicalSection` + energia, `DroneBridge::build()` (merge de segmentos curtos < min_duration_ms), `build_synced()` (LED e Drone com hints contrastantes em Chorus), JSON export. 16 testes.

12. **musical_section propagado** via `led-bridge::adapt`/`adapt_into`: `map_section(V1Section→V0Section)` converte entre namespaces. `led-core::MusicalSection` adicionado com 8 variantes. `led-core::AudioFeatures` v0 agora tem `musical_section: Option<MusicalSection>`.

**Invariants verified.** Gate audit_gate.py: 0 Critical, 0 Warning, 8 OK. `cargo build --workspace --all-targets` zero errors.

**Pending.** Miri reactive 8-thread test (resource limit). `led-show-recorder` integração com `led-demo` para gerar .lumyx junto com show.gif. `SectionClip` demonstrado em show real. `AutoGpuPlasma` integrado como padrão no `led-demo`. Coleta de spans reais no `Hal::send_frame`.

**Decisions.** `Provenance` é `Option` em `LogicalFrame` — backward-compat obrigatório. `InstrumentClassifier` integrado diretamente no `Analyzer` (não separado) — reduz latência. `ChaosHarness` usa SplitMix64 idêntico ao `ShowIntentGenerator` — seed consistente. `DroneBridge` produz somente hints, nunca waypoints — invariante lumyx-ai-governor preservada.

### 2026-06-26 — 8-item roadmap execution: GPU executor, DDP, NetworkGuard integration, SectionDetector, LiveTempoMap, Council CI gates, RouterDevice, ShowRecorder

**Done.**

1. **TD-004 CLOSED — wgpu 22.1.0 GPU executor** (`crates/led-pixel-engine/src/gpu_executor.rs`): `GpuContext::try_init()` (no hang on Metal headless), `GpuPlasmaExecutor` (pre-allocated buffers, per-frame WGSL dispatch, readback). 3 tests: init_does_not_hang, parity_with_cpu, deterministic. Evidence at `docs/evidence/td-004-wgpu-metal-fix.txt`. Added `pollster` + `bytemuck` as optional deps under `gpu` feature.

2. **DDP protocol** (`crates/led-protocols/src/ddp.rs`): `build_ddp_packet` / `parse_ddp_packet` (flags, offset big-endian, length big-endian, seq wrapping), `DdpDevice` (auto-fragment, per-device seq, pixel_offset). `DDP_MAX_PIXELS=487` (487×3=1461 ≤ 1462 MTU limit). 18 tests: wire format, round-trip, loopback, fragmentation, adversarial.

3. **NetworkGuard integrated into Hal** (`crates/led-hal/src/hal.rs`): `Hal::new()` uses `PermissiveGuard` (default, tests/simulator). `Hal::with_guard(layout, devices, guard)` accepts any `NetworkGuard`. `Hal::check_network()` — call once before show start, never inside `send_frame` (proven by `CountingGuard` test). 4 new HAL tests.

4. **SectionDetector** (`crates/audio-core/src/section.rs`): real-time musical section classification (Intro/Verse/Chorus/Build/Bridge/Drop/Outro) using dual EMA (short α=0.05, long α=0.002), rolling beat density window (50 hops), hysteresis (20 hops). Integrated into `Analyzer::process_hop` — `musical_section` is now `Some(...)` after WARMUP_HOPS=100 hops. Old "always None" contract superseded by "None before warm-up, Some after". 14 section tests + 1 new analyzer contract test.

5. **LiveTempoMap** (`crates/led-sequencer/src/live_tempo.rs`): real-time beat accumulator feeding one `AudioFeatures` pair at a time. Sorted+deduped internal buffer; BPM smoothed from last 8 intervals (200–3000ms gate). `tempo_map()` cached when dirty=false (O(1) repeat call). `snap()` for live clip alignment. 21 tests: accumulation, dedup, BPM smoothing, edge gating, caching, snap, reset, 30s simulation, tempo-change mid-stream, 10k stress.

6. **Engineering Council CI gates** (Phase 7 in `~/lumyx-e2e.sh`): 11 automated gates (C1–C11) mapping to Council members: C1 layer isolation, C2 per-universe seq, C3 zero-alloc HAL, C4 triple-buffer Miri, C5 Hann-before-FFT, C6 DDP MTU, C7 AI boundary, C8 NetworkGuard, C9 seam types in led-core, C10 warning-free, C11 heartbeat gaps ≥ GOSL minimums. All 11 PASS.

7. **RouterDevice** (`crates/led-protocols/src/router.rs`): `ProtocolBackend` trait (send_universe + protocol_name), `SacnBackend` + `DdpBackend` (both Sync via `Mutex<u8>` seq), `RouterDevice` (DeviceDriver, binary-search routing table, default backend for unmapped universes). 9 tests: dispatch, default, sorted-insert, partial-routes, UDP loopback for both protocols.

8. **led-show-recorder** (new crate `crates/led-show-recorder`): `.lumyx` binary format (16-byte header: LUMX magic + version=1 + pixel_count + frame_count). `ShowWriter`/`ShowReader` (std-only, no deps). `AudioSnapshot` (7 fields, 25 bytes serialised). `finalise_seekable()` updates frame_count. `pixel_hash()` (FNV-1a 64-bit) for regression comparison. 13 tests: round-trip, magic, pixel-count mismatch, hash determinism, finalise, stress 1000 frames.

**Invariants verified.** `cargo test --workspace` → **404 passed, 0 failed**. `cargo build --workspace --all-targets` → 0 warnings (excluding `block v0.1.6` future-incompat note from wgpu dep, not actionable). `audit_gate.py` → 0 Critical, 0 Warning, 9 OK. Engineering Council gates C1–C11 all PASS.

**Pending.** Miri reactive 8-thread test (resource limit). 2 remaining tokio async sleeps in led-protocols (Type B). wgpu parity test with real Metal GPU requires hardware (passes with skip on CI). `led-show-recorder` not yet integrated into `led-demo` (next: record show.gif session to .lumyx). `musical_section` in `led-core::AudioFeatures` still always None (separate from audio-core's SectionDetector — integration pending).

**Decisions.** `SectionDetector` warm-up is 100 hops (~2.5s at 25ms/hop); tests needing stable EMA ratios use 2500–3000 hop warm-up (5τ_long). `LiveTempoMap` BPM window=8 intervals (balances responsiveness vs. noise). `DdpBackend` uses `Mutex<u8>` for seq (not `Cell`) — required for `Sync`. `RouterDevice` silently skips unmapped universes when no default backend is set (not an error — partial rigs are common). `.lumyx` format stores raw RGB bytes (not compressed) — simplicity over size; compression can be added in v2 via format version bump.

### 2026-06-25 — WiFi-forbidden enforcement + LUMYX Engineering Council

**Done.**

*WiFi enforcement (`led-hal/src/network_guard.rs`):*
`NetworkGuard` trait (object-safe, `Send + Sync`) + `NetworkPolicyError` (typed: `WifiActive{interfaces}` /
`ProbeUnavailable{reason}`) + `WifiBlockGuard` (macOS: `networksetup -listallhardwareports` → extract
Wi-Fi device names → `ifconfig <iface>` → check `status: active`; Linux: `/sys/class/net/wl*/operstate` →
`up`; other: `ProbeUnavailable`) + `PermissiveGuard` (always passes — tests/simulator/non-hardware).
`parse_macos_wifi_interfaces` recognises both "Wi-Fi" and "AirPort" port names.
All exported from `led_hal::*`. 14 tests (5 common + 5 macOS-only + 4 error/trait). Zero warnings.

*LUMYX Engineering Council (`~/lumyx-skills/`):*
18 skills created and packaged to `dist/` (18 × `.skill`, 0 validation failures).
10-level hierarchy, 15 essential members. Each skill: `NetworkGuard` trait (agent principal +
subagents + perguntas obrigatórias + gates de bloqueio). Framework de 15 categorias de perguntas
embedded (arquitetura, determinismo, performance, concorrência, DSP, render, sequencer,
protocolos, segurança, qualidade, TD closure, PR review, governança de agentes,
anti-alucinação de IA, evolução CEO). Protocolo de consenso: 1 BLOQUEIO veta qualquer mudança.

**Invariants verified.** `cargo build --workspace --all-targets` zero warnings.
326 tests green (312 previous + 14 new network_guard). `WifiBlockGuard` does not panic on
macOS (typed result always returned). `PermissiveGuard` always `Ok(())`. `NetworkGuard`
is object-safe (compiles as `Box<dyn NetworkGuard>`). Error display includes `CRITICAL`
/ `WARNING` prefix per LUMYX Hardware Rules.

**Pending.** Miri reactive 8-thread test (resource limit — OOM/timeout do runner).
2 remaining tokio async sleeps in led-protocols (Type B — testing absence of events,
legitimate use of sleep). Real wgpu GPU executor.

**Decisions.** `WifiBlockGuard::check()` returns `ProbeUnavailable` (not panic) on
unsupported platforms — allows non-hardware environments to proceed with a warning.
Enforcement is at show-start (one call before first frame), not per-frame — zero hot-path
overhead. `PermissiveGuard` is the correct choice for `SimulatorDevice` deployments.

### 2026-06-19 — Governance: KB-012 + audit_gate + TD-006 hop-count + lumyx-e2e Phase 5b

**Done.**

KB-012 registered: "False-green gate — verification that passes without exercising the
property it claims." Root cause: non-falsifiable gates have no described run that would
make them FAIL. Two instances this session: `>= 183` slack (absorbed variance) and
Miri N=0 (ran nothing).

`scripts/audit_gate.py` (new, hardened):
- Enforces `evidence_ref` + `negative_control` on every `status: closed` TD
- Requires N>0 in `N passed; 0 failed` — explicitly rejects "0 passed" (Miri N=0)
- `required_test:` field: named test must appear by name in evidence file
- `source_files:` + `git-hash` in artefact → stale evidence detection
- `pending-verification`: valid within `review_by`, Critical past deadline

`tests/test_audit_gate.py` (new — gate's own negative control, 9/9 PASS):
  bad-ledger → exit 1 (3 Criticals: no-evidence, 0-passed, empty-negctrl)
  good-ledger → exit 0 (all 8 closed TDs verified)

`lumyx-e2e.sh Phase 5b` added: runs audit_gate + gate self-tests on every CI pass.

TD-006 (hop-count): `assert_eq!(187)` — Scenario A confirmed (10/10 runs = 187 exact).
  Falsifiable: reproves if `len == 186`. Was `>= 183` (non-falsifiable, KB-012).

`docs/evidence/`: committed artefacts for all 7 closed TDs with `git-hash:` headers.

**Invariants verified.** Gate self-tests 9/9. Real ledger: 0 Critical. 312 tests green.

**Pending.** Miri reactive 8-thread test (resource limit).

**Decisions.** evidence_ref + negative_control are mandatory fields for `closed` by
construction (gate rejects without them). `pending-verification` is a first-class status —
not `closed`, not `open`. Two fields only (KB-012: each field must pay its place by
catching a real error from this session).

### 2026-06-19 — TD-002: RT-LOCK-RENDER-001 — ArcSwap lock-free + coherent

**Done.**

`AudioShare.scalars()` adquiria lock (Mutex → RwLock) no render hot-path a cada frame,
e uma tentativa com 7 atômicos campo-a-campo era lock-free mas incoerente (tearing entre
`beat` e `timestamp_ms` quebrava `BeatFlash`). Solução final: `ArcSwap<AudioScalars>`.

```
publish(): self.scalars.store(Arc::new(AudioScalars{..}))  — 1 swap atômico, struct inteira
scalars(): *self.scalars.load().as_ref()                   — 1 load atômico, sem lock
```

Dep `arc-swap = "1"` adicionada a `led-pixel-engine/Cargo.toml` com comentário de
justificativa (RT-LOCK-RENDER-001). Zero `unsafe` em `reactive.rs`; arc-swap encapsula
seu próprio unsafe. Superfície unsafe de `led-pixel-engine` não mudou (só `triple.rs`).

Detector estrutural: `grep read()/write()/lock() reactive.rs` → ZERO em `scalars()`.
Detector semântico: `audioshare_scalars_beat_timestamp_coherent_under_concurrency`
(10k frames, `beat == timestamp_ms%2==1` em cada snapshot, 0 violações com ArcSwap,
~5000 violações teriam ocorrido com per-field atomics).

KB-011 registrado: "AudioFeatures cross-thread = snapshot coerente publicado inteiro".
Miri: gate obrigatório — rodou testes simples (subset sem threading pesado);
teste de 8 threads sob Miri excede tempo do sistema (OOM/timeout do runner).

**Invariants verified.** `grep` detector limpo. 312 tests green. Clippy 0.

**Pending.** Miri no teste de concorrência pesado (limitação de recursos — OOM/timeout).
TD-004 (wgpu→Metal, MEDIUM-1). TD-006 (wall-clock budget, MEDIUM-3).

**Decisions.** ArcSwap > tokio::watch para led-pixel-engine (std-only; watch usa RwLock
internamente). Per-field atomics descartados permanentemente (KB-011: tearing semântico).

### 2026-06-18 — HIGH-3: TEST-SLEEP-001 — 8 thread::sleep → causal barriers

**Done.**

All 8 unconditional `thread::sleep` calls in integration tests replaced with causal
spin-barriers (wait on `frames_sent() >= N` with 5s deadline + 1ms poll backoff).
Classification: all 8 were Type A (countable event, spy device available). Zero Type B found.

Wall-clock removed from critical path: ~1810ms total (350+500+150+120+120+120+250+200ms)
→ <10ms per barrier. Suite: 311 passed, 0 failed. Clippy -D warnings: 0.

Bonus fix: `contract.rs` test had `_s1` (unused spy device); now used as the event source
for the `frames_sent >= 4` assertion, making the test meaningful instead of trivially
asserting elapsed arithmetic.

**Invariants verified.** 311 tests green; causal barriers are deterministic (no false
flakiness from system load); timeout errors are diagnostic ("timeout: N/M events") not silent.

**Pending.** TD-004 (wgpu→Metal, MEDIUM-1 dedicated session). Tokio async sleeps in
led-protocols tests are cooperative yields (acceptable pattern in async runtimes — different
from the blocking thread::sleep of TD-003).

**Decisions.** TD-003 closed. Residual `sleep(1ms)` in spin-loop bodies is poll backoff,
not a fixed delay — correct by design.

### 2026-06-17 — LOW-1: cargo fix panic + clippy -D warnings + cargo-audit + ledger

**Done.**

cargo fix introduced two bugs fixed in this cycle:

[BUG-A KB-010] `capture.rs` — `slice.fill()` panic when `start > total`. cargo fix converted
a safe empty-range indexed loop into `s[start..end].fill(v)`. k=7: start=216000 > total=192000
→ panic. Guard `if start < end` added. Regression test: `mock_hop_window_past_buffer_end_no_panic`.

[BUG-B KB-009] `fft.rs` + `beat.rs` — `zip()` iterators added by cargo fix are 3-5× slower
in debug builds, breaking wall-clock budget tests (`mock_analyze_all_realtime_speed` <1s,
`classifier_10k_frames_fast` <2000ms). Reverted to indexed loops + `#[allow(clippy::needless_range_loop)]`
with explanatory comment. Confirmed: tests pass in 0.02s in release (indexed loop wins both modes).

Both bugs proven as regressions (not pre-existing): git stash + test → PASS on clean HEAD,
FAIL with changes → PASS after fix.

All workspace clippy `-D warnings` resolved (13 files). cargo-audit 0.22.2 installed via
Homebrew; 205 deps scanned, 0 vulnerabilities, 1 warning (paste 1.0.15 unmaintained,
RUSTSEC-2024-0436, no CVE — acceptable). `docs/technical-debt-ledger.md` created (canonical
TD tracker); `docs/knowledge-base.md` created (KB-009, KB-010 permanent failure records).

**Invariants verified.** 80 audio-core tests green (including new regression test).
Clippy workspace -D warnings = 0. cargo audit = 0 vulns.

**Pending.** TD-003 (8 thread::sleep in tests, MEDIUM-3). TD-004 (wgpu→Metal, MEDIUM-1).

**Decisions.** cargo-audit installed via Homebrew bottle (no compile, 14MB). Ledger is
the file `docs/technical-debt-ledger.md` going forward — not conversation-only.
TD-003 NOT closed: the 8 sleep() calls in tests are untouched; what was fixed in this
cycle is a different set of timing regressions (KB-009, now TD-009).

### 2026-06-16 — CI Cycles 6-8: clustering, CPAL mock, harmonic gating, GPU, E2E script

**Done.**

*Cycle 6:* `ClusteredHal` bug fix (universe index per-domain); CPAL `MockCaptureSource` doctest fix + 9 adversarial tests (stereo downmix, timestamps, bass tone, silence, 10s stress, real-time speed); `HarmonicClassifier` — new `audio-core/harmonics.rs` module (Inf guard, 10 tests: sine tonal, noise not tonal, f0_hz accuracy ±2 bins, NaN/Inf robustness, integration with MockCapture). Exported as `audio_core::HarmonicClassifier + TONAL_THRESHOLD`.

*Cycle 7:* Harmonic gating integrated into `Analyzer`: `TONAL_GATE_MIN=0.80`; beat suppressed when `harmonic_ratio ≥ 0.80` (pure sines), passes when click-on-sine (ratio ~0.6). 4 gating tests. `AudioFeatures` v1.1: added `harmonic_ratio: f32` field. `ClusterHeartbeat`: wraps `Arc<ClusteredHal>` + `Arc<Heartbeat>`; `beat()` and `spawn()` drive all segments atomically; 4 tests including threaded fires + gap ≤ HEARTBEAT_MS < WARN_GAP_MS. DSP bugs fixed: `TONAL_GATE_MIN` tuning; alternating-±0.5 "noise" is actually a square wave → replaced with 7 incoherent frequencies.

*Cycle 8:* GPU adversarial tests (10 new): WGSL structural validation (1 `@compute`, ≥2 bind groups), params struct complete, parity at t=0 and t=u64::MAX, zero/extreme scale, 10k-pixel stress, 100 time-steps × 256px, CPU render < 5ms, all pixels written, production path documented. `led-bridge` adapter v1.1: `harmonic_ratio()` + `is_tonal()` helper fns. `SimOutput` v2: `harmonic_ratio_log: Vec<f32>` field tracking harmonic content per hop. E2E validation script `~/lumyx-e2e.sh` (cross-platform: LED + Drone + 5 invariant checks + SimLoop + optional Miri).

**Invariants verified.** Harmonic gating: ≤2/50 false beats on sustained 440Hz; real broadband impulse still fires; sine harmonic_ratio > multi-tone. ClusterHeartbeat: both segments get equal frame count; no frame → nothing sent; threaded fires ≥3 in 250ms. GPU: CPU-GPU parity at all time steps and pixel counts; WGSL contains required binding + compute annotations. E2E script: all 5 cross-platform invariants pass (NaN drone, heartbeat never-zeros, zero-alloc audio, triple buffer, harmonic gating).

**Pending.** Real wgpu GPU dispatch (needs GPU hardware + `--features gpu`). Cross-workspace shared type for drone+LED combined output. CPAL capture test with real device.

### 2026-06-15 — CI Cycle 5: TempoMap live-beats, jitter, protocol chaos, multi-system

**Done.**

*P1 — TempoMap from live beats (led-sequencer, 8 tests):*
`from_beat_flags` sorted+deduped invariant; 120 BPM beat-time accuracy ±2 hops; `snap()` to nearest beat; fuzz with empty/all-false stream; jitter tolerance ±10ms; constant vs detected BPM agreement; 10k stream build <50ms.

*P2 — Scheduler jitter simulation (led-bridge/sim.rs, 5 tests):*
`SimLoop::run_with_jitter()` injects hop timestamp gaps. Tests: 50% / 100% / 80% jitter survive; sample_rate valid throughout; pixels valid; zero-jitter == normal run.

*P3 — Protocol chaos (led-protocols/packet.rs, 8 tests):*
Sequence wrap 255→0 detected; out-of-order via signed diff; corrupted ACN PID detected; corrupted universe no panic; short buffer no panic; burst 256 sequential packets all valid; heartbeat after seq wrap preserves payload.

*P4 — Multi-system simultaneous (led-bridge/tests/multi_system.rs, 5 tests):*
LED thread (SimLoop→adapt→HAL) + Drone safety (O(n²) 50-drone) concurrent: both complete within budget. AudioShare under 200Hz write + 60fps read: no deadlock. 2 independent HAL instances: independent content (red vs blue). Drone + LED heartbeat concurrent: 0 violations, ≥2 heartbeats. Stress: 4 LED + 4 Drone threads, all pass.

*P5 — Miri:*
`audio-core ring_buffer` 5 PASS (SPSC `unsafe impl Sync`). `led-bridge/adapter` **6 PASS** (adapt/adapt_into/NaN/1M iter — no UB detected, ~7min under Miri).

**Invariants verified.**
- TempoMap::from_beat_flags: sorted, deduped, consistent with constant BPM at ±2 hops.
- Jitter: sample_rate never corrupted; pixels always valid u8; run_with_jitter(0,0)=run().
- Protocol chaos: corrupted PID detected; sequence wrap valid; no panic on bad inputs.
- Multi-system: 4+4 threads complete; AudioShare no deadlock; HAL instances independent.
- 214 tests, 0 warnings.

**Pending.** Real wgpu GPU executor; multi-device clustering; harmonic/overtone detection for richer beat classification; cross-workspace drone+LED integration test (requires shared workspace or FFI boundary); CPAL capture test (no hardware).

### 2026-06-15 — CI Cycles 1-4: adversarial suites, audio→LED bridge, BeatDetector v2

**Done.**

*Cycles 1-2 — adversarial test suites:*
Added 52+ adversarial tests across `led-sequencer/timeline` (determinism, 1k overlapping clips, marker flood, blend invariants, u64::MAX), `audio-core/contracts` (spectrum len, Copy stress, timestamp monotonicity), `led-protocols/packet` (wire format, fuzz, 10k build stress), `led-protocols/pool` (1M chaos, 16-thread concurrent), `led-pixel-engine/triple` (1M cycles no torn frames, concurrent threads, latency), `led-pixel-engine/reactive` (8×8 concurrent AudioShare, NaN/Inf handling), `drone-safety` (geofence boundary, NaN/∞, 200-drone O(n²)), `drone-trajectory` (smoothstep invariants, 1k-drone stress).

Fixed 4 bugs: [BUG-3] `smoothstep(NaN)` propagated NaN into drone positions (CRITICAL — SAFETY); [BUG-4] `fits_envelope(dur≤0)` returned `true` via negative-speed comparison (CRITICAL — SAFETY); [BUG-5] `BufferPool` grows without bound under burst load (design risk, documented); [BUG-6] `led-core::AudioFeatures` is not `Copy` (test error).

*Cycle 3 — audio→LED bridge:*
New crate `led-bridge`: `adapter.rs` (`adapt`/`adapt_into`, v1→v0, zero-alloc after warmup, ptr-comparison proof), `bridge.rs` (`BridgeHandle`, tokio current_thread runtime, watch→AudioShare, clean shutdown), `sim.rs` (`SimLoop`: SineGen+BeatImpulse→Analyzer→adapt→AudioShare→BandPulse/BeatFlash→SimOutput, deterministic, <5ms/hop). 23 unit tests.

DSP finding: 440Hz sine with 75% overlap produces ~55 false beats/2s (Hann-windowing non-integer bin rotation). Paradox: adding impulses REDUCES beats (EMA threshold elevation). Documented and tracked.

*Cycle 4 — BeatDetector v2, heartbeat timing, E2E stack, Miri:*
`BeatDetector::new()` tuned: sensitivity 1.5→2.3, refractory 3→8 frames. Validated: 120 BPM still detected (≥8/10 beats), sustained flat spectrum no longer re-triggers. New regression suite (5 tests).
Heartbeat real-timing tests: thread fires ≥2× in 350ms at 100ms interval; gap thresholds match LUMYX_GOSL (HEARTBEAT_MS=800 < WARN_GAP_MS=2000 < CRIT_GAP_MS=2500).
E2E integration tests (`led-bridge/tests/e2e_pipeline.rs`, 7 tests): SimLoop→adapt→AudioShare→effects→LogicalFrame→Hal→SimulatorDevice full stack verified; full-stack latency <5ms avg; heartbeat resends last sim frame.
Miri: `audio-core ring_buffer` 5 tests PASS (SPSC `unsafe impl Sync` verified). `led-pixel-engine/triple` Miri verified in previous sessions (24 scheduler seeds).

**Invariants verified.**
- smoothstep(NaN)=0.0 (never propagates into drone positions).
- fits_envelope(dur≤0)=false always (negative/zero duration always fails safety gate).
- adapt_into() ptr-stable after warmup (zero-alloc on steady-state bridge).
- BeatDetector: refractory=8 blocks exactly 8 frames; sensitivity=2.3 rejects sustained-sine windowing flux; 120 BPM detection ≥80%.
- Heartbeat thread fires within GOSL budget; never sends zeros.
- SimLoop deterministic: same config→same output; timestamp monotone; <5ms/hop.
- E2E: pixel 0 maps to device channel 0 with correct RGB order; mapping applied exactly N×.
- Miri clean: ring_buffer SPSC (5 tests); triple buffer (24 scheduler seeds, prior session).
- 186 tests, 0 warnings.

**Pending.** Real wgpu GPU executor; multi-device clustering; WiFi-forbidden enforcement; `audio-core` CPAL capture not testable without hardware. `BeatDetector` EMA-paradox on impulse+sine (documented in `sim.rs`). Cycle 5 targets: harmonic/overtone detection, TempoMap-from-live-beats integration, latency measurement under simulated scheduler jitter.

**Decisions.** BeatDetector defaults changed globally (v2); downstream consumers using `BeatDetector::new()` will see stricter gate — correct direction for production. `led-bridge` is the permanent adapter seam; never import `audio-core` from any other workspace crate. `SimLoop` is the canonical E2E regression target for future DSP and bridge changes.

### 2026-06-10 — `audio-core`: realtime audio intelligence (leaf crate)

**Done.** Added `audio-core`, a new leaf crate (lumyx-system-architect §6: imports nothing
from sequencer/effect-engine/protocols/led-core). Pipeline: CPAL default-input capture
(`capture.rs`, F32/I16/U16, downmixed to mono) → SPSC lock-free `RingBuffer` (`ring_buffer.rs`)
→ `Analyzer` (`analyzer.rs`) sliding a 1024-sample window 256 samples at a time (75%
overlap) → Hann window (`window.rs`) → `rustfft` magnitude spectrum with preallocated
scratch (`fft.rs`) → band energy/RMS/peak/spectral centroid/rolloff (`bands.rs`) →
spectral-flux beat/onset detection with `flux_avg = flux_avg*0.9 + flux*0.1`
(`beat.rs`) → smoothed BPM (`bpm.rs`) → `AudioFeatures` (`contracts.rs`, the
lumyx-system-architect v1.0 contract: adds `peak`/`onset`/`bpm`/`spectral_centroid`/
`spectral_rolloff`/`spectral_flux`/`musical_section` vs `led-core`'s; `spectrum` is a fixed
`[f32; 512]` so the struct is `Copy`) → broadcast via `tokio::sync::watch`
(`pipeline.rs::AudioPipeline`). 26 new tests (25 unit/lib + 1 `tests/no_alloc.rs`).

**Invariants verified.** Hann-before-FFT (`fft::SpectrumAnalyzer::magnitude_spectrum` is the
only FFT path, takes the window as a required arg); `sample_rate` explicit end-to-end (from
CPAL device config through `Analyzer` to every `AudioFeatures`, `bands` tests prove
bin↔Hz uses it not a hardcoded rate); spectral-flux beat fires on bursts not
silence/sustain with the specified 0.9/0.1 EMA and a refractory window (`beat.rs`); BPM
tracker converges to 120 on a steady 500 ms beat (`bpm.rs`). Zero-alloc hot path:
`audio-core/tests/no_alloc.rs` proves 1000 `Analyzer::process_hop` + `watch::send` cycles
allocate nothing after warm-up (relies on `AudioFeatures: Copy` + `rustfft`'s
`process_with_scratch` + preallocated FFT/window/ring buffers). The new `unsafe impl Sync`
+ `unsafe` cells in `RingBuffer` are covered by an SPSC stress test, Miri-clean
(`cargo +nightly miri test -p audio-core --lib ring_buffer::`) and across 8 scheduler seeds
with `-Zmiri-many-seeds`/`-Zmiri-preemption-rate`. Workspace stays warning-free; 103/103
tests green (`cargo test --workspace`).

**Pending.** `audio-core` is not wired into the existing render-side `AudioShare`
bridge — it currently has no consumers in this workspace. CPAL capture (`capture.rs`,
`pipeline.rs`) cannot be exercised by automated tests here (no audio hardware in the
sandbox); only the hardware-independent DSP/ring-buffer/analyzer modules have tests.
`musical_section` is always `None` (realtime-only pipeline, per data-contracts.md). U16
CPAL format is supported for downmixing; other sample formats (I8/I32/I64/U8/U32/U64/F64)
return `AudioCoreError::UnsupportedSampleFormat`.

**Decisions.** Per lumyx-system-architect §10/§15 ("when sub-skills conflict, this document
wins, flag the conflict"): built `audio-core` as a standalone leaf with its **own**
`AudioFeatures` v1.0 (the richer architect-skill contract) rather than reusing/extending
`led-core::AudioFeatures` (the smaller Phase-1 contract `led-audio`/`led-pixel-engine`
already depend on) — flagged in the crate map as a divergence to reconcile later, not
silently merged. Chose a fixed-size `[f32; 512]` `spectrum` field (vs the contract doc's
`Vec<f32>`) specifically so `AudioFeatures` is `Copy` and the `watch` channel send is
allocation-free — a deliberate, documented deviation in service of invariant 3.
`cpal`/`rustfft`/`tokio` (sync feature only) are `audio-core`'s only dependencies, scoped to
this leaf so the rest of the workspace stays std-only.

### 2026-06-04 — Rendered demo + git baseline

**Done.** Added `led-demo` (binary): renders a 6 s show to `show.gif` (384×216) — a 32×18
matrix driven by the real render path (layout → `Timeline` with a `Plasma` compute effect +
beat-synced white flashes on a 120 BPM `TempoMap`, Add blend), encoded with the `gif` crate.
First watchable artifact. Initialized git in both `~/led-platform` and `~/drone-show-suite`
(local identity, `main`, initial commits).

**Invariants verified.** Workspace still warning-free and 54/54 green with the new binary;
libraries remain std-only (only the `led-demo` app pulls `gif`). The demo uses the same
`Effect::render` path the pipeline drives — no special-case rendering.

**Pending.** Push to a remote (backup); real wgpu executor (`gpu` feature); drone codebase
(safety+sim); multi-device clustering; realtime audio.

**Decisions.** Demo is a separate binary crate so the libs stay dependency-free. Now that
there are real deps (`gif`), `Cargo.lock` is tracked (committed) for reproducible builds.
`show.gif` is committed as the demo artifact.

### 2026-06-03 — Phase 1 foundation + render core + governance

**Done.** Stood up the `~/led-platform` Rust workspace (std-only) as 5 crates: `led-core`
(seams), `led-hal` (HAL facade, `SimulatorDevice`, `Heartbeat`+async thread, `Core`,
`IDevice`), `led-layout` (model, MegaTree/matrix-serpentine generators, `LayoutMapper`),
`led-protocols` (`SacnDevice` = real E1.31 packets over UDP), `led-pixel-engine` (effects,
HSV/gamma, lock-free triple buffer, render→send `Pipeline`), `led-sequencer` (non-destructive
`Timeline` — clips, fades/crossfade, opacity keyframes, add/multiply/override blend — which
*is* an `Effect`, so the pipeline drives it directly), `led-audio` (std-only Hann-windowed
radix-2 FFT, band energy, spectral-flux beat detection → `AudioFeatures`). Added the
`AudioFeatures` seam type to `led-core`. Built the audio→light bridge in `led-pixel-engine`
(`reactive`): `AudioShare` (latest features; scalar reads are Copy/alloc-free, spectrum
behind a borrow) + `BandPulse`/`BeatFlash` reactive effects — `led-pixel-engine` reads
`AudioFeatures` from `led-core`, so it does NOT depend on `led-audio`. Added beat-sync to
`led-sequencer`: `TempoMap` (constant BPM or explicit/detected beats, incl.
`from_beat_flags` over `AudioFeatures`) + `Clip::on_beats`/`Clip::snapped`/`Keyframe::on_beat`
— beat timings resolve to ms at build time, so render stays non-destructive. Added pro
output to `led-protocols`: per-universe **multicast** sACN (`SacnDevice::multicast`, group
239.255.hi.lo, multicast TTL/loop set) and **ArtPoll/ArtPollReply** source-conflict
detection (`find_conflicts` names the other IP for an overlapping universe). Added GPU-style
compute effects: a portable per-pixel `ComputeKernel`/`ComputeEffect` (`Plasma`) runnable on
CPU now + the matching `PLASMA_WGSL` `@compute @workgroup_size(64)` shader, with the real
wgpu executor specified behind a hardware-gated `gpu` feature (`references/gpu-compute.md`).
Added governance: `LUMYX_GOSL.md` (Definition of Done, Hardware Rules, standard commands
incl. `/changelog`) and this `CLAUDE.md`. 54 tests across 7 crates.

**Invariants verified.** One-mapping-applied-once + Core-only-`ProtocolOutput` + fan-out by
ownership + heartbeat-never-zeros (`led-hal` contract.rs, lifecycle.rs); no hot-path
allocation (`no_alloc.rs`, counting allocator); render/send never share a buffer (`triple`
stress 200k + **Miri clean across 24 scheduler seeds**); correct E1.31 bytes + per-universe
wrapping sequence (`sacn_wire.rs`); layout→mapper→HAL→device + serpentine order
(`end_to_end.rs`); `IDevice` firmware-refused-on-live (lifecycle.rs). Sequencer:
non-destructive re-render + blend modes + crossfade + opacity keyframes + Timeline-as-Effect
seam (`led-sequencer` lib.rs unit tests + `pipeline_drive.rs`). Audio: Hann zero-at-ends +
symmetry, FFT peaks at the tone bin, **Hann reduces leakage** vs rectangular,
**sample_rate is explicit** (same buffer ⇒ different Hz), band energy tracks the tone,
spectral-flux beat fires on onset not sustain/silence + refractory, `AudioFeatures` carry
their sample_rate (`led-audio` unit tests). Bridge: reactive `BandPulse` tracks band energy
+ `BeatFlash` triggers-on-new-beat-then-decays (alloc-free scalar reads); end-to-end real
Analyzer → `AudioShare` → effect → pipeline → HAL → device (`led-pixel-engine`
reactive.rs + audio_bridge.rs). Beat-sync: `TempoMap` beat↔ms + snap (constant/offset/
explicit/from-audio-flags), clips on the beat grid, keyframes on beats, all deterministic
(`led-sequencer` tempo.rs + lib.rs tests). Multicast: per-universe group addressing
(deterministic unit test) + a best-effort loopback delivery test; ArtPoll: build/parse
round-trip + `find_conflicts` names the offending IP, proven over a UDP loopback
(`led-protocols` artnet.rs + artnet_conflict.rs + sacn_multicast.rs). GPU compute: portable
`Plasma` kernel deterministic + known-value (cyan at origin/t=0), fills every pixel; the WGSL
mirrors the CPU math (`led-pixel-engine` compute.rs). Build warning-free.

**Pending.** Beat-synced clip timing in the sequencer (consume beat/tempo), multicast sACN +
ArtPoll source-conflict, GPU compute effects. `/seam` and `/security` are defined but not yet
executable checks. Miri run only on `led-pixel-engine`. No git commits yet (by request).
WiFi/2.4 s rules are documented but there is no live-output transport code to enforce them
against yet.

**Decisions.** Extracted `led-core` so `led-hal`/`led-layout`/`led-protocols` depend on a
neutral core (clean DAG, no cycles). `Hal` holds `Vec<Arc<dyn DeviceDriver>>` (sidesteps the
orphan rule now that the trait is foreign, and lets tests keep an inspection handle). Triple
buffer is 3 `UnsafeCell` slots + 1 `AtomicUsize` (index|fresh) with a permutation invariant —
that invariant *is* the safety proof. `SacnDevice` is unicast for testability with a
`multicast_addr` helper present for production. Governance docs live at the codebase root.
