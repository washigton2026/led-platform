//! The `Analyzer` ties it together: one frame of samples + a timestamp → [`AudioFeatures`].
//! It owns the Hann window (sized once) and the beat detector. `sample_rate` is supplied at
//! construction and travels out with every result.

use led_core::AudioFeatures;

use crate::bands::{band_energy, rms};
use crate::beat::BeatDetector;
use crate::fft::{hann, magnitude_spectrum};

pub struct Analyzer {
    sample_rate: u32,
    window: Vec<f32>,
    beat: BeatDetector,
}

impl Analyzer {
    /// `frame_size` must be a power of two (the FFT size).
    pub fn new(sample_rate: u32, frame_size: usize) -> Self {
        assert!(frame_size.is_power_of_two(), "frame_size must be a power of two");
        assert!(sample_rate > 0, "sample_rate must be explicit and non-zero");
        Self { sample_rate, window: hann(frame_size), beat: BeatDetector::new() }
    }

    pub fn frame_size(&self) -> usize {
        self.window.len()
    }

    /// Analyze one frame. `samples.len()` must equal `frame_size`.
    pub fn analyze(&mut self, samples: &[f32], timestamp_ms: u64) -> AudioFeatures {
        let n = samples.len();
        assert_eq!(n, self.window.len(), "frame length must equal frame_size");
        let sr = self.sample_rate;

        let spectrum = magnitude_spectrum(samples, &self.window);
        let bass = band_energy(&spectrum, n, sr, 20.0, 250.0);
        let mid = band_energy(&spectrum, n, sr, 250.0, 4000.0);
        let high = band_energy(&spectrum, n, sr, 4000.0, sr as f32 / 2.0);
        let beat = self.beat.process(&spectrum);

        AudioFeatures {
            sample_rate: sr,
            timestamp_ms,
            rms: rms(samples),
            beat,
            bass,
            mid,
            high,
            spectrum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn audio_features_carry_their_sample_rate() {
        let sr = 48_000;
        let n = 1024;
        let mut a = Analyzer::new(sr, n);
        let samples: Vec<f32> = (0..n).map(|i| (2.0 * PI * 1000.0 * i as f32 / sr as f32).sin()).collect();
        let f = a.analyze(&samples, 1234);
        assert_eq!(f.sample_rate, sr, "sample_rate travels with the features");
        assert_eq!(f.timestamp_ms, 1234);
        assert_eq!(f.spectrum.len(), n / 2);
        assert!(f.rms > 0.0);
        // a 1 kHz tone at 48 kHz is in the mid band
        assert!(f.mid > f.bass && f.mid > f.high, "1 kHz ⇒ mid-dominant");
    }
}
