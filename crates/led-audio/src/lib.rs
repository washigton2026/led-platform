//! # led-audio — hearing the music
//!
//! Turns sound into [`AudioFeatures`](led_core::AudioFeatures): a Hann-windowed FFT spectrum,
//! bass/mid/high band energy, RMS, and spectral-flux beat detection. Two invariants the
//! whole layer rests on (master §4):
//!
//! - **Hann window before every FFT** — enforced structurally: [`fft::magnitude_spectrum`]
//!   is the only analysis path and it always applies the window.
//! - **`sample_rate` is explicit** — supplied to the [`Analyzer`] and carried out with the
//!   features; bin↔Hz math ([`bands`]) uses it, never a hardcoded 44100.

pub mod analyzer;
pub mod bands;
pub mod beat;
pub mod fft;

pub use analyzer::Analyzer;
pub use bands::{band_energy, bin_to_hz, hz_to_bin, rms};
pub use beat::BeatDetector;
pub use fft::{fft, hann, magnitude_spectrum, Complex};
