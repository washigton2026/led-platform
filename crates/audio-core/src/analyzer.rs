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

        let (beat, onset, spectral_flux) = self.beat.process(&spectrum);
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
