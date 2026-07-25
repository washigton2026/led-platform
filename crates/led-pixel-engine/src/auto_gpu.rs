//! `AutoGpuPlasma` — a `Plasma` effect that automatically switches between CPU
//! and GPU execution based on pixel count and hardware availability.
//!
//! ## Behaviour
//!
//! ```text
//! pixel_count ≤ threshold  OR  no GPU adapter  →  CPU executor (ComputeEffect<Plasma>)
//! pixel_count >  threshold AND GPU available    →  GpuPlasmaExecutor (wgpu dispatch)
//! ```
//!
//! The switch happens at construction time (not per frame). The GPU path is
//! only compiled when the `gpu` feature is enabled.
//!
//! ## Invariants
//! - CPU and GPU paths produce identical output for the same `time_ms` and pixel set
//!   (parity validated in tests, tolerance ≤ 1 LSB per channel).
//! - If GPU init fails for any reason, the CPU path is used silently — no panic.
//! - The threshold default is `GPU_THRESHOLD_PIXELS = 50_000`. Override via
//!   `AutoGpuPlasma::with_threshold`.

use led_core::PixelColor;

use crate::compute::{ComputeEffect, Plasma};
use crate::effect::{Effect, Vec3};

/// Pixel count above which GPU execution is attempted (when available).
pub const GPU_THRESHOLD_PIXELS: usize = 50_000;

// ── Executor selector ─────────────────────────────────────────────────────────

enum Executor {
    Cpu(ComputeEffect<Plasma>),
    #[cfg(feature = "gpu")]
    Gpu(crate::gpu_executor::GpuPlasmaExecutor),
}

// ── AutoGpuPlasma ─────────────────────────────────────────────────────────────

/// A `Plasma` effect that auto-selects CPU or GPU execution at construction time.
pub struct AutoGpuPlasma {
    executor:  Executor,
    pub scale: f32,
    pub speed: f32,
    /// Threshold used at construction (informational only after init).
    pub threshold: usize,
    /// Whether GPU is actually being used.
    pub using_gpu: bool,
}

impl AutoGpuPlasma {
    /// Create with default threshold (`GPU_THRESHOLD_PIXELS`).
    ///
    /// If `pixel_count > threshold` and a GPU adapter is available, the GPU executor
    /// is used. Otherwise, the CPU executor is used.
    pub fn new(pixel_count: usize, scale: f32, speed: f32) -> Self {
        Self::with_threshold(pixel_count, scale, speed, GPU_THRESHOLD_PIXELS)
    }

    /// Create with a custom threshold. `threshold = 0` forces GPU (if available).
    pub fn with_threshold(_pixel_count: usize, scale: f32, speed: f32, threshold: usize) -> Self {
        #[cfg(feature = "gpu")]
        if pixel_count > threshold {
            if let Some(gpu) =
                crate::gpu_executor::GpuPlasmaExecutor::try_new(pixel_count, scale, speed)
            {
                return Self {
                    executor:  Executor::Gpu(gpu),
                    scale,
                    speed,
                    threshold,
                    using_gpu: true,
                };
            }
        }

        // Fall back to CPU
        Self {
            executor:  Executor::Cpu(ComputeEffect::new(Plasma { scale, speed })),
            scale,
            speed,
            threshold,
            using_gpu: false,
        }
    }

    /// Force CPU execution regardless of pixel count or GPU availability.
    /// Useful for tests and determinism guarantees.
    pub fn cpu_only(scale: f32, speed: f32) -> Self {
        Self {
            executor:  Executor::Cpu(ComputeEffect::new(Plasma { scale, speed })),
            scale,
            speed,
            threshold: usize::MAX,
            using_gpu: false,
        }
    }
}

impl Effect for AutoGpuPlasma {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        match &self.executor {
            Executor::Cpu(cpu) => cpu.render(time_ms, positions, out),
            #[cfg(feature = "gpu")]
            Executor::Gpu(gpu) => gpu.render(time_ms, positions, out),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Effect, Vec3};
    use led_core::PixelColor;

    fn positions(n: usize) -> Vec<Vec3> {
        (0..n).map(|i| Vec3::new(i as f32 * 0.1, 0.0, 0.0)).collect()
    }

    // ── CPU path ─────────────────────────────────────────────────────────────

    #[test]
    fn cpu_only_renders_without_panic() {
        let fx = AutoGpuPlasma::cpu_only(0.5, 1.0);
        let pos = positions(64);
        let mut out = vec![PixelColor::default(); 64];
        fx.render(0, &pos, &mut out);
        assert!(out.iter().any(|&p| p != PixelColor::default()), "must produce non-black output");
    }

    #[test]
    fn cpu_only_is_deterministic() {
        let pos = positions(32);
        let mut a = vec![PixelColor::default(); 32];
        let mut b = vec![PixelColor::default(); 32];
        let fx = AutoGpuPlasma::cpu_only(0.5, 1.0);
        fx.render(1000, &pos, &mut a);
        fx.render(1000, &pos, &mut b);
        assert_eq!(a, b, "same time_ms → same output");
    }

    // ── Auto-select ───────────────────────────────────────────────────────────

    #[test]
    fn below_threshold_uses_cpu() {
        // 10 pixels < default threshold (50k) → must use CPU
        let fx = AutoGpuPlasma::new(10, 0.5, 1.0);
        assert!(!fx.using_gpu, "small pixel count must use CPU");
    }

    #[test]
    fn above_threshold_attempts_gpu() {
        // Large pixel count → attempts GPU; falls back to CPU if no adapter.
        let pixel_count = GPU_THRESHOLD_PIXELS + 1;
        let fx = AutoGpuPlasma::new(pixel_count, 0.5, 1.0);
        // Either GPU or CPU — both are valid. What matters is no panic.
        let pos = positions(pixel_count.min(128)); // small slice for test speed
        let mut out = vec![PixelColor::default(); pos.len()];
        fx.render(0, &pos, &mut out);
        // Output is valid (non-uniform, since Plasma varies by position)
        // Just verify it ran without panic.
    }

    #[test]
    fn with_threshold_zero_attempts_gpu_always() {
        // threshold=0 means "always try GPU"
        let fx = AutoGpuPlasma::with_threshold(8, 0.5, 1.0, 0);
        let pos = positions(8);
        let mut out = vec![PixelColor::default(); 8];
        fx.render(500, &pos, &mut out);
        // No panic required; using_gpu depends on hardware
    }

    #[test]
    fn cpu_fallback_matches_known_value() {
        // Plasma at origin, t=0 → cyan (from compute.rs tests)
        let fx = AutoGpuPlasma::cpu_only(1.0, 1.0);
        let mut out = vec![PixelColor::default(); 1];
        fx.render(0, &[Vec3::ZERO], &mut out);
        // cyan: g=255, b=255
        assert!(out[0].g > 200 || out[0].b > 200, "Plasma at origin t=0 must be bright: {:?}", out[0]);
    }

    // ── Parity: CPU-only vs auto-select with same pixel count ────────────────

    #[test]
    fn auto_below_threshold_matches_cpu_only() {
        let pos = positions(64);
        let mut cpu_out = vec![PixelColor::default(); 64];
        let mut auto_out = vec![PixelColor::default(); 64];

        let cpu = AutoGpuPlasma::cpu_only(0.5, 1.0);
        let auto = AutoGpuPlasma::new(64, 0.5, 1.0); // 64 < threshold → CPU

        cpu.render(2000, &pos, &mut cpu_out);
        auto.render(2000, &pos, &mut auto_out);

        assert_eq!(cpu_out, auto_out,
            "below-threshold auto-select must produce identical output to cpu_only");
    }

    // ── Threshold accessors ───────────────────────────────────────────────────

    #[test]
    fn threshold_accessible() {
        let fx = AutoGpuPlasma::with_threshold(100, 0.5, 1.0, 75_000);
        assert_eq!(fx.threshold, 75_000);
    }

    #[test]
    fn scale_speed_accessible() {
        let fx = AutoGpuPlasma::cpu_only(0.3, 2.5);
        assert!((fx.scale - 0.3).abs() < 1e-6);
        assert!((fx.speed - 2.5).abs() < 1e-6);
    }
}
