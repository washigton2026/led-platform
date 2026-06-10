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
cargo test --workspace                  # all suites
cargo build --workspace --all-targets   # must be warning-free
cargo +nightly miri test -p led-pixel-engine --lib   # lock-free unsafe under Miri
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

Data flow: `led-layout` compiles the mapping → `led-sequencer` composes effects over time
(a `Timeline` *is* an `Effect`) → `led-pixel-engine` renders `LogicalFrame`s → (lock-free
triple buffer) → `Hal` applies the mapping **once** → `DeviceDriver` fan-out →
`SimulatorDevice` / `SacnDevice`. `Heartbeat` runs on its own thread.

Audio→light (Phase-1, wired): `led-audio` analyzes samples → `led-core::AudioFeatures` →
`led-pixel-engine`'s `AudioShare` (written by the audio thread) → reactive effects
(`BandPulse`, `BeatFlash`) read it on the render thread. `led-pixel-engine` consumes
`AudioFeatures` from `led-core`, so it does **not** depend on `led-audio` (only the
app/test wires them together).

Audio intelligence (`audio-core`, not yet wired): a separate, richer realtime pipeline —
own `AudioFeatures` (adds `peak`, `onset`, `bpm`, `spectral_centroid`, `spectral_rolloff`,
`spectral_flux`, `musical_section`; `spectrum` is `[f32; 512]` not `Vec<f32>` for a `Copy`,
alloc-free struct) — per the `lumyx-system-architect` skill's contract (§3/§11, owner
`audio-core/`). **Contract divergence flagged per that skill's §10/§15**: `led-core`'s
smaller `AudioFeatures` (consumed by `led-audio`/`led-pixel-engine` above) is now the
outdated sub-skill version relative to the architect skill's canonical contract. Not yet
reconciled — `audio-core` does not feed the existing `AudioShare` bridge. Future work:
either migrate `led-core::AudioFeatures` to the v1.0 shape (major-version, multi-crate
change, see `references/data-contracts.md`) or add an adapter at the `AudioShare` boundary.

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

8 lib crates + `led-demo` binary · **103 tests green** (`cargo test --workspace`) · zero
warnings · a runnable demo renders `show.gif` (a watchable show) · triple buffer validated
natively (200k-publish stress) and under Miri across 24 scheduler seeds.

Built: HAL core + mapping, layout (MegaTree/matrix-serpentine) + mapper, E1.31 driver,
render core (effects + triple buffer + render→send pipeline), async heartbeat, `IDevice`
lifecycle with firmware safety, non-destructive sequencer (clips, fades/crossfade, opacity
keyframes, add/multiply/override blend; the timeline drives the pipeline as an `Effect`),
audio analysis (Hann FFT, band energy, spectral-flux beat → `AudioFeatures`), audio→light
bridge (`AudioShare` + `BandPulse`/`BeatFlash` reactive effects driven by live features),
beat-synced clip/keyframe timing (`TempoMap` from constant BPM or detected beats),
per-universe multicast sACN, ArtPoll source-conflict detection, GPU-style compute effects
(portable per-pixel `Plasma` kernel + matching `PLASMA_WGSL`; CPU executor tested, real
wgpu executor specified behind a hardware-gated `gpu` feature).

Realtime audio (`audio-core`, new): CPAL default-input capture (F32/I16/U16 formats,
downmixed to mono) → SPSC ring buffer → sliding 1024-sample window (256-sample hop, 75%
overlap) → Hann window → `rustfft` → bands/centroid/rolloff/spectral-flux beat (EMA
0.9/0.1)/BPM → `AudioFeatures` (v1.0, lumyx-system-architect contract) → broadcast via
`tokio::sync::watch`. Leaf crate: no workspace dependencies, no imports of
sequencer/effect-engine/protocols.

Not built yet: wiring the real wgpu GPU executor (needs a `gpu` feature + GPU hardware),
multi-device clustering / shared frame deadline, reconciling `audio-core`'s `AudioFeatures`
with `led-core`'s (see crate map), wiring `audio-core` into the render-side `AudioShare`
bridge.

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
