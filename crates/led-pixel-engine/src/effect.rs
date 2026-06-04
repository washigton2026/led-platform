//! Effects turn (time, pixel positions) into per-pixel colors, in **logical space**. They
//! are pure functions of time + params — same time ⇒ same output — which is what makes the
//! render deterministic and testable.

use led_core::PixelColor;

use crate::color;

/// A logical-space position (metres). Spatial effects read these; uniform effects ignore them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// Renders one frame. `out.len() == positions.len()`. Must be allocation-free.
pub trait Effect: Send {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]);
}

/// A constant color on every pixel.
pub struct SolidColor(pub PixelColor);

impl Effect for SolidColor {
    fn render(&self, _time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        out.fill(self.0);
    }
}

/// A hue sweep across the rig's x-axis, scrolling over time.
pub struct Rainbow {
    /// Hue revolutions per second (temporal scroll).
    pub speed_hz: f32,
    /// Hue revolutions per metre along x (spatial spread).
    pub cycles_per_m: f32,
}

impl Effect for Rainbow {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        for (i, p) in positions.iter().enumerate() {
            let hue = t * self.speed_hz + p.x * self.cycles_per_m;
            out[i] = color::hsv_to_rgb(hue, 1.0, 1.0);
        }
    }
}

/// A whole-rig brightness pulse (sine) of a base color.
pub struct Pulse {
    pub color: PixelColor,
    pub hz: f32,
}

impl Effect for Pulse {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let phase = std::f32::consts::TAU * self.hz * t;
        let brightness = 0.5 * (1.0 + phase.sin()); // 0..1
        let c = color::scale(self.color, brightness);
        out.fill(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_fills_all() {
        let mut out = [PixelColor::default(); 4];
        SolidColor(PixelColor::rgb(1, 2, 3)).render(123, &[Vec3::ZERO; 4], &mut out);
        assert!(out.iter().all(|&c| c == PixelColor::rgb(1, 2, 3)));
    }

    #[test]
    fn rainbow_is_deterministic_in_time() {
        let positions: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32 * 0.1, 0.0, 0.0)).collect();
        let fx = Rainbow { speed_hz: 0.5, cycles_per_m: 1.0 };
        let mut a = [PixelColor::default(); 8];
        let mut b = [PixelColor::default(); 8];
        fx.render(777, &positions, &mut a);
        fx.render(777, &positions, &mut b);
        assert_eq!(a, b, "same time ⇒ same frame");
    }

    #[test]
    fn pulse_is_zero_at_trough() {
        // sin(phase) = -1 when hz*t = 0.75  → brightness 0.
        let fx = Pulse { color: PixelColor::rgb(200, 0, 0), hz: 1.0 };
        let mut out = [PixelColor::default(); 2];
        fx.render(750, &[Vec3::ZERO; 2], &mut out); // t = 0.75 s
        assert_eq!(out[0], PixelColor::rgb(0, 0, 0), "trough is dark");
    }
}
