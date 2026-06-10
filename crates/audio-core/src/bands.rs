//! Frequency helpers: bin<->Hz (always with the chunk's real `sample_rate` — never a
//! hardcoded 44100, invariant 7), band energy, RMS/peak amplitude, spectral centroid and
//! rolloff. All operate on fixed-size spectra/slices — no allocation.

use crate::contracts::SPECTRUM_LEN;

/// Centre frequency of an FFT bin, in Hz.
pub fn bin_to_hz(bin: usize, n: usize, sample_rate: u32) -> f32 {
    bin as f32 * sample_rate as f32 / n as f32
}

/// Nearest FFT bin for a frequency, given the frame size and sample rate.
pub fn hz_to_bin(hz: f32, n: usize, sample_rate: u32) -> usize {
    (hz * n as f32 / sample_rate as f32).round().max(0.0) as usize
}

/// Fraction of total spectral magnitude that falls in `[lo, hi)` Hz, normalized to
/// `0.0..=1.0`. `n` is the FFT (frame) size, `sr` the sample rate.
pub fn band_energy(spectrum: &[f32; SPECTRUM_LEN], n: usize, sr: u32, lo: f32, hi: f32) -> f32 {
    let b0 = hz_to_bin(lo, n, sr).min(spectrum.len());
    let b1 = hz_to_bin(hi, n, sr).min(spectrum.len()).max(b0);
    let total: f32 = spectrum.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let band: f32 = spectrum[b0..b1].iter().sum();
    (band / total).clamp(0.0, 1.0)
}

/// Root-mean-square level of the raw samples (0.0..=1.0 for normalized [-1, 1] audio).
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Peak absolute sample magnitude (0.0..=1.0 for normalized [-1, 1] audio).
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

/// Spectral centroid in Hz — the magnitude-weighted average frequency ("brightness").
pub fn spectral_centroid(spectrum: &[f32; SPECTRUM_LEN], n: usize, sr: u32) -> f32 {
    let mut weighted = 0.0f32;
    let mut total = 0.0f32;
    for (bin, &mag) in spectrum.iter().enumerate() {
        weighted += bin_to_hz(bin, n, sr) * mag;
        total += mag;
    }
    if total > 0.0 {
        weighted / total
    } else {
        0.0
    }
}

/// Spectral rolloff in Hz — the frequency below which `threshold` (e.g. 0.85) of the total
/// spectral magnitude falls.
pub fn spectral_rolloff(spectrum: &[f32; SPECTRUM_LEN], n: usize, sr: u32, threshold: f32) -> f32 {
    let total: f32 = spectrum.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let target = total * threshold;
    let mut cum = 0.0f32;
    for (bin, &mag) in spectrum.iter().enumerate() {
        cum += mag;
        if cum >= target {
            return bin_to_hz(bin, n, sr);
        }
    }
    bin_to_hz(spectrum.len() - 1, n, sr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::FFT_SIZE;
    use crate::fft::SpectrumAnalyzer;
    use crate::window::hann_window;
    use std::f32::consts::PI;

    fn tone(freq_hz: f32, sr: u32) -> [f32; FFT_SIZE] {
        std::array::from_fn(|i| (2.0 * PI * freq_hz * i as f32 / sr as f32).sin())
    }

    fn spectrum_of(freq_hz: f32, sr: u32) -> [f32; SPECTRUM_LEN] {
        let win = hann_window();
        let samples = tone(freq_hz, sr);
        let mut spec = [0.0f32; SPECTRUM_LEN];
        SpectrumAnalyzer::new().magnitude_spectrum(&samples, &win, &mut spec);
        spec
    }

    #[test]
    fn sample_rate_is_explicit_not_hardcoded() {
        let n = FFT_SIZE;
        assert!((bin_to_hz(64, n, n as u32) - 64.0).abs() < 1e-3);
        assert!((bin_to_hz(64, n, 48_000) - 3000.0).abs() < 1e-1);
        assert_ne!(hz_to_bin(3000.0, n, n as u32), hz_to_bin(3000.0, n, 48_000));
    }

    #[test]
    fn band_energy_tracks_the_tone() {
        let sr = 16_000; // Nyquist 8 kHz
        let n = FFT_SIZE;

        let bass = spectrum_of(100.0, sr);
        let b_bass = band_energy(&bass, n, sr, 20.0, 250.0);
        let b_high = band_energy(&bass, n, sr, 4000.0, 8000.0);
        assert!(b_bass > b_high * 10.0, "100 Hz tone lives in the bass band");

        let high = spectrum_of(5000.0, sr);
        let h_bass = band_energy(&high, n, sr, 20.0, 250.0);
        let h_high = band_energy(&high, n, sr, 4000.0, 8000.0);
        assert!(h_high > h_bass * 10.0, "5 kHz tone lives in the high band");
    }

    #[test]
    fn spectral_centroid_and_rolloff_track_brightness() {
        let sr = 16_000;
        let n = FFT_SIZE;
        let low = spectrum_of(100.0, sr);
        let high = spectrum_of(5000.0, sr);

        assert!(spectral_centroid(&low, n, sr) < spectral_centroid(&high, n, sr));
        assert!(spectral_rolloff(&low, n, sr, 0.85) < spectral_rolloff(&high, n, sr, 0.85));
    }

    #[test]
    fn rms_and_peak_of_unit_sine() {
        let samples = tone(1000.0, 48_000);
        let r = rms(&samples);
        let p = peak(&samples);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01, "rms of a unit sine ~= 1/sqrt(2), got {r}");
        assert!((p - 1.0).abs() < 0.01, "peak of a unit sine ~= 1.0, got {p}");
    }
}
