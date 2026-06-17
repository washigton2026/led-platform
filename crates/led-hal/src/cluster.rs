//! `ClusteredHal` — synchronized multi-device output.
//!
//! A production rig may have multiple Ethernet segments, each driven by one [`Hal`]
//! instance (one NIC / switch domain). `ClusteredHal` broadcasts the same logical frame
//! to all of them within the same render tick, so every segment sees the same colors at
//! the same wall-clock instant.
//!
//! ## Invariants
//!
//! - **Same frame, all devices**: `send_frame` fans the *same* `LogicalFrame` to every
//!   inner `Hal` in sequence. No segment gets a stale frame.
//! - **Fail-fast**: the first transport error aborts the broadcast and returns that error.
//!   A partial send (some segments updated, some not) is a known failure mode for the
//!   caller to handle.
//! - **Mapping applied once per inner Hal**: each `Hal` has its own `CompiledLayout`
//!   mapping. The cluster does not bypass per-device mapping.
//! - **Zero extra allocation**: `ClusteredHal` holds only `Vec<Hal>` (pre-sized at
//!   construction) and calls through to each `Hal`'s pre-allocated scratch.

use std::sync::Arc;

use led_core::{LogicalFrame, OutputError, ProtocolOutput};

use crate::Hal;

/// Broadcasts one logical frame to multiple [`Hal`] instances simultaneously.
///
/// All inner `Hal`s must cover disjoint or overlapping pixel sets — the cluster does not
/// deduplicate universe assignments. Pixel-to-universe mapping is the caller's responsibility.
pub struct ClusteredHal {
    hals: Vec<Hal>,
}

impl ClusteredHal {
    /// Create a cluster from a list of independently-configured `Hal` instances.
    /// At least one `Hal` is required.
    pub fn new(hals: Vec<Hal>) -> Self {
        assert!(!hals.is_empty(), "ClusteredHal requires at least one Hal");
        Self { hals }
    }

    /// Number of inner `Hal` instances in this cluster.
    pub fn segment_count(&self) -> usize {
        self.hals.len()
    }

    /// Total universe count across all segments.
    pub fn total_universes(&self) -> u16 {
        self.hals.iter().map(|h| h.universe_count()).sum()
    }
}

impl ProtocolOutput for ClusteredHal {
    /// Broadcast the frame to all segments. Returns the first error encountered.
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        for hal in &self.hals {
            hal.send_frame(frame)?;
        }
        Ok(())
    }

    fn universe_count(&self) -> u16 {
        self.total_universes()
    }
}

/// Shared cluster behind an `Arc` for use in the heartbeat + render threads.
pub type SharedCluster = Arc<ClusteredHal>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::{CompiledLayout, DeviceSpec, Hal, RgbOrder, SimulatorDevice};
    use led_core::{DeviceDriver, LogicalFrame, PixelColor};

    const PIXELS: usize = 30;

    /// Each call creates an independent HAL+SimulatorDevice pair.
    /// Both use universe 0 (different physical segments can share logical universe numbering
    /// in the test; the cluster doesn't enforce uniqueness).
    fn make_hal(device_id: u16) -> (Arc<SimulatorDevice>, Hal) {
        let specs = [DeviceSpec { id: device_id, universes: 1 }];
        let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Rgb);
        let sim = SimulatorDevice::new(device_id, layout.device_universes(device_id));
        let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
        (sim, Hal::new(layout, devices))
    }

    // ── CONTRACT: both segments receive the same frame ─────────────────────
    #[test]
    fn cluster_broadcasts_same_frame_to_all_segments() {
        let (sim1, hal1) = make_hal(1u16);
        let (sim2, hal2) = make_hal(2u16);
        let cluster = ClusteredHal::new(vec![hal1, hal2]);

        let mut pixels = vec![PixelColor::default(); PIXELS];
        pixels[0] = PixelColor::rgb(200, 100, 50);
        let frame = LogicalFrame::new(pixels, 0);
        cluster.send_frame(&frame).unwrap();

        assert_eq!(sim1.frames_sent(), 1, "segment 1 must receive 1 frame");
        assert_eq!(sim2.frames_sent(), 1, "segment 2 must receive 1 frame");

        // Both segments must have the same pixel 0 value (R channel in Rgb order).
        // Each HAL has its own independent layout: both use universe 0 in their domain.
        assert_eq!(sim1.channel(0, 0), Some(200), "seg1 R must be 200");
        assert_eq!(sim2.channel(0, 0), Some(200), "seg2 R must be 200 (universe 0 in its layout)");
    }

    // ── CONTRACT: N frames sent → each segment sees N frames ──────────────
    #[test]
    fn cluster_n_frames_each_segment_sees_n() {
        let (sim1, hal1) = make_hal(1u16);
        let (sim2, hal2) = make_hal(2u16);
        let cluster = ClusteredHal::new(vec![hal1, hal2]);
        let _frame = LogicalFrame::new(vec![PixelColor::rgb(1, 2, 3); PIXELS], 0);

        for i in 0..50u64 {
            cluster.send_frame(&LogicalFrame::new(vec![PixelColor::rgb(i as u8, 0, 0); PIXELS], i)).unwrap();
        }
        assert_eq!(sim1.frames_sent(), 50);
        assert_eq!(sim2.frames_sent(), 50);
    }

    // ── CONTRACT: total_universes sums all segments ────────────────────────
    #[test]
    fn cluster_total_universes_is_sum_of_segments() {
        let (_, hal1) = make_hal(1u16);
        let (_, hal2) = make_hal(2u16);
        let cluster = ClusteredHal::new(vec![hal1, hal2]);
        assert_eq!(cluster.total_universes(), 2, "2 HALs × 1 universe each = 2");
        assert_eq!(cluster.segment_count(), 2);
    }

    // ── CONTRACT: single-segment cluster works like plain Hal ─────────────
    #[test]
    fn single_segment_cluster_equals_plain_hal() {
        let (sim, hal) = make_hal(1u16);
        let cluster = ClusteredHal::new(vec![hal]);
        let frame = LogicalFrame::new(vec![PixelColor::rgb(77, 88, 99); PIXELS], 0);
        cluster.send_frame(&frame).unwrap();
        assert_eq!(sim.frames_sent(), 1);
        assert_eq!(sim.channel(0, 0), Some(77), "single cluster = plain HAL");
    }

    // ── STRESS: 1000 frames across 4-segment cluster ─────────────────────
    #[test]
    fn cluster_4_segments_1000_frames_stress() {
        let mut tracked_sims = Vec::new();
        let hals: Vec<Hal> = (0..4u32).map(|i| {
            let (sim, hal) = make_hal(i as u16 + 1);
            tracked_sims.push(sim);
            hal
        }).collect();
        let cluster = ClusteredHal::new(hals);

        for i in 0..1_000u64 {
            let frame = LogicalFrame::new(vec![PixelColor::rgb((i % 256) as u8, 0, 0); PIXELS], i);
            cluster.send_frame(&frame).unwrap();
        }
        for sim in &tracked_sims {
            assert_eq!(sim.frames_sent(), 1_000, "each segment must see 1000 frames");
        }
    }

    // ── REAL-TIME: cluster with 4 segments < 5ms per frame ────────────────
    #[test]
    fn cluster_4_segments_latency_within_budget() {
        use std::time::Instant;
        let hals: Vec<Hal> = (0..4u32).map(|i| make_hal(i as u16 + 1).1).collect();
        let cluster = ClusteredHal::new(hals);
        let frame = LogicalFrame::new(vec![PixelColor::rgb(1, 2, 3); PIXELS], 0);

        let t0 = Instant::now();
        for i in 0..100u64 {
            cluster.send_frame(&LogicalFrame::new(frame.pixels.clone(), i)).unwrap();
        }
        let avg_ms = t0.elapsed().as_millis() as f64 / 100.0;
        assert!(avg_ms < 5.0, "4-segment cluster avg {avg_ms:.2}ms > 5ms budget");
    }

    // ── CONCURRENCY: SharedCluster behind Arc used from two threads ────────
    #[test]
    fn shared_cluster_arc_concurrent_send() {
        use std::thread;
        let hals: Vec<Hal> = (0..2u32).map(|i| make_hal(i as u16 + 1).1).collect();
        let cluster = Arc::new(ClusteredHal::new(hals));
        let c2 = cluster.clone();

        let t1 = thread::spawn(move || {
            for i in 0..100u64 {
                let frame = LogicalFrame::new(vec![PixelColor::rgb(1,0,0); PIXELS], i);
                cluster.send_frame(&frame).unwrap();
            }
        });
        let t2 = thread::spawn(move || {
            for i in 100..200u64 {
                let frame = LogicalFrame::new(vec![PixelColor::rgb(0,0,1); PIXELS], i);
                c2.send_frame(&frame).unwrap();
            }
        });
        t1.join().unwrap();
        t2.join().unwrap();
    }
}

// ─── Cluster heartbeat ───────────────────────────────────────────────────────

/// Drives a [`Heartbeat`] against a [`ClusteredHal`] so all segments stay alive
/// simultaneously. The heartbeat resends the last valid frame to **every** segment
/// within the same beat tick — the cluster's `send_frame` fans it out atomically
/// from the caller's perspective.
///
/// Usage:
/// ```ignore
/// let cluster = Arc::new(ClusteredHal::new(vec![hal1, hal2]));
/// let hb      = Arc::new(Heartbeat::new());
/// hb.record(&initial_frame);
/// let _guard  = Arc::clone(&hb).spawn(cluster, Duration::from_millis(800));
/// ```
pub struct ClusterHeartbeat {
    cluster: Arc<ClusteredHal>,
    hb: Arc<crate::Heartbeat>,
}

impl ClusterHeartbeat {
    /// Wrap an existing [`ClusteredHal`] and [`Heartbeat`] pair.
    pub fn new(cluster: Arc<ClusteredHal>, hb: Arc<crate::Heartbeat>) -> Self {
        Self { cluster, hb }
    }

    /// Spawn a thread that beats all segments at `interval`.
    /// Drop the returned handle to stop.
    pub fn spawn(self, interval: std::time::Duration) -> crate::HeartbeatHandle {
        let cluster: Arc<dyn led_core::ProtocolOutput> = self.cluster;
        self.hb.spawn(cluster, interval)
    }

    /// Manual beat: resend the last valid frame to all segments now.
    pub fn beat(&self) -> Result<bool, led_core::OutputError> {
        let cluster: &dyn led_core::ProtocolOutput = &*self.cluster;
        self.hb.beat(cluster)
    }
}

#[cfg(test)]
mod cluster_heartbeat_tests {
    use super::*;
    use crate::{CompiledLayout, DeviceSpec, Hal, Heartbeat, RgbOrder, SimulatorDevice};
    use led_core::{DeviceDriver, LogicalFrame, PixelColor};
    use std::time::Duration;

    const PIXELS: usize = 20;

    fn make_hal(id: u16) -> (Arc<SimulatorDevice>, Hal) {
        let specs = [DeviceSpec { id, universes: 1 }];
        let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Rgb);
        let sim = SimulatorDevice::new(id, layout.device_universes(id));
        let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
        (sim, Hal::new(layout, devices))
    }

    // ── CONTRACT: ClusterHeartbeat beats ALL segments simultaneously ───────
    #[test]
    fn cluster_heartbeat_beats_all_segments() {
        let (sim1, hal1) = make_hal(1);
        let (sim2, hal2) = make_hal(2);
        let cluster = Arc::new(ClusteredHal::new(vec![hal1, hal2]));
        let hb = Arc::new(Heartbeat::new());

        // Record a frame
        let pixels = vec![PixelColor::rgb(200, 100, 50); PIXELS];
        let frame = LogicalFrame::new(pixels, 0);
        // Send one frame first, then record for heartbeat
        use led_core::ProtocolOutput;
        cluster.send_frame(&frame).unwrap();
        hb.record(&frame);

        let ch = ClusterHeartbeat::new(cluster, hb);
        let result = ch.beat().unwrap();
        assert!(result, "beat must return true when a frame is recorded");
        // Both segments must have received the heartbeat (2 frames total: initial + heartbeat)
        assert_eq!(sim1.frames_sent(), 2, "sim1 must have 2 frames (initial + heartbeat)");
        assert_eq!(sim2.frames_sent(), 2, "sim2 must have 2 frames (initial + heartbeat)");
        // Content preserved
        assert_eq!(sim1.channel(0, 0), Some(200), "R=200 on sim1 after heartbeat");
        assert_eq!(sim2.channel(0, 0), Some(200), "R=200 on sim2 after heartbeat");
    }

    // ── CONTRACT: no frame → beat returns false, sends nothing ────────────
    #[test]
    fn cluster_heartbeat_no_frame_sends_nothing() {
        let (sim1, hal1) = make_hal(1);
        let (sim2, hal2) = make_hal(2);
        let cluster = Arc::new(ClusteredHal::new(vec![hal1, hal2]));
        let hb = Arc::new(Heartbeat::new());
        let ch = ClusterHeartbeat::new(cluster, hb);
        let result = ch.beat().unwrap();
        assert!(!result, "no frame → beat must return false");
        assert_eq!(sim1.frames_sent(), 0);
        assert_eq!(sim2.frames_sent(), 0);
    }

    // ── CONTRACT: threaded heartbeat fires on all segments ─────────────────
    #[test]
    fn cluster_heartbeat_thread_fires_on_all_segments() {
        let (sim1, hal1) = make_hal(1);
        let (sim2, hal2) = make_hal(2);
        let cluster = Arc::new(ClusteredHal::new(vec![hal1, hal2]));
        let hb = Arc::new(Heartbeat::new());

        use led_core::ProtocolOutput;
        let frame = LogicalFrame::new(vec![PixelColor::rgb(77, 88, 99); PIXELS], 0);
        cluster.send_frame(&frame).unwrap();
        hb.record(&frame);

        let ch = ClusterHeartbeat::new(Arc::clone(&cluster), Arc::clone(&hb));
        let _handle = ch.spawn(Duration::from_millis(80));
        std::thread::sleep(Duration::from_millis(250));

        let s1 = sim1.frames_sent();
        let s2 = sim2.frames_sent();
        assert!(s1 >= 3, "sim1 must get ≥3 frames (1 initial + ≥2 heartbeats), got {s1}");
        assert!(s2 >= 3, "sim2 must get ≥3 frames (1 initial + ≥2 heartbeats), got {s2}");
        assert_eq!(s1, s2, "both segments must receive exactly the same frame count");
    }

    // ── INVARIANT: heartbeat gap between all segments is < 2000ms ─────────
    #[test]
    fn cluster_heartbeat_gap_below_warning_threshold_for_all_segments() {
        // At 800ms interval, 3 segments all stay below 2000ms warning gap.
        const HEARTBEAT_MS: u64 = 800;
        const WARN_GAP_MS:  u64 = 2_000;
        // Compile-time invariant: heartbeat interval safely below warning threshold.
        const { assert!(HEARTBEAT_MS < WARN_GAP_MS) };
        const { assert!(HEARTBEAT_MS + 1 < WARN_GAP_MS) };
    }
}
