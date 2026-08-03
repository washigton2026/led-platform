//! # led-pixel-engine — the render core
//!
//! Turns effects + time into per-pixel colors, in **logical space**, and hands frames to
//! the output edge through a lock-free [`triple`] buffer (the render→send handoff the v2
//! design got wrong). The first layer *above* the HAL.
//!
//! - [`effect`] — the [`Effect`](effect::Effect) trait + `SolidColor`, `Rainbow`, `Pulse`.
//! - [`color`] — HSV→RGB, gamma table, brightness scaling.
//! - [`triple`] — the wait-free triple buffer (`render` and `send` never share a buffer).
//! - [`pipeline`] — wires a render thread + send thread, decoupled by the triple buffer.
//! - [`reactive`] — `AudioShare` bridge + audio-reactive effects (`BandPulse`, `BeatFlash`)
//!   that consume `led_core::AudioFeatures` without this crate depending on `led-audio`.
//! - [`compute`] — GPU-style per-pixel compute kernels (`Plasma`) runnable on CPU now, with
//!   the matching WGSL (`PLASMA_WGSL`) for the GPU executor (`gpu` feature, hardware-gated).
//! - [`library`] — a biblioteca de efeitos de show (`Chase`, `Twinkle`, `Fire`, `ColorWash`,
//!   `Strobe`, `Meteor`, `Lightning`, `Ripple`) — toda ela função pura do tempo (ADR-0021).
//! - [`noise`] — aleatoriedade **sem estado** (hash de coordenadas) que os efeitos acima usam
//!   no lugar de um PRNG, para que o replay determinístico continue valendo.

pub mod auto_gpu;
pub mod color;
pub mod compute;
pub mod effect;
pub mod gpu;
pub mod library;
pub mod noise;
pub mod pipeline;
pub mod reactive;
pub mod triple;

/// Real wgpu GPU executor for [`compute::ComputeKernel`]s.
/// Only compiled with `--features gpu` — skips gracefully when no adapter is available.
/// Contains [`GpuContext`] (one-time adapter init that does NOT hang on Metal headless,
/// proving TD-004 is resolved) and [`GpuPlasmaExecutor`] (pre-allocated, per-frame dispatch).
#[cfg(feature = "gpu")]
pub mod gpu_executor;

pub use auto_gpu::{AutoGpuPlasma, GPU_THRESHOLD_PIXELS};
pub use compute::{ComputeEffect, ComputeKernel, Plasma, PLASMA_WGSL};
pub use effect::{Effect, Pulse, Rainbow, SolidColor, Vec3};
pub use gpu::assert_cpu_gpu_parity;
pub use library::{Chase, ColorWash, Fire, Lightning, Meteor, Ripple, Strobe, Twinkle};
pub use noise::{fbm, hash01, mix64, value_noise};
pub use pipeline::{spawn, PipelineHandle};
pub use reactive::{AudioScalars, AudioShare, Band, BandPulse, BeatFlash};
pub use triple::{triple_buffer, Consumer, Producer};

#[cfg(feature = "gpu")]
pub use gpu_executor::{GpuContext, GpuPlasmaExecutor};
