//! Harmonic content analysis — detects whether a spectrum is tonal (pitched instrument,
//! sustain) or transient (percussion, attack).
//!
//! ## Why this exists
//!
//! The spectral-flux [`BeatDetector`](crate::beat::BeatDetector) fires on *any* large
//! spectral change — including the windowing artifacts of a sustained tone (non-integer
//! FFT bin, 75% overlap). The `HarmonicClassifier` provides a second signal:
//!
//! - **Tonal** (harmonic ratio > threshold): sustained pitched content — BeatDetector
//!   false-positives should be suppressed or discounted.
//! - **Transient**: percussive, broadband — BeatDetector fires are more reliable.
//!
//! ## Algorithm
//!
//! 1. Find the peak bin (`f0` candidate).
//! 2. Sum energy at the peak and its first N harmonics (2f0, 3f0, … Nf0), each ±1 bin.
//! 3. Divide by total spectral energy → **harmonic ratio** ∈ [0, 1].
//!
//! A ratio above `TONAL_THRESHOLD` (0.40 by default) indicates tonal content.
//! The threshold is tuned so that a pure sine scores ≈ 0.9 and white noise scores ≈ 0.01.
//!
//! ## No allocation
//!
//! The classifier holds only two `f32` scalars. All computation works on fixed-size
//! spectrum slices — zero allocation, safe on the hot path.

use crate::contracts::{FFT_SIZE, SPECTRUM_LEN};

/// Harmonic ratio above which a spectrum is considered tonal.
pub const TONAL_THRESHOLD: f32 = 0.40;

/// Number of harmonics to sum (fundamental + 4 overtones = 5 peaks total).
const HARMONIC_COUNT: usize = 5;

/// Tolerance window around each harmonic bin (±BIN_WINDOW bins).
const BIN_WINDOW: usize = 1;

/// Determines whether a spectrum is tonal (pitched) or transient (percussive).
///
/// Holds the `harmonic_ratio` of the last processed frame and the `f0_bin` (fundamental
/// frequency bin). Both are reset to 0.0 / 0 on construction.
#[derive(Clone, Debug, Default)]
pub struct HarmonicClassifier {
    /// Harmonic energy ratio of the last frame (0.0 = pure noise, 1.0 = pure sine).
    pub harmonic_ratio: f32,
    /// FFT bin of the detected fundamental (`f0`). 0 if no frame processed yet.
    pub f0_bin: usize,
}

impl HarmonicClassifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one magnitude spectrum and update `harmonic_ratio` + `f0_bin`.
    ///
    /// `sample_rate` is required to convert bins to Hz (invariant 7 — never hardcoded).
    /// Returns `(harmonic_ratio, is_tonal)`.
    pub fn process(&mut self, spectrum: &[f32; SPECTRUM_LEN], sample_rate: u32) -> (f32, bool) {
        let total: f32 = spectrum.iter().sum();
        // Guard: zero, NaN, or Inf total → treat as silence (no harmonic content).
        if !total.is_finite() || total <= 0.0 {
            self.harmonic_ratio = 0.0;
            self.f0_bin = 0;
            return (0.0, false);
        }

        // Find the peak bin (fundamental candidate), skipping DC (bin 0).
        let f0_bin = spectrum[1..]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i + 1)
            .unwrap_or(1);

        // Sum energy at f0 and its harmonics (2f0, 3f0, …).
        let mut harmonic_energy = 0.0f32;
        for k in 1..=HARMONIC_COUNT {
            let target_bin = f0_bin * k;
            if target_bin >= SPECTRUM_LEN {
                break;
            }
            let lo = target_bin.saturating_sub(BIN_WINDOW);
            let hi = (target_bin + BIN_WINDOW + 1).min(SPECTRUM_LEN);
            harmonic_energy += spectrum[lo..hi].iter().sum::<f32>();
        }

        let ratio = (harmonic_energy / total).clamp(0.0, 1.0);
        self.harmonic_ratio = ratio;
        self.f0_bin = f0_bin;
        (ratio, ratio >= TONAL_THRESHOLD)
    }

    /// Fundamental frequency in Hz, given the sample rate used during the last `process`.
    pub fn f0_hz(&self, sample_rate: u32) -> f32 {
        self.f0_bin as f32 * sample_rate as f32 / FFT_SIZE as f32
    }

    /// True if the last processed frame was tonal.
    pub fn is_tonal(&self) -> bool {
        self.harmonic_ratio >= TONAL_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Analyzer, MockCaptureSource, contracts::{FFT_SIZE, SPECTRUM_LEN}};
    use std::f32::consts::TAU;

    fn sine_spectrum(hz: f32, sr: u32) -> [f32; SPECTRUM_LEN] {
        // Generate a one-window sine and run it through the analyzer to get a real spectrum.
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (TAU * hz * i as f32 / sr as f32).sin() * 0.8)
            .collect();
        let mock = MockCaptureSource::new(sr, samples);
        let results = mock.analyze_all();
        results.last().map(|af| af.spectrum).unwrap_or([0.0; SPECTRUM_LEN])
    }

    fn noise_spectrum() -> [f32; SPECTRUM_LEN] {
        // Flat spectrum = white noise approximation.
        [0.01f32; SPECTRUM_LEN]
    }

    // ── CONTRACT: pure sine is classified as tonal ────────────────────────
    #[test]
    fn sine_is_tonal() {
        let mut hc = HarmonicClassifier::new();
        let spectrum = sine_spectrum(440.0, 48_000);
        let (ratio, tonal) = hc.process(&spectrum, 48_000);
        assert!(tonal, "440Hz sine must be tonal (ratio={ratio:.3})");
        assert!(ratio >= TONAL_THRESHOLD, "ratio must exceed {TONAL_THRESHOLD} (got {ratio:.3})");
    }

    // ── CONTRACT: flat noise spectrum is NOT tonal ────────────────────────
    #[test]
    fn flat_noise_is_not_tonal() {
        let mut hc = HarmonicClassifier::new();
        let spectrum = noise_spectrum();
        let (ratio, tonal) = hc.process(&spectrum, 48_000);
        assert!(!tonal, "flat noise must not be tonal (ratio={ratio:.3})");
        assert!(ratio < TONAL_THRESHOLD, "ratio must be below {TONAL_THRESHOLD} (got {ratio:.3})");
    }

    // ── CONTRACT: silence (zero spectrum) → ratio=0, not tonal ──────────
    #[test]
    fn silence_is_not_tonal() {
        let mut hc = HarmonicClassifier::new();
        let (ratio, tonal) = hc.process(&[0.0; SPECTRUM_LEN], 48_000);
        assert_eq!(ratio, 0.0);
        assert!(!tonal);
    }

    // ── CONTRACT: f0_hz returns frequency near the sine frequency ─────────
    #[test]
    fn f0_hz_near_sine_frequency() {
        let target_hz = 880.0f32;
        let sr = 48_000u32;
        let mut hc = HarmonicClassifier::new();
        let spectrum = sine_spectrum(target_hz, sr);
        hc.process(&spectrum, sr);
        let detected = hc.f0_hz(sr);
        // Allow ±2 bins tolerance (each bin ≈ 46.875 Hz at 48kHz/1024)
        let bin_width = sr as f32 / FFT_SIZE as f32;
        assert!((detected - target_hz).abs() < bin_width * 2.0,
            "f0_hz ({detected:.1}Hz) must be within 2 bins of {target_hz}Hz");
    }

    // ── CONTRACT: different tones both classified as tonal ────────────────
    #[test]
    fn multiple_tones_all_tonal() {
        let sr = 48_000u32;
        let mut hc = HarmonicClassifier::new();
        for hz in [110.0, 220.0, 440.0, 880.0, 1760.0f32] {
            let spectrum = sine_spectrum(hz, sr);
            let (ratio, tonal) = hc.process(&spectrum, sr);
            assert!(tonal, "{hz}Hz sine must be tonal (ratio={ratio:.3})");
        }
    }

    // ── CONTRACT: is_tonal() matches last process() result ───────────────
    #[test]
    fn is_tonal_matches_last_process() {
        let mut hc = HarmonicClassifier::new();
        let spectrum = sine_spectrum(440.0, 48_000);
        let (_, tonal) = hc.process(&spectrum, 48_000);
        assert_eq!(hc.is_tonal(), tonal, "is_tonal() must match last process() result");
    }

    // ── INTEGRATION: Analyzer + HarmonicClassifier on real MockCapture ───
    #[test]
    fn integration_analyzer_harmonic_classifier_on_mock() {
        let sr = 48_000u32;
        // 10 full FFT windows of 440Hz sine
        let samples: Vec<f32> = (0..FFT_SIZE * 10)
            .map(|i| (TAU * 440.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let mock = MockCaptureSource::new(sr, samples);
        let results = mock.analyze_all();

        let mut hc = HarmonicClassifier::new();
        let mut tonal_count = 0u32;
        for af in results.iter().skip(4) { // skip warmup
            let (_, tonal) = hc.process(&af.spectrum, sr);
            if tonal { tonal_count += 1; }
        }
        let total = (results.len().saturating_sub(4)) as u32;
        assert!(total > 0);
        let tonal_frac = tonal_count as f32 / total as f32;
        assert!(tonal_frac > 0.7,
            "steady 440Hz must be tonal in >70% of frames (got {tonal_frac:.2})");
    }

    // ── FUZZ: NaN in spectrum → no panic ──────────────────────────────────
    #[test]
    fn nan_spectrum_no_panic() {
        let mut hc = HarmonicClassifier::new();
        let mut spectrum = [0.0f32; SPECTRUM_LEN];
        spectrum[10] = f32::NAN;
        let _ = hc.process(&spectrum, 48_000); // must not panic
    }

    // ── FUZZ: Inf in spectrum → ratio clamped to [0,1] ───────────────────
    #[test]
    fn inf_spectrum_ratio_clamped() {
        let mut hc = HarmonicClassifier::new();
        let mut spectrum = [0.0f32; SPECTRUM_LEN];
        spectrum[5] = f32::INFINITY;
        let (ratio, _) = hc.process(&spectrum, 48_000);
        assert!(ratio >= 0.0 && ratio <= 1.0,
            "ratio must be clamped to [0,1] even with Inf spectrum (got {ratio})");
    }

    // ── STRESS: 10k spectra classified within wall-clock budget ──────────
    // Budget is relaxed for debug builds (no optimizations).
    #[test]
    fn classifier_10k_frames_fast() {
        use std::time::Instant;
        let mut hc = HarmonicClassifier::new();
        let spectrum = sine_spectrum(440.0, 48_000);
        let t0 = Instant::now();
        for _ in 0..10_000 {
            hc.process(&spectrum, 48_000);
        }
        let ms = t0.elapsed().as_millis();
        // Release: <50ms. Debug: allow up to 2000ms (unoptimized).
        let budget = if cfg!(debug_assertions) { 2_000 } else { 50 };
        assert!(ms < budget, "10k classifications must complete in <{budget}ms (took {ms}ms)");
    }
}
