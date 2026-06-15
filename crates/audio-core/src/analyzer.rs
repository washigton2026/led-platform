//! [`Analyzer`] ties the DSP pipeline together: a sliding [`FFT_SIZE`]-sample window
//! (advanced [`HOP_SIZE`] samples at a time, i.e. 75% overlap) -> Hann window -> FFT ->
//! bands/centroid/rolloff/flux/beat/bpm -> [`AudioFeatures`].
//!
//! Every buffer (`window_buf`, `hann`, the [`SpectrumAnalyzer`]'s internal buffers, the
//! [`BeatDetector`]'s previous-spectrum array, the [`BpmTracker`]'s interval ring) is
//! allocated once in [`Analyzer::new`]. [`Analyzer::process_hop`] only shifts/overwrites
//! these buffers and returns a `Copy` [`AudioFeatures`] — no allocation (invariant 3).

use crate::bands::{band_energy, peak, rms, spectral_centroid, spectral_rolloff};
use crate::beat::BeatDetector;
use crate::bpm::BpmTracker;
use crate::contracts::{AudioFeatures, FFT_SIZE, HOP_SIZE, SPECTRUM_LEN};
use crate::fft::SpectrumAnalyzer;
use crate::harmonics::HarmonicClassifier;
use crate::window::hann_window;

/// Spectral rolloff threshold (data-contracts.md: "frequency below which 85% of energy
/// falls").
const ROLLOFF_THRESHOLD: f32 = 0.85;

pub struct Analyzer {
    sample_rate: u32,
    /// Sliding window of the last [`FFT_SIZE`] samples; shifted left by [`HOP_SIZE`] and
    /// refilled on each [`Analyzer::process_hop`].
    window_buf: [f32; FFT_SIZE],
    hann: [f32; FFT_SIZE],
    spectrum_analyzer: SpectrumAnalyzer,
    beat: BeatDetector,
    bpm: BpmTracker,
    /// Harmonic content classifier — runs on every spectrum, gates beat false-positives
    /// in tonal (sustained) frames.
    harmonic: HarmonicClassifier,
}

impl Analyzer {
    /// `sample_rate` must be the real device rate (44100 | 48000 | 88200 | 96000 | ...) —
    /// never hardcoded (invariant 7).
    pub fn new(sample_rate: u32) -> Self {
        assert!(sample_rate > 0, "sample_rate must be explicit and non-zero");
        Self {
            sample_rate,
            window_buf: [0.0; FFT_SIZE],
            hann: hann_window(),
            spectrum_analyzer: SpectrumAnalyzer::new(),
            beat: BeatDetector::new(),
            bpm: BpmTracker::new(),
            harmonic: HarmonicClassifier::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Feed one hop ([`HOP_SIZE`] new samples) and produce updated [`AudioFeatures`].
    ///
    /// Slides the [`FFT_SIZE`]-sample analysis window forward by `HOP_SIZE` (75% overlap
    /// with the previous call), re-runs the FFT, and updates beat/BPM state. `timestamp_ms`
    /// is the wall-clock time of the *end* of `hop`.
    pub fn process_hop(&mut self, hop: &[f32; HOP_SIZE], timestamp_ms: u64) -> AudioFeatures {
        self.window_buf.copy_within(HOP_SIZE.., 0);
        self.window_buf[FFT_SIZE - HOP_SIZE..].copy_from_slice(hop);

        let mut spectrum = [0.0f32; SPECTRUM_LEN];
        self.spectrum_analyzer.magnitude_spectrum(&self.window_buf, &self.hann, &mut spectrum);

        let sr = self.sample_rate;
        let n = FFT_SIZE;
        let nyquist = sr as f32 / 2.0;

        let bass_energy = band_energy(&spectrum, n, sr, 20.0, 250.0);
        let mid_energy = band_energy(&spectrum, n, sr, 250.0, 4000.0);
        let high_energy = band_energy(&spectrum, n, sr, 4000.0, nyquist.max(4000.0 + 1.0));
        let spectral_centroid = spectral_centroid(&spectrum, n, sr);
        let spectral_rolloff = spectral_rolloff(&spectrum, n, sr, ROLLOFF_THRESHOLD);

        let (beat_raw, onset, spectral_flux) = self.beat.process(&spectrum);

        // Harmonic gating (Cycle 7): if the frame is strongly tonal (sustained instrument),
        // suppress the beat signal. Windowing artifacts on non-integer bins create flux
        // spikes in tonal frames that look like beats but aren't musical onsets.
        // Gate threshold: harmonic_ratio >= TONAL_GATE_MIN suppresses beat.
        let (harmonic_ratio, is_tonal) = self.harmonic.process(&spectrum, sr);
        // Gate only very clean sustained tones (pure sine ≈ 0.9).
        // Transients on top of a tone dilute the ratio to ~0.5-0.7 — must not be gated.
        const TONAL_GATE_MIN: f32 = 0.80;
        let beat = beat_raw && !(is_tonal && harmonic_ratio >= TONAL_GATE_MIN);

        let bpm = self.bpm.update(beat, timestamp_ms);

        AudioFeatures {
            timestamp_ms,
            sample_rate: sr,
            rms: rms(hop),
            peak: peak(hop),
            beat,
            onset,
            bpm,
            bass_energy,
            mid_energy,
            high_energy,
            spectral_centroid,
            spectral_rolloff,
            spectral_flux,
            harmonic_ratio,
            spectrum,
            musical_section: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn audio_features_carry_their_sample_rate_and_correct_spectrum_len() {
        let sr = 48_000;
        let mut a = Analyzer::new(sr);

        // Feed enough hops to fill the FFT_SIZE window with a 1 kHz tone.
        let mut features = AudioFeatures::default();
        let mut t = 0u64;
        let mut sample_idx = 0usize;
        for _ in 0..(FFT_SIZE / HOP_SIZE) {
            let hop: [f32; HOP_SIZE] = std::array::from_fn(|i| {
                let s = sample_idx + i;
                (2.0 * PI * 1000.0 * s as f32 / sr as f32).sin()
            });
            sample_idx += HOP_SIZE;
            t += 1;
            features = a.process_hop(&hop, t);
        }

        assert_eq!(features.sample_rate, sr, "sample_rate travels with the features");
        assert_eq!(features.spectrum.len(), SPECTRUM_LEN);
        assert!(features.rms > 0.0);
        // 1 kHz at 48 kHz is in the mid band.
        assert!(features.mid_energy > features.bass_energy);
        assert!(features.mid_energy > features.high_energy);
        assert!(features.musical_section.is_none(), "realtime pipeline never sets musical_section");
    }

    #[test]
    fn beat_burst_is_detected_across_hops() {
        let sr = 44_100;
        let mut a = Analyzer::new(sr);

        // Warm up on silence.
        for t in 0..8u64 {
            a.process_hop(&[0.0; HOP_SIZE], t);
        }

        // A sudden loud hop should register a beat/onset.
        let loud = [0.8f32; HOP_SIZE];
        let f = a.process_hop(&loud, 100);
        assert!(f.onset, "a sudden energy burst must register as an onset");
    }
}

#[cfg(test)]
mod harmonic_gating_tests {
    use super::*;
    use crate::contracts::{HOP_SIZE, FFT_SIZE};
    use std::f32::consts::TAU;

    fn make_hop_sine(hz: f32, sr: u32, offset: usize) -> [f32; HOP_SIZE] {
        std::array::from_fn(|i| {
            (TAU * hz * (offset + i) as f32 / sr as f32).sin() * 0.8
        })
    }

    fn make_hop_impulse() -> [f32; HOP_SIZE] {
        let mut hop = [0.0f32; HOP_SIZE];
        hop[0] = 1.0; // broadband click
        hop
    }

    fn warmup(a: &mut Analyzer, hz: f32, sr: u32, hops: usize) {
        for i in 0..hops {
            let hop = make_hop_sine(hz, sr, i * HOP_SIZE);
            a.process_hop(&hop, i as u64 * 5);
        }
    }

    // ── GATING: sustained sine → beat suppressed in tonal frames ──────────
    #[test]
    fn harmonic_gating_suppresses_beats_on_sustained_sine() {
        let sr = 48_000u32;
        let mut a = Analyzer::new(sr);
        // Warm up: fill the window and EMA with a steady 440Hz sine
        warmup(&mut a, 440.0, sr, FFT_SIZE / HOP_SIZE + 10);

        // Now run 50 more hops of sustained sine — no beat impulses
        let mut beats_after_warmup = 0u32;
        for i in 0..50usize {
            let hop = make_hop_sine(440.0, sr, (FFT_SIZE / HOP_SIZE + 10 + i) * HOP_SIZE);
            let af = a.process_hop(&hop, (i as u64 + 10) * 5);
            if af.beat { beats_after_warmup += 1; }
        }
        // With harmonic gating, tonal frames should fire 0 beats (or very few)
        assert!(beats_after_warmup <= 2,
            "harmonic gating must suppress sine beats; got {beats_after_warmup} in 50 hops");
    }

    // ── GATING: real beat impulse on silence still fires ──────────────────
    #[test]
    fn harmonic_gating_does_not_suppress_real_impulse() {
        let sr = 48_000u32;
        let mut a = Analyzer::new(sr);
        // Warm up on silence
        for i in 0..10 {
            a.process_hop(&[0.0; HOP_SIZE], i * 5);
        }
        // Broadband impulse — not tonal → should fire
        let hop = make_hop_impulse();
        let af = a.process_hop(&hop, 50);
        assert!(af.beat, "broadband impulse must not be gated (harmonic_ratio should be low)");
    }

    // ── CONTRACT: harmonic_ratio in AudioFeatures is populated ───────────
    #[test]
    fn audio_features_contains_harmonic_ratio() {
        let sr = 48_000u32;
        let mut a = Analyzer::new(sr);
        // Feed a sine hop (multiple hops to fill window)
        for i in 0..(FFT_SIZE / HOP_SIZE) {
            let hop = make_hop_sine(440.0, sr, i * HOP_SIZE);
            let af = a.process_hop(&hop, i as u64 * 5);
            let _ = af.harmonic_ratio; // field must exist
        }
        let hop = make_hop_sine(440.0, sr, FFT_SIZE);
        let af = a.process_hop(&hop, 100);
        assert!(af.harmonic_ratio >= 0.0 && af.harmonic_ratio <= 1.0,
            "harmonic_ratio must be in [0,1]: {}", af.harmonic_ratio);
    }

    // ── CONTRACT: silence → harmonic_ratio = 0 ───────────────────────────
    #[test]
    fn silence_gives_zero_harmonic_ratio() {
        let mut a = Analyzer::new(48_000);
        let af = a.process_hop(&[0.0; HOP_SIZE], 0);
        assert_eq!(af.harmonic_ratio, 0.0, "silence must give harmonic_ratio=0");
    }

    // ── CONTRACT: tonal sine has higher harmonic_ratio than broad noise ──
    // NOTE: "noise" here means a multi-tone signal with energy spread across
    // many incoherent frequencies — not alternating ±1 (which is a square wave
    // at Nyquist, itself highly harmonic). We use 3 incommensurate tones that
    // create a rich, non-periodic spectrum.
    #[test]
    fn sine_has_higher_harmonic_ratio_than_noise() {
        let sr = 48_000u32;
        let mut a = Analyzer::new(sr);
        // Warm up with a 440Hz sine to fill the analysis window
        for i in 0..(FFT_SIZE / HOP_SIZE + 2) {
            let hop = make_hop_sine(440.0, sr, i * HOP_SIZE);
            a.process_hop(&hop, i as u64 * 5);
        }
        let sine_hop = make_hop_sine(440.0, sr, (FFT_SIZE / HOP_SIZE + 2) * HOP_SIZE);
        let af_sine = a.process_hop(&sine_hop, 100);

        // Multi-tone "noise": 7 incommensurate frequencies spread across the spectrum
        let noise_freqs = [137.0f32, 523.0, 1117.0, 2333.0, 4721.0, 9001.0, 17123.0];
        let mut a2 = Analyzer::new(sr);
        for i in 0..(FFT_SIZE / HOP_SIZE + 2) {
            let hop: [f32; HOP_SIZE] = std::array::from_fn(|j| {
                let s = (i * HOP_SIZE + j) as f32;
                noise_freqs.iter().map(|&f| (TAU * f * s / sr as f32).sin() * 0.14).sum::<f32>()
            });
            a2.process_hop(&hop, i as u64 * 5);
        }
        let noise_hop: [f32; HOP_SIZE] = std::array::from_fn(|j| {
            let s = ((FFT_SIZE / HOP_SIZE + 2) * HOP_SIZE + j) as f32;
            noise_freqs.iter().map(|&f| (TAU * f * s / sr as f32).sin() * 0.14).sum::<f32>()
        });
        let af_noise = a2.process_hop(&noise_hop, 100);

        assert!(af_sine.harmonic_ratio > af_noise.harmonic_ratio,
            "pure 440Hz sine ({:.3}) must have higher harmonic_ratio than multi-tone ({:.3})",
            af_sine.harmonic_ratio, af_noise.harmonic_ratio);
    }
}
