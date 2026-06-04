//! Frequency helpers: bin↔Hz (always with the chunk's real `sample_rate` — never a
//! hardcoded 44100), band energy, and RMS.

/// Centre frequency of an FFT bin, in Hz.
pub fn bin_to_hz(bin: usize, n: usize, sample_rate: u32) -> f32 {
    bin as f32 * sample_rate as f32 / n as f32
}

/// Nearest FFT bin for a frequency, given the frame size and sample rate.
pub fn hz_to_bin(hz: f32, n: usize, sample_rate: u32) -> usize {
    (hz * n as f32 / sample_rate as f32).round().max(0.0) as usize
}

/// Summed magnitude in `[lo, hi)` Hz. `n` is the FFT (frame) size, `sr` the sample rate.
pub fn band_energy(spectrum: &[f32], n: usize, sr: u32, lo: f32, hi: f32) -> f32 {
    let b0 = hz_to_bin(lo, n, sr).min(spectrum.len());
    let b1 = hz_to_bin(hi, n, sr).min(spectrum.len());
    spectrum[b0..b1].iter().sum()
}

/// Root-mean-square level of the raw samples.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::{hann, magnitude_spectrum};
    use std::f32::consts::PI;

    fn tone(n: usize, freq_hz: f32, sr: u32) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI * freq_hz * i as f32 / sr as f32).sin()).collect()
    }

    fn argmax(s: &[f32]) -> usize {
        s.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0
    }

    #[test]
    fn sample_rate_is_explicit_not_hardcoded() {
        // The SAME samples (peak at bin 64 of a 1024-pt FFT) map to different Hz depending
        // on the sample rate that travels with the chunk.
        let n = 1024;
        assert!((bin_to_hz(64, n, 1024) - 64.0).abs() < 1e-3);
        assert!((bin_to_hz(64, n, 48_000) - 3000.0).abs() < 1e-1);
        assert_ne!(hz_to_bin(3000.0, n, 1024), hz_to_bin(3000.0, n, 48_000));
    }

    #[test]
    fn hann_reduces_spectral_leakage() {
        // A tone BETWEEN bins (64.5) leaks badly with no window; Hann concentrates it.
        let n = 1024;
        let sr = 1024;
        let samples = tone(n, 64.5, sr);
        let rect = magnitude_spectrum(&samples, &vec![1.0; n]); // rectangular = no window
        let hannd = magnitude_spectrum(&samples, &hann(n));

        let concentration = |s: &[f32]| {
            let pk = argmax(s);
            let lo = pk.saturating_sub(3);
            let hi = (pk + 4).min(s.len());
            let near: f32 = s[lo..hi].iter().sum();
            let total: f32 = s.iter().sum();
            near / total
        };
        let c_rect = concentration(&rect);
        let c_hann = concentration(&hannd);
        assert!(c_hann > c_rect, "Hann must concentrate energy (rect {c_rect:.3} vs hann {c_hann:.3})");
    }

    #[test]
    fn band_energy_tracks_the_tone() {
        let n = 2048;
        let sr = 16_000; // Nyquist 8 kHz
        let win = hann(n);

        let bass = magnitude_spectrum(&tone(n, 100.0, sr), &win);
        let b_bass = band_energy(&bass, n, sr, 20.0, 250.0);
        let b_high = band_energy(&bass, n, sr, 4000.0, 8000.0);
        assert!(b_bass > b_high * 10.0, "100 Hz tone lives in the bass band");

        let high = magnitude_spectrum(&tone(n, 5000.0, sr), &win);
        let h_bass = band_energy(&high, n, sr, 20.0, 250.0);
        let h_high = band_energy(&high, n, sr, 4000.0, 8000.0);
        assert!(h_high > h_bass * 10.0, "5 kHz tone lives in the high band");
    }
}
