//! # audio-core — realtime audio intelligence (leaf crate)
//!
//! Pipeline: **CPAL capture -> Hann window -> rustfft -> [`AudioFeatures`]**, published on a
//! [`tokio::sync::watch`] channel. This crate is the canonical owner of the
//! [`AudioFeatures`] contract (lumyx-system-architect §3 / §11, v1.0) — see
//! [`contracts`].
//!
//! ## Architecture (lumyx-system-architect)
//!
//! - **Leaf crate** (architect §6): `audio-core` does not import `led-core`,
//!   `led-sequencer`, `led-pixel-engine` (effect-engine), `led-protocols`, or any other
//!   workspace crate. Consumers read [`AudioFeatures`] off the watch channel — never by
//!   depending on this crate's internals.
//! - **Hann window before every FFT** (invariant 6): [`fft::SpectrumAnalyzer`] is the only
//!   FFT entry point and always windows first ([`window::hann_window`]).
//! - **`sample_rate` travels with every chunk** (invariant 7): read from the CPAL device
//!   config in [`capture`], passed to [`analyzer::Analyzer::new`], and copied into every
//!   [`AudioFeatures`] — never hardcoded.
//! - **Zero allocation on the hot path** (invariant 3): [`analyzer::Analyzer::process_hop`]
//!   reuses preallocated buffers and `AudioFeatures` is `Copy` (fixed-size `spectrum`
//!   array), so the `watch` send doesn't allocate either. The CPAL callback
//!   ([`capture`]) and the [`ring_buffer::RingBuffer`] between it and the analysis thread
//!   are likewise allocation-free after setup.
//!
//! ## Pipeline parameters
//!
//! - FFT size [`contracts::FFT_SIZE`] = 1024, hop [`contracts::HOP_SIZE`] = 256 (75%
//!   overlap).
//! - Beat/onset detection: spectral flux with a slow EMA threshold,
//!   `flux_avg = flux_avg * 0.9 + flux * 0.1` ([`beat::BeatDetector`]).
//! - BPM: smoothed from beat-to-beat intervals ([`bpm::BpmTracker`]).
//!
//! ## Modules
//!
//! - [`contracts`] — the [`AudioFeatures`] / [`MusicalSection`] contract and pipeline
//!   constants.
//! - [`window`] — the Hann window.
//! - [`fft`] — `rustfft`-backed magnitude spectrum.
//! - [`bands`] — bin<->Hz, band energy, RMS/peak, spectral centroid/rolloff.
//! - [`beat`] — spectral-flux beat/onset detection.
//! - [`bpm`] — BPM tracking.
//! - [`ring_buffer`] — SPSC sample ring buffer (capture thread -> analysis thread).
//! - [`analyzer`] — ties the above into one frame -> [`AudioFeatures`].
//! - [`capture`] — CPAL input device capture.
//! - [`pipeline`] — [`AudioPipeline`]: capture + analysis thread + watch channel.

pub mod analyzer;
pub mod bands;
pub mod beat;
pub mod bpm;
pub mod capture;
pub mod contracts;
pub mod fft;
pub mod pipeline;
pub mod ring_buffer;
pub mod window;

pub use analyzer::Analyzer;
pub use capture::{AudioCoreError, CaptureStream};
pub use contracts::{AudioFeatures, MusicalSection, FFT_SIZE, HOP_SIZE, SPECTRUM_LEN};
pub use pipeline::AudioPipeline;
pub use ring_buffer::RingBuffer;
