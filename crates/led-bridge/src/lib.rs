//! # led-bridge — the only seam between audio-core (v1) and led-pixel-engine (v0)
//!
//! ## Why this crate exists
//!
//! The LUMYX platform has two `AudioFeatures` contracts that must never merge:
//!
//! | Contract | Owner | Shape | Purpose |
//! |---|---|---|---|
//! | v0 | `led-core` | `Vec<f32>` spectrum, 7 scalars | Phase-1 LED effects |
//! | v1 | `audio-core` | `[f32; 512]` spectrum, 15 scalars, `Copy` | Realtime DSP |
//!
//! **This crate is the only place that imports both.** Every other crate sees only one side.
//! Adding a dependency on this crate is *architecturally significant* — it means the
//! consumer is wiring the audio intelligence layer into the render pipeline.
//!
//! ## Components
//!
//! - [`adapt`] — pure function: `audio_core::AudioFeatures` → `led_core::AudioFeatures`.
//!   No allocation on the first call (spectrum Vec is pre-sized); after that, zero-alloc.
//! - [`BridgeHandle`] — spawns a dedicated bridge thread that polls a
//!   `tokio::sync::watch::Receiver<audio_core::AudioFeatures>` and calls
//!   `AudioShare::publish` at every new analysis frame.
//! - [`SimLoop`] — a fully-synthetic, hardware-free live loop for tests and CI:
//!   sine + beat impulse → `audio_core::Analyzer` → adapt → `AudioShare` → effects.

pub mod adapter;
pub mod bridge;
pub mod sim;

pub use adapter::adapt;
pub use bridge::BridgeHandle;
pub use sim::SimLoop;
