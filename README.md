# led-platform

Reference implementation of the LED pixel platform whose architecture is specified in the
`led-strip-platform` skill suite (`~/led-strip-platform-skill/`). Built **slice by slice**,
foundation first — each slice proves a load-bearing contract in real, compiling, tested
code rather than prose.

## What's built (Phase 1 foundation + render core)

```text
led-layout ──┐  builds logical pixels + compiles the ONE mapping
             │
led-sequencer: Timeline (clips/fades/keyframes/blend) — IS an Effect ──┐
led-pixel-engine: Effect ──render──▶ [ triple buffer ] ──send thread──┐│
             │     (render thread, logical space)   (lock-free handoff)││
             ▼                                                         ▼▼
Core ── LogicalFrame ──────────────────────────────▶ Hal ──(apply mapping once)──▶ DeviceDriver fan-out
                                                                          ├─ SimulatorDevice  (virtual)
                                                                          └─ SacnDevice       (E1.31 on the wire)
                                  Heartbeat (independent thread) ─────────┘ resends last valid frame, never zeros

led-audio: samples ──Hann FFT──▶ AudioFeatures ──▶ AudioShare ──▶ reactive effects (BandPulse/BeatFlash) ──▶ (same pipeline above)
```

### Crates

| Crate | Owns | Maps to sub-skill |
|---|---|---|
| `led-core` | seam types, `ProtocolOutput`/`DeviceDriver`/`IDevice`, `CompiledLayout` | the master §3 seams |
| `led-hal` | `Hal` facade (sole `ProtocolOutput`), `SimulatorDevice`, `Heartbeat`, `Core` | `led-hal` |
| `led-layout` | `PixelLogical`/`Layout`, prop generators, `LayoutMapper` | `led-layout` |
| `led-protocols` | `SacnDevice` (E1.31, unicast + per-universe multicast) + packet builder + ArtPoll source-conflict | `led-protocols` |
| `led-pixel-engine` | `Effect`s, HSV/gamma, lock-free triple buffer, render→send `Pipeline`, audio-reactive bridge, GPU-style compute kernels (`Plasma` + WGSL) | `led-pixel-engine` |
| `led-sequencer` | non-destructive `Timeline`/`Track`/`Clip`/keyframes + `TempoMap` beat-sync; a `Timeline` *is* an `Effect` | `led-sequencer` |
| `led-audio` | std-only Hann FFT, band energy, spectral-flux beat detection → `AudioFeatures` | `led-audio` |

### What the tests prove (`cargo test --workspace` — 54 tests, all green)

| Invariant (from the skill suite) | Test |
|---|---|
| The Core reaches hardware **only** through `ProtocolOutput` | `led-hal contract.rs` |
| **One mapping, applied once** per frame, then fan-out | `led-hal contract.rs` |
| Each device gets **only the universes it owns** | `led-hal contract.rs` |
| Keep-alive resends the **last valid frame, never zeros** | `led-hal contract.rs` + `lifecycle.rs` |
| **Zero allocation on the hot path** (counting allocator, 10k frames) | `led-hal no_alloc.rs` |
| Independent heartbeat thread keeps sending (≥1 Hz) | `led-hal lifecycle.rs` |
| `IDevice` lifecycle + **firmware refused on a live device** | `led-hal lifecycle.rs` |
| **Correct E1.31 bytes on the wire**, per-universe wrapping sequence | `led-protocols sacn_wire.rs` |
| Full chain **layout → mapper → HAL → device**; serpentine wiring order | `led-layout end_to_end.rs` |
| **Triple buffer never tears** under two threads (200k publishes); latest-value semantics | `led-pixel-engine triple.rs` |
| HSV primaries, gamma endpoints/monotonicity, brightness scaling | `led-pixel-engine color.rs` |
| Effects are **deterministic in time**; pulse trough is dark | `led-pixel-engine effect.rs` |
| **Render→send pipeline** drives a device across two real threads | `led-pixel-engine pipeline.rs` |
| Clips schedule in time; **crossfade**, add/multiply/override blend, opacity keyframes | `led-sequencer` lib tests |
| Composition is **non-destructive** (same t ⇒ same frame) | `led-sequencer` lib tests |
| A **`Timeline` drives the pipeline** as an `Effect`, end to end | `led-sequencer pipeline_drive.rs` |
| FFT peaks at the tone bin; **Hann reduces leakage**; `sample_rate` is explicit | `led-audio fft.rs` + `bands.rs` |
| Band energy tracks the tone; **spectral-flux beat** fires on onset, not sustain | `led-audio bands.rs` + `beat.rs` |
| Audio-reactive: `BandPulse` tracks energy; `BeatFlash` triggers-then-decays | `led-pixel-engine reactive.rs` |
| **End-to-end audio→light**: real Analyzer → `AudioShare` → effect → pipeline → device | `led-pixel-engine audio_bridge.rs` |
| **Beat-sync**: `TempoMap` beat↔ms + snap; clips/keyframes on the beat grid (incl. from detected beats) | `led-sequencer tempo.rs` + lib tests |
| **Multicast** sACN: per-universe group addressing (239.255.hi.lo) + loopback delivery | `led-protocols device.rs` + `sacn_multicast.rs` |
| **ArtPoll source-conflict**: build/parse + `find_conflicts` names the other IP, over the wire | `led-protocols artnet.rs` + `artnet_conflict.rs` |
| **GPU-style compute**: portable `Plasma` kernel deterministic + known value; WGSL mirrors it | `led-pixel-engine compute.rs` |

### Run it

```sh
cargo test --workspace
```

## Next slices (not built yet)

- Wire the real **wgpu GPU executor** behind a `gpu` feature (hardware-gated; the WGSL +
  CPU reference are already in place — see `crates/led-pixel-engine/references/gpu-compute.md`).
- Multi-device **clustering** / shared frame deadline (PTP/NTP clock domain).
- Realtime audio capture (Phase 3).

Each maps directly to its sub-skill in `~/led-strip-platform-skill/`.

## Hardening notes

- The triple buffer is `unsafe` lock-free code. It is validated three ways:
  1. a threaded no-tearing test (200k publishes) that passes repeatably in `--release`;
  2. **Miri** clean — no UB, no data race (`cargo +nightly miri test -p led-pixel-engine --lib`);
  3. **Miri across 24 scheduler seeds** with raised preemption
     (`MIRIFLAGS="-Zmiri-many-seeds=0..24 -Zmiri-preemption-rate=0.1"`) — all pass, so the
     Release/Acquire handoff holds under many thread interleavings, not just one.
  The test auto-shrinks its workload under `cfg!(miri)` so the interpreted run stays fast.
