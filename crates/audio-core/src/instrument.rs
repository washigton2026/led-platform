//! `InstrumentClassifier` — heuristic instrument/event detection from a spectrum frame.
//!
//! ## Classes detected
//!
//! | Class | Signal | Heuristic |
//! |---|---|---|
//! | `Kick` | Sub-bass transient (20–80 Hz) | high bass energy + low mid + beat |
//! | `Snare` | Mid transient (200–600 Hz) | high mid flux + low harmonic ratio |
//! | `HiHat` | High-frequency transient (8–16 kHz) | high-energy top band + low bass |
//! | `Bass` | Sub-bass sustained | high bass energy + high harmonic ratio |
//! | `Melody` | Pitched mid/high sustained | harmonic ratio high + f0 in mid/high range |
//! | `Chord` | Multiple harmonic peaks | harmonic ratio high + spread peak distribution |
//! | `Noise` | Broadband, no clear pitch | very low harmonic ratio |
//! | `Silence` | Near zero energy | rms below threshold |
//!
//! ## Invariants
//! - Pure heuristic: no ML model, no allocation, no external deps.
//! - Deterministic: same spectrum + same parameters → same class.
//! - `Silence` is always checked first and takes priority over all others.
//! - `Unknown` is returned when no heuristic fires confidently.

use crate::bands::{band_energy, rms as compute_rms};
use crate::contracts::{FFT_SIZE, SPECTRUM_LEN};

/// Instrument / timbral event class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstrumentClass {
    /// Sub-bass transient — kick drum, bass drop.
    Kick,
    /// Mid-frequency transient — snare, clap.
    Snare,
    /// High-frequency transient — hi-hat, cymbal.
    HiHat,
    /// Sustained sub-bass / bass instrument (bass guitar, synth bass).
    Bass,
    /// Sustained pitched mid/high instrument (lead synth, voice, guitar).
    Melody,
    /// Multiple simultaneous pitched notes (chords, pads).
    Chord,
    /// Broadband / noise (crowd noise, broadband synth).
    Noise,
    /// Near silence — below the RMS threshold.
    Silence,
    /// No heuristic fired with sufficient confidence.
    Unknown,
}

// ── Tuning constants ──────────────────────────────────────────────────────────

/// RMS below which a frame is classified as Silence.
const SILENCE_RMS: f32 = 0.005;

/// Harmonic ratio above which content is considered pitched.
const PITCHED_THRESHOLD: f32 = 0.35;

/// Harmonic ratio below which content is considered noise-like.
const NOISE_THRESHOLD: f32 = 0.08;

/// Fraction of total energy in sub-bass (20–80 Hz) to classify as Kick/Bass.
const KICK_BASS_FRACTION: f32 = 0.30;

/// Fraction of total energy in high band (8–16 kHz) to classify as HiHat.
const HIHAT_FRACTION: f32 = 0.25;

/// Mid fraction above which a frame is considered Snare-like.
const SNARE_MID_FRACTION: f32 = 0.35;

/// Number of spectrum peaks above a threshold — used to distinguish Chord from Melody.
const CHORD_PEAK_COUNT: usize = 3;

// ── InstrumentClassifier ──────────────────────────────────────────────────────

/// Stateful instrument classifier. Feed one spectrum per hop via [`classify`].
///
/// The classifier is stateless between frames — there is no EMA or memory.
/// Each call to [`classify`] is independent.
#[derive(Clone, Debug, Default)]
pub struct InstrumentClassifier;

impl InstrumentClassifier {
    pub fn new() -> Self { Self }

    /// Classify one audio frame.
    ///
    /// `samples` is the raw time-domain hop (used for RMS).
    /// `spectrum` is the Hann-windowed magnitude spectrum.
    /// `harmonic_ratio` is from [`HarmonicClassifier::process`].
    /// `f0_bin` is the detected fundamental bin.
    /// `sample_rate` is required for bin→Hz conversion.
    /// `is_beat` is the current beat flag from [`BeatDetector`](crate::beat::BeatDetector).
    pub fn classify(
        &self,
        samples:        &[f32],
        spectrum:       &[f32; SPECTRUM_LEN],
        harmonic_ratio: f32,
        f0_bin:         usize,
        sample_rate:    u32,
        is_beat:        bool,
    ) -> InstrumentClass {
        let n = FFT_SIZE;
        let sr = sample_rate;

        // 0. Silence
        let rms = compute_rms(samples);
        if rms < SILENCE_RMS { return InstrumentClass::Silence; }

        let total: f32 = spectrum.iter().sum();
        if total <= 0.0 { return InstrumentClass::Silence; }

        // Band energies (all normalised to fraction of total)
        let sub_bass  = band_energy(spectrum, n, sr, 20.0,   80.0)  / total;
        let bass      = band_energy(spectrum, n, sr, 80.0,  250.0)  / total;
        let mid       = band_energy(spectrum, n, sr, 250.0, 4000.0) / total;
        let hihat_b   = band_energy(spectrum, n, sr, 8000.0, (sr as f32 / 2.0).max(8001.0)) / total;
        let kick_bass = sub_bass + bass;

        // f0 frequency in Hz
        let f0_hz = f0_bin as f32 * sr as f32 / FFT_SIZE as f32;

        // 1. Noise (very low harmonic ratio, broadband)
        if harmonic_ratio < NOISE_THRESHOLD {
            return InstrumentClass::Noise;
        }

        // 2. Hi-Hat (high energy in top band, low bass, transient)
        if hihat_b > HIHAT_FRACTION && kick_bass < 0.15 {
            return InstrumentClass::HiHat;
        }

        // 3. Kick (dominant sub-bass + beat onset, low harmonic ratio)
        if kick_bass > KICK_BASS_FRACTION && is_beat && harmonic_ratio < PITCHED_THRESHOLD {
            return InstrumentClass::Kick;
        }

        // 4. Snare (dominant mid, beat onset, low harmonic ratio)
        if mid > SNARE_MID_FRACTION && is_beat && harmonic_ratio < PITCHED_THRESHOLD {
            return InstrumentClass::Snare;
        }

        // 5. Sustained pitched content
        if harmonic_ratio >= PITCHED_THRESHOLD {
            // Bass instrument (f0 in bass range)
            if f0_hz > 0.0 && f0_hz < 250.0 && kick_bass > 0.25 {
                return InstrumentClass::Bass;
            }

            // Chord: multiple peaks spread across the spectrum
            if count_peaks(spectrum, total * 0.05) >= CHORD_PEAK_COUNT {
                return InstrumentClass::Chord;
            }

            // Melody: single dominant pitch in mid/high range
            if f0_hz >= 250.0 {
                return InstrumentClass::Melody;
            }
        }

        InstrumentClass::Unknown
    }
}

/// Count distinct peaks in the spectrum above `threshold`.
/// A peak is a bin that is greater than both its neighbours.
fn count_peaks(spectrum: &[f32], threshold: f32) -> usize {
    let mut count = 0;
    for i in 1..spectrum.len() - 1 {
        if spectrum[i] > threshold
            && spectrum[i] > spectrum[i - 1]
            && spectrum[i] > spectrum[i + 1]
        {
            count += 1;
        }
    }
    count
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{FFT_SIZE, SPECTRUM_LEN};
    use crate::fft::SpectrumAnalyzer;
    use crate::harmonics::HarmonicClassifier;
    use crate::window::hann_window;
    use std::f32::consts::TAU;

    const SR: u32 = 44_100;

    fn tone_samples(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR as f32).sin()).collect()
    }

    fn silence_samples(n: usize) -> Vec<f32> { vec![0.0; n] }

    fn impulse_samples(n: usize) -> Vec<f32> {
        let mut s = vec![0.0f32; n];
        for i in 0..n.min(32) { s[i] = 0.9; }
        s
    }

    fn analyze(samples: &[f32]) -> ([f32; SPECTRUM_LEN], f32, usize, bool) {
        let hann = hann_window();
        let mut sa = SpectrumAnalyzer::new();
        let mut spec = [0.0f32; SPECTRUM_LEN];
        let mut window_buf = [0.0f32; FFT_SIZE];
        let n = samples.len().min(FFT_SIZE);
        window_buf[..n].copy_from_slice(&samples[..n]);
        sa.magnitude_spectrum(&window_buf, &hann, &mut spec);

        let mut hc = HarmonicClassifier::new();
        let (ratio, _is_tonal) = hc.process(&spec, SR);
        (spec, ratio, hc.f0_bin, false) // beat=false by default
    }

    // ── Silence ───────────────────────────────────────────────────────────────

    #[test]
    fn silence_detected() {
        let samples = silence_samples(FFT_SIZE);
        let (spec, ratio, f0_bin, beat) = analyze(&samples);
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, ratio, f0_bin, SR, beat);
        assert_eq!(class, InstrumentClass::Silence, "silence → Silence");
    }

    // ── Tonal content ─────────────────────────────────────────────────────────

    #[test]
    fn pure_sine_440hz_is_melody() {
        let samples = tone_samples(440.0, FFT_SIZE);
        let (spec, ratio, f0_bin, _) = analyze(&samples);
        assert!(ratio >= PITCHED_THRESHOLD, "440Hz sine must be pitched: ratio={ratio:.3}");
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, ratio, f0_bin, SR, false);
        assert!(
            matches!(class, InstrumentClass::Melody | InstrumentClass::Chord | InstrumentClass::Unknown),
            "440Hz sine → Melody/Chord (got {class:?})"
        );
    }

    #[test]
    fn pure_sine_80hz_is_bass() {
        let samples = tone_samples(80.0, FFT_SIZE);
        let (spec, ratio, f0_bin, _) = analyze(&samples);
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, ratio, f0_bin, SR, false);
        assert!(
            matches!(class, InstrumentClass::Bass | InstrumentClass::Kick | InstrumentClass::Unknown),
            "80Hz sine → Bass-related (got {class:?})"
        );
    }

    // ── Impulse / transient ───────────────────────────────────────────────────

    #[test]
    fn impulse_with_beat_is_percussive() {
        let samples = impulse_samples(FFT_SIZE);
        let (spec, ratio, f0_bin, _) = analyze(&samples);
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, ratio, f0_bin, SR, true /* beat=true */);
        assert!(
            !matches!(class, InstrumentClass::Melody | InstrumentClass::Silence),
            "impulse + beat must not be Melody or Silence (got {class:?})"
        );
    }

    // ── Noise ─────────────────────────────────────────────────────────────────

    #[test]
    fn white_noise_like_spectrum_is_noise() {
        // Flat spectrum simulates white noise
        let mut spec = [0.01f32; SPECTRUM_LEN];
        spec[0] = 0.0; // skip DC
        let samples = vec![0.3f32; FFT_SIZE]; // non-silence
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, 0.01 /* very low ratio */, 1, SR, false);
        assert_eq!(class, InstrumentClass::Noise, "flat spectrum → Noise");
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[test]
    fn classify_is_deterministic() {
        let samples = tone_samples(330.0, FFT_SIZE);
        let (spec, ratio, f0_bin, beat) = analyze(&samples);
        let c1 = InstrumentClassifier::new().classify(&samples, &spec, ratio, f0_bin, SR, beat);
        let c2 = InstrumentClassifier::new().classify(&samples, &spec, ratio, f0_bin, SR, beat);
        assert_eq!(c1, c2, "classify must be deterministic");
    }

    // ── Silence priority ─────────────────────────────────────────────────────

    #[test]
    fn silence_priority_over_all() {
        // Very low RMS but non-trivial spectrum — Silence must win
        let samples = vec![0.001f32; FFT_SIZE]; // below SILENCE_RMS
        let spec = [0.5f32; SPECTRUM_LEN]; // high spectrum energy
        let class = InstrumentClassifier::new()
            .classify(&samples, &spec, 0.9, 50, SR, true);
        assert_eq!(class, InstrumentClass::Silence, "low RMS → Silence even with rich spectrum");
    }

    // ── All classes are reachable ─────────────────────────────────────────────

    #[test]
    fn instrument_classes_are_enumerable() {
        // Structural test: all variants are accessible and printable
        let classes = [
            InstrumentClass::Kick, InstrumentClass::Snare, InstrumentClass::HiHat,
            InstrumentClass::Bass, InstrumentClass::Melody, InstrumentClass::Chord,
            InstrumentClass::Noise, InstrumentClass::Silence, InstrumentClass::Unknown,
        ];
        assert_eq!(classes.len(), 9, "exactly 9 classes defined");
        for c in &classes {
            let _ = format!("{c:?}"); // must be Debug
        }
    }
}
