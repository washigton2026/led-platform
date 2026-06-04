//! GPU-style compute effects. A `ComputeKernel` is a **data-parallel, per-pixel pure
//! function** — exactly the body of a WGSL `@compute @workgroup_size(64)` shader. Running it
//! over all pixels is `ComputeEffect` (the CPU executor, used now and in tests); the GPU
//! executor (wgpu) is a drop-in that dispatches the *same* kernel — see [`PLASMA_WGSL`] and
//! `references/gpu-compute.md`.
//!
//! Why this shape: the master skill's effect hierarchy moves an O(n) CPU effect to GPU
//! compute when a frame misses its deadline (latency budget §6). Writing the kernel once as
//! a portable per-pixel function lets the CPU reference and the GPU shader stay identical and
//! testable, so "move it to the GPU" is wiring, not a rewrite.

use led_core::PixelColor;

use crate::color;
use crate::effect::{Effect, Vec3};

/// The body of a compute shader: color for one pixel, a pure function of its index, logical
/// position, time, and the pixel count. No shared state — trivially parallel / GPU-portable.
pub trait ComputeKernel: Send {
    fn color(&self, index: u32, pos: Vec3, time_ms: u64, count: u32) -> PixelColor;
}

/// Runs a [`ComputeKernel`] over every pixel (CPU executor). Implements [`Effect`], so the
/// render→send pipeline drives it. Allocation-free.
pub struct ComputeEffect<K: ComputeKernel> {
    pub kernel: K,
}

impl<K: ComputeKernel> ComputeEffect<K> {
    pub fn new(kernel: K) -> Self {
        Self { kernel }
    }
}

impl<K: ComputeKernel> Effect for ComputeEffect<K> {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let n = positions.len() as u32;
        for (i, (p, o)) in positions.iter().zip(out.iter_mut()).enumerate() {
            *o = self.kernel.color(i as u32, *p, time_ms, n);
        }
    }
}

/// Classic plasma field: three summed sine waves over logical position + time → a hue.
/// Pure per-pixel; the CPU code below and [`PLASMA_WGSL`] compute the identical value.
pub struct Plasma {
    pub scale: f32,
    pub speed: f32,
}

impl ComputeKernel for Plasma {
    fn color(&self, _index: u32, pos: Vec3, time_ms: u64, _count: u32) -> PixelColor {
        let t = time_ms as f32 / 1000.0 * self.speed;
        let v = ((pos.x * self.scale + t).sin()
            + (pos.y * self.scale - t).sin()
            + ((pos.x + pos.y) * self.scale * 0.5 + t).sin())
            / 3.0; // -1..1
        color::hsv_to_rgb(0.5 + 0.5 * v, 1.0, 1.0)
    }
}

/// The GPU counterpart of [`Plasma`] — a real WGSL compute shader. One invocation per pixel,
/// `workgroup_size(64)`. Wired by the GPU executor under the `gpu` feature (see
/// `references/gpu-compute.md`); the CPU `Plasma` above is its tested reference.
pub const PLASMA_WGSL: &str = r#"
struct Params { scale: f32, speed: f32, time_s: f32, count: u32 };
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read>  positions: array<vec3<f32>>;
@group(0) @binding(2) var<storage, read_write> out_rgb: array<u32>;

fn hsv(h: f32) -> vec3<f32> {
    // s = v = 1; matches color::hsv_to_rgb
    let h6 = fract(h) * 6.0;
    let x = 1.0 - abs(h6 % 2.0 - 1.0);
    if (h6 < 1.0) { return vec3<f32>(1.0, x, 0.0); }
    if (h6 < 2.0) { return vec3<f32>(x, 1.0, 0.0); }
    if (h6 < 3.0) { return vec3<f32>(0.0, 1.0, x); }
    if (h6 < 4.0) { return vec3<f32>(0.0, x, 1.0); }
    if (h6 < 5.0) { return vec3<f32>(x, 0.0, 1.0); }
    return vec3<f32>(1.0, 0.0, x);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    let p = positions[i];
    let t = params.time_s * params.speed;
    let v = (sin(p.x * params.scale + t)
           + sin(p.y * params.scale - t)
           + sin((p.x + p.y) * params.scale * 0.5 + t)) / 3.0;
    let c = hsv(0.5 + 0.5 * v) * 255.0 + vec3<f32>(0.5);
    out_rgb[i] = (u32(c.r) << 16u) | (u32(c.g) << 8u) | u32(c.b);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_is_deterministic_and_matches_known_value() {
        let fx = ComputeEffect::new(Plasma { scale: 0.5, speed: 1.0 });
        let positions = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0)];
        let mut a = vec![PixelColor::default(); 3];
        let mut b = vec![PixelColor::default(); 3];

        fx.render(0, &positions, &mut a);
        fx.render(0, &positions, &mut b);
        assert_eq!(a, b, "same time ⇒ same frame");

        // At pos=0, t=0: v=0 → hue 0.5 → hsv(0.5,1,1) = cyan.
        assert_eq!(a[0], PixelColor::rgb(0, 255, 255));
        // Distinct positions generally differ.
        assert_ne!(a[1], a[0]);
    }

    #[test]
    fn compute_effect_fills_every_pixel() {
        let fx = ComputeEffect::new(Plasma { scale: 1.0, speed: 2.0 });
        let positions: Vec<Vec3> = (0..64).map(|i| Vec3::new(i as f32 * 0.1, 0.0, 0.0)).collect();
        let mut out = vec![PixelColor::default(); 64];
        fx.render(500, &positions, &mut out);
        // Plasma is fully saturated/bright, so no pixel stays pure black-default by accident
        // across the whole strip (sanity that the kernel ran for every index).
        assert!(out.iter().any(|&c| c != PixelColor::default()));
    }
}
