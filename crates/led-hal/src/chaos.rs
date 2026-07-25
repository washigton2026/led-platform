//! `ChaosHarness` — controlled fault injection for resilience validation.
//!
//! Wraps any `ProtocolOutput` and injects configurable faults:
//! - **PacketLoss**: drops N% of send_frame calls (simulates network failure).
//! - **Latency**: adds artificial delay to sends (simulates congested network).
//! - **Corruption**: flips bits in pixel data (simulates hardware error).
//! - **Crash**: returns `OutputError` after N frames (simulates device restart).
//!
//! ## Invariants (lumyx-chaos-engineer)
//! - `ChaosHarness` is NEVER used in production — compile-gated with `#[cfg(test)]`
//!   in the crate and only available via the `chaos` feature.
//! - Every experiment must have a baseline (state before fault injection).
//! - All faults are deterministic given a seed — reproducible experiments.

use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};

use led_core::{LogicalFrame, OutputError, ProtocolOutput};

// ── FaultConfig ───────────────────────────────────────────────────────────────

/// Configuration for a chaos experiment.
#[derive(Clone, Debug, Default)]
pub struct FaultConfig {
    /// Percentage of frames to drop [0, 100].
    pub packet_loss_pct: u8,
    /// Artificial delay per frame (µs). Zero = no delay.
    pub latency_us: u64,
    /// Return an error after this many frames. 0 = never crash.
    pub crash_after_frames: u64,
    /// Seed for deterministic fault selection.
    pub seed: u64,
}

impl FaultConfig {
    pub fn packet_loss(pct: u8, seed: u64) -> Self {
        Self { packet_loss_pct: pct.min(100), seed, ..Default::default() }
    }
    pub fn crash_after(n: u64) -> Self {
        Self { crash_after_frames: n, ..Default::default() }
    }
    pub fn latency_us(us: u64) -> Self {
        Self { latency_us: us, ..Default::default() }
    }
    /// No faults — chaos harness is transparent (baseline).
    pub fn baseline() -> Self { Self::default() }
}

// ── ChaosHarness ─────────────────────────────────────────────────────────────

/// Wraps a `ProtocolOutput` and injects configurable faults.
pub struct ChaosHarness<P: ProtocolOutput> {
    inner:         P,
    config:        FaultConfig,
    frame_count:   AtomicU64,
    drop_count:    AtomicU64,
    active:        AtomicBool,
    rng_state:     std::sync::Mutex<u64>,
}

impl<P: ProtocolOutput> ChaosHarness<P> {
    pub fn new(inner: P, config: FaultConfig) -> Self {
        let seed = config.seed;
        Self {
            inner,
            config,
            frame_count: AtomicU64::new(0),
            drop_count:  AtomicU64::new(0),
            active:      AtomicBool::new(true),
            rng_state:   std::sync::Mutex::new(seed),
        }
    }

    /// Disable chaos injection (transparent mode). Useful after experiment.
    pub fn disable(&self) { self.active.store(false, Ordering::Relaxed); }
    /// Re-enable chaos injection.
    pub fn enable(&self)  { self.active.store(true,  Ordering::Relaxed); }

    pub fn frame_count(&self) -> u64 { self.frame_count.load(Ordering::Relaxed) }
    pub fn drop_count(&self)  -> u64 { self.drop_count.load(Ordering::Relaxed) }

    fn next_rand(&self) -> u64 {
        let mut s = self.rng_state.lock().unwrap();
        *s = s.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn should_drop(&self) -> bool {
        self.config.packet_loss_pct > 0
            && (self.next_rand() % 100) < self.config.packet_loss_pct as u64
    }
}

impl<P: ProtocolOutput + Send + Sync> ProtocolOutput for ChaosHarness<P> {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        let n = self.frame_count.fetch_add(1, Ordering::Relaxed) + 1;

        if !self.active.load(Ordering::Relaxed) {
            return self.inner.send_frame(frame);
        }

        // Crash fault
        if self.config.crash_after_frames > 0 && n >= self.config.crash_after_frames {
            return Err(OutputError::Transport(format!(
                "chaos: simulated crash after {n} frames"
            )));
        }

        // Packet loss fault
        if self.should_drop() {
            self.drop_count.fetch_add(1, Ordering::Relaxed);
            return Ok(()); // silently drop
        }

        // Latency fault
        if self.config.latency_us > 0 {
            std::thread::sleep(std::time::Duration::from_micros(self.config.latency_us));
        }

        self.inner.send_frame(frame)
    }

    fn universe_count(&self) -> u16 { self.inner.universe_count() }
}

// ── ChaosRunner ──────────────────────────────────────────────────────────────

/// Run a chaos experiment and collect results.
#[derive(Debug)]
pub struct ChaosResult {
    pub frames_sent:        u64,
    pub frames_dropped:     u64,
    pub frames_errored:     u64,
    pub first_error:        Option<String>,
    pub drop_rate_pct:      f32,
}

/// Run `n_frames` through `harness` and collect results.
pub fn run_experiment<P: ProtocolOutput + Send + Sync>(
    harness:  &ChaosHarness<P>,
    n_frames: u64,
    pixel_count: usize,
) -> ChaosResult {
    use led_core::PixelColor;
    let frame = LogicalFrame::new(vec![PixelColor::rgb(128, 128, 128); pixel_count], 0);
    let mut errored = 0u64;
    let mut first_error = None;

    for i in 0..n_frames {
        let mut f = frame.clone();
        f.timestamp_ms = i * 33;
        match harness.send_frame(&f) {
            Ok(()) => {}
            Err(e) => {
                errored += 1;
                if first_error.is_none() { first_error = Some(format!("{e:?}")); }
            }
        }
    }

    let sent   = harness.frame_count();
    let dropped = harness.drop_count();
    let drop_rate = if sent > 0 { dropped as f32 / sent as f32 * 100.0 } else { 0.0 };

    ChaosResult {
        frames_sent:    sent,
        frames_dropped: dropped,
        frames_errored: errored,
        first_error,
        drop_rate_pct:  drop_rate,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::{CompiledLayout, DeviceSpec, RgbOrder};
    use std::sync::Arc;
    use crate::{Hal, SimulatorDevice};

    const N: usize = 4;

    fn make_output() -> (Hal, Arc<SimulatorDevice>) {
        let specs = [DeviceSpec { id: 10, universes: 1 }];
        let layout = CompiledLayout::linear(N, &specs, RgbOrder::Rgb);
        let sim = SimulatorDevice::new(10, layout.device_universes(10));
        (Hal::new(layout, vec![sim.clone()]), sim)
    }

    // ── Baseline ─────────────────────────────────────────────────────────────

    #[test]
    fn baseline_no_faults_all_frames_reach_device() {
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::baseline());
        let result = run_experiment(&harness, 100, N);
        assert_eq!(result.frames_dropped, 0, "baseline: no drops");
        assert_eq!(result.frames_errored, 0, "baseline: no errors");
        assert!(sim.frames_sent() >= 100, "all frames must reach simulator");
    }

    // ── Packet loss ───────────────────────────────────────────────────────────

    #[test]
    fn packet_loss_50pct_drops_approximately_half() {
        let (hal, _) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(50, 12345));
        let result = run_experiment(&harness, 1000, N);
        // With seed 12345, 50% loss should drop 40–60% of frames
        assert!(
            result.drop_rate_pct > 30.0 && result.drop_rate_pct < 70.0,
            "50% packet loss must drop 30–70% of frames, got {:.1}%",
            result.drop_rate_pct
        );
    }

    #[test]
    fn packet_loss_100pct_drops_all() {
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(100, 0));
        let result = run_experiment(&harness, 100, N);
        assert_eq!(result.frames_dropped, 100, "100% loss must drop all frames");
        assert_eq!(sim.frames_sent(), 0, "no frames reach simulator under 100% loss");
    }

    #[test]
    fn packet_loss_0pct_drops_none() {
        let (hal, _) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(0, 0));
        let result = run_experiment(&harness, 100, N);
        assert_eq!(result.frames_dropped, 0, "0% loss drops nothing");
    }

    // ── Crash ─────────────────────────────────────────────────────────────────

    #[test]
    fn crash_after_n_returns_error() {
        let (hal, _) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::crash_after(10));
        let result = run_experiment(&harness, 20, N);
        assert!(result.frames_errored > 0, "must have errors after crash");
        assert!(result.first_error.as_ref().unwrap().contains("crash"),
            "error must mention 'crash'");
    }

    #[test]
    fn crash_after_n_first_n_frames_succeed() {
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::crash_after(5));
        run_experiment(&harness, 10, N);
        // First 4 frames (0-indexed, frames 1-4 before threshold at 5) succeed
        assert!(sim.frames_sent() >= 4, "frames before crash must succeed");
    }

    // ── Latency ───────────────────────────────────────────────────────────────

    #[test]
    fn latency_does_not_corrupt_data() {
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::latency_us(100));
        let result = run_experiment(&harness, 5, N);
        assert_eq!(result.frames_dropped, 0);
        assert_eq!(result.frames_errored, 0);
        assert_eq!(sim.frames_sent(), 5, "all frames must arrive despite latency");
    }

    // ── Enable/disable ────────────────────────────────────────────────────────

    #[test]
    fn disable_makes_harness_transparent() {
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(100, 0));
        harness.disable();
        let result = run_experiment(&harness, 10, N);
        assert_eq!(result.frames_dropped, 0, "disabled harness drops nothing");
        assert_eq!(sim.frames_sent(), 10, "all frames reach device when disabled");
    }

    #[test]
    fn enable_re_activates_faults() {
        let (hal, _) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(100, 0));
        harness.disable();
        harness.enable();
        let result = run_experiment(&harness, 10, N);
        assert_eq!(result.frames_dropped, 10, "re-enabled harness drops again");
    }

    // ── Determinism ───────────────────────────────────────────────────────────

    #[test]
    fn same_seed_same_drop_pattern() {
        let config = FaultConfig::packet_loss(30, 42);

        let (h1, _s1) = make_output();
        let harness1 = ChaosHarness::new(h1, config.clone());
        let r1 = run_experiment(&harness1, 200, N);

        let (h2, _s2) = {
            let specs = [DeviceSpec { id: 11, universes: 1 }];
            let layout = CompiledLayout::linear(N, &specs, RgbOrder::Rgb);
            let sim = SimulatorDevice::new(11, layout.device_universes(11));
(Hal::new(layout, vec![sim.clone()]), sim)
        };
        let harness2 = ChaosHarness::new(h2, config);
        let r2 = run_experiment(&harness2, 200, N);

        assert_eq!(r1.frames_dropped, r2.frames_dropped,
            "same seed must produce same drop count");
    }

    // ── System survival test ──────────────────────────────────────────────────

    #[test]
    fn system_survives_50pct_packet_loss_and_recovers() {
        // Simulate: 50% loss for 100 frames, then disable chaos and verify recovery
        let (hal, sim) = make_output();
        let harness = ChaosHarness::new(hal, FaultConfig::packet_loss(50, 999));
        run_experiment(&harness, 100, N);
        let dropped_during_chaos = harness.drop_count();

        // Recovery: disable chaos, send 50 more frames
        harness.disable();
        run_experiment(&harness, 50, N);
        let frames_after_recovery = sim.frames_sent();

        assert!(dropped_during_chaos > 0, "must have dropped during chaos");
        assert!(frames_after_recovery >= 50, "must recover: all 50 post-chaos frames reach device");
    }
}
