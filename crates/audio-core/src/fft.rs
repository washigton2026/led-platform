//! `rustfft`-backed magnitude spectrum. [`SpectrumAnalyzer`] is the **only** FFT entry
//! point in this crate (invariant 6: Hann window before every FFT — `magnitude_spectrum`
//! takes the window as a required argument, so it can never be skipped). All buffers
//! (complex working buffer + FFT scratch) are allocated once in [`SpectrumAnalyzer::new`]
//! and reused on every call — no allocation on the hot path (invariant 3).

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

use crate::contracts::{FFT_SIZE, SPECTRUM_LEN};

pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    buffer: [Complex32; FFT_SIZE],
    scratch: Vec<Complex32>,
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            buffer: [Complex32::new(0.0, 0.0); FFT_SIZE],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
        }
    }

    /// Hann-window `samples` and compute the magnitude spectrum into `out`.
    ///
    /// `samples` and `window` are [`FFT_SIZE`] real-valued; `out` is the first
    /// [`SPECTRUM_LEN`] (= `FFT_SIZE / 2`) magnitude bins. Reuses `self.buffer` and
    /// `self.scratch` — no allocation.
    pub fn magnitude_spectrum(
        &mut self,
        samples: &[f32; FFT_SIZE],
        window: &[f32; FFT_SIZE],
        out: &mut [f32; SPECTRUM_LEN],
    ) {
        for i in 0..FFT_SIZE {
            self.buffer[i] = Complex32::new(samples[i] * window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.buffer, &mut self.scratch);
        for i in 0..SPECTRUM_LEN {
            out[i] = self.buffer[i].norm();
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::hann_window;
    use std::f32::consts::PI;

    #[test]
    fn fft_peaks_at_the_tone_bin() {
        let bin = 64usize;
        let samples: [f32; FFT_SIZE] =
            std::array::from_fn(|i| (2.0 * PI * bin as f32 * i as f32 / FFT_SIZE as f32).sin());
        let window = hann_window();
        let mut spec = [0.0f32; SPECTRUM_LEN];
        let mut analyzer = SpectrumAnalyzer::new();
        analyzer.magnitude_spectrum(&samples, &window, &mut spec);

        let argmax = spec
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(argmax, bin, "a tone at bin {bin} must peak at bin {bin}");
    }

    #[test]
    fn hann_reduces_spectral_leakage_vs_rectangular() {
        // A tone BETWEEN bins (64.5) leaks badly with no window; Hann concentrates it.
        let samples: [f32; FFT_SIZE] =
            std::array::from_fn(|i| (2.0 * PI * 64.5 * i as f32 / FFT_SIZE as f32).sin());
        let rect = [1.0f32; FFT_SIZE];
        let hann = hann_window();

        let mut spec_rect = [0.0f32; SPECTRUM_LEN];
        let mut spec_hann = [0.0f32; SPECTRUM_LEN];
        let mut analyzer = SpectrumAnalyzer::new();
        analyzer.magnitude_spectrum(&samples, &rect, &mut spec_rect);
        analyzer.magnitude_spectrum(&samples, &hann, &mut spec_hann);

        let concentration = |s: &[f32; SPECTRUM_LEN]| {
            let pk = s.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
            let lo = pk.saturating_sub(3);
            let hi = (pk + 4).min(s.len());
            let near: f32 = s[lo..hi].iter().sum();
            let total: f32 = s.iter().sum();
            near / total
        };
        assert!(concentration(&spec_hann) > concentration(&spec_rect));
    }
}
