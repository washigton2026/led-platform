//! Minimal complex radix-2 FFT + the Hann window. **Every analysis FFT goes through a Hann
//! window first** — that is what [`magnitude_spectrum`] enforces. Without it, a single tone
//! leaks across hundreds of bins and all downstream analysis is garbage.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
    #[inline]
    fn norm_sq(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
}

/// A Hann window of length `n`: `0.5 - 0.5·cos(2πi/(n-1))`. Zero at both ends, 1 at centre.
pub fn hann(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos()).collect()
}

/// In-place iterative Cooley–Tukey FFT. `a.len()` must be a power of two.
pub fn fft(a: &mut [Complex]) {
    let n = a.len();
    assert!(n.is_power_of_two(), "FFT length must be a power of two");
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            a.swap(i, j);
        }
    }

    // Butterflies.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32; // forward transform
        let wlen = Complex::new(ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let mut w = Complex::new(1.0, 0.0);
            for k in 0..len / 2 {
                let u = a[i + k];
                let v = a[i + k + len / 2].mul(w);
                a[i + k] = u.add(v);
                a[i + k + len / 2] = u.sub(v);
                w = w.mul(wlen);
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Hann-window `samples`, FFT, and return the magnitude of the first `n/2` bins.
/// This is the ONLY analysis entry point, so the Hann window can never be skipped.
pub fn magnitude_spectrum(samples: &[f32], window: &[f32]) -> Vec<f32> {
    let n = samples.len();
    assert_eq!(n, window.len(), "window must match frame length");
    let mut buf: Vec<Complex> =
        samples.iter().zip(window).map(|(s, w)| Complex::new(s * w, 0.0)).collect();
    fft(&mut buf);
    buf[..n / 2].iter().map(|c| c.norm_sq().sqrt()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_is_zero_at_ends_and_symmetric() {
        let w = hann(8);
        assert!(w[0].abs() < 1e-6);
        assert!(w[7].abs() < 1e-6);
        // symmetric
        for i in 0..4 {
            assert!((w[i] - w[7 - i]).abs() < 1e-6);
        }
        // peak in the middle
        assert!(w[3] > 0.9 && w[4] > 0.9);
    }

    #[test]
    fn fft_peaks_at_the_tone_bin() {
        let n = 1024usize;
        let bin = 64usize;
        let samples: Vec<f32> =
            (0..n).map(|i| (2.0 * PI * bin as f32 * i as f32 / n as f32).sin()).collect();
        let spec = magnitude_spectrum(&samples, &hann(n));
        let argmax = spec
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(argmax, bin, "a tone at bin {bin} must peak at bin {bin}");
    }
}
