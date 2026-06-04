//! Color math: HSV→RGB for effects, a gamma table and brightness scaling for output.
//! Gamma correction matters because LED brightness is perceptually non-linear; applying it
//! at the output stage keeps effects working in clean linear-ish color.

use led_core::PixelColor;

/// HSV (each 0..1, hue wraps) → 8-bit RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> PixelColor {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let to_u8 = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    PixelColor::rgb(to_u8(r), to_u8(g), to_u8(b))
}

/// Scale a color by a 0..1 brightness factor.
pub fn scale(c: PixelColor, brightness: f32) -> PixelColor {
    let f = brightness.clamp(0.0, 1.0);
    let m = |x: u8| (x as f32 * f + 0.5) as u8;
    PixelColor::rgb(m(c.r), m(c.g), m(c.b))
}

/// A precomputed gamma lookup table, applied at the output edge.
pub struct Gamma {
    lut: [u8; 256],
}

impl Gamma {
    pub fn new(gamma: f32) -> Self {
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = ((i as f32 / 255.0).powf(gamma) * 255.0 + 0.5) as u8;
        }
        Self { lut }
    }

    #[inline]
    pub fn apply(&self, c: PixelColor) -> PixelColor {
        PixelColor::rgb(self.lut[c.r as usize], self.lut[c.g as usize], self.lut[c.b as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_primaries() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), PixelColor::rgb(255, 0, 0));
        assert_eq!(hsv_to_rgb(1.0 / 3.0, 1.0, 1.0), PixelColor::rgb(0, 255, 0));
        assert_eq!(hsv_to_rgb(2.0 / 3.0, 1.0, 1.0), PixelColor::rgb(0, 0, 255));
        // value 0 is black regardless of hue
        assert_eq!(hsv_to_rgb(0.42, 1.0, 0.0), PixelColor::rgb(0, 0, 0));
    }

    #[test]
    fn gamma_endpoints_and_monotonic() {
        let g = Gamma::new(2.2);
        assert_eq!(g.apply(PixelColor::rgb(0, 0, 0)), PixelColor::rgb(0, 0, 0));
        assert_eq!(g.apply(PixelColor::rgb(255, 255, 255)), PixelColor::rgb(255, 255, 255));
        // monotonic non-decreasing, and darkens midtones (gamma > 1)
        let mut prev = 0u8;
        for i in 0..=255u8 {
            let v = g.apply(PixelColor::rgb(i, 0, 0)).r;
            assert!(v >= prev, "gamma must be monotonic");
            prev = v;
        }
        assert!(g.apply(PixelColor::rgb(128, 0, 0)).r < 128, "midtone darkened by gamma 2.2");
    }

    #[test]
    fn brightness_scale() {
        assert_eq!(scale(PixelColor::rgb(200, 100, 50), 0.0), PixelColor::rgb(0, 0, 0));
        assert_eq!(scale(PixelColor::rgb(200, 100, 50), 1.0), PixelColor::rgb(200, 100, 50));
        assert_eq!(scale(PixelColor::rgb(200, 100, 50), 0.5), PixelColor::rgb(100, 50, 25));
    }
}
