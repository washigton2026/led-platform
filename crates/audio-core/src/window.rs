//! The Hann window. Computed once at [`crate::analyzer::Analyzer::new`] time and reused for
//! every frame — invariant 6 (lumyx-system-architect §4): **Hann window before every FFT**.

use std::f32::consts::PI;

use crate::contracts::FFT_SIZE;

/// A Hann window of length [`FFT_SIZE`]: `0.5 - 0.5*cos(2*pi*i/(n-1))`. Zero at both ends,
/// 1.0 at the centre.
pub fn hann_window() -> [f32; FFT_SIZE] {
    let n = FFT_SIZE;
    std::array::from_fn(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_is_zero_at_ends_symmetric_and_peaks_at_one() {
        let w = hann_window();
        assert!(w[0].abs() < 1e-6);
        assert!(w[FFT_SIZE - 1].abs() < 1e-6);
        for i in 0..FFT_SIZE / 2 {
            assert!((w[i] - w[FFT_SIZE - 1 - i]).abs() < 1e-6, "window must be symmetric");
        }
        let mid = FFT_SIZE / 2;
        assert!(w[mid] > 0.99 && w[mid] <= 1.0);
    }
}
