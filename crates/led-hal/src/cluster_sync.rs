//! `SyncedCluster` — multi-node cluster with SharedClock sync, failover, and hot-join.
//!
//! Extends `ClusteredHal` with:
//! - **SharedClock sync**: all nodes use the same `timestamp_ms` derived from the clock.
//! - **Drift detection**: frames arriving with drift > threshold are flagged.
//! - **Failover**: if a segment fails, the cluster continues sending to healthy segments.
//! - **Hot-join**: new segments can be added while the show is running.
//! - **Health tracking**: per-segment `SegmentHealth` with failure counts and last send time.
//!
//! ## Invariants (lumyx-system-architect)
//! - Drift tolerance is configurable; default 5ms (tight LAN sync).
//! - A segment is marked `Degraded` after 3 consecutive failures.
//! - A segment is marked `Failed` after 10 consecutive failures — excluded from sends.
//! - `Failed` segments can be re-added via `rejoin_segment`.
//! - The last valid frame is always cached for heartbeat resend.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use led_core::{LogicalFrame, OutputError, PixelColor, ProtocolOutput};

use crate::hal::Hal;
use crate::shared_clock::SharedClock;
use crate::metrics::MetricsEmitter;

// ── SegmentHealth ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentState {
    /// Sending successfully.
    Healthy,
    /// 3–9 consecutive failures.
    Degraded,
    /// ≥ 10 consecutive failures — excluded from sends until rejoin.
    Failed,
}

#[derive(Debug)]
pub struct SegmentHealth {
    pub state:             SegmentState,
    pub consecutive_fails: u32,
    pub frames_sent:       u64,
    pub last_fail_reason:  Option<String>,
}

impl SegmentHealth {
    fn new() -> Self {
        Self { state: SegmentState::Healthy, consecutive_fails: 0, frames_sent: 0, last_fail_reason: None }
    }

    fn record_success(&mut self) {
        self.consecutive_fails = 0;
        self.frames_sent += 1;
        self.state = SegmentState::Healthy;
    }

    fn record_failure(&mut self, reason: &str) {
        self.consecutive_fails += 1;
        self.last_fail_reason = Some(reason.to_owned());
        self.state = if self.consecutive_fails >= 10 {
            SegmentState::Failed
        } else if self.consecutive_fails >= 3 {
            SegmentState::Degraded
        } else {
            SegmentState::Healthy
        };
    }
}

// ── SyncedCluster ─────────────────────────────────────────────────────────────

/// A production-grade multi-segment cluster with clock sync and failover.
pub struct SyncedCluster {
    /// Segments + their health state. Protected by RwLock for hot-join support.
    pub segments:        RwLock<Vec<(Hal, SegmentHealth)>>,
    /// Shared monotonic clock for timestamp alignment.
    clock:           Arc<SharedClock>,
    /// Maximum acceptable drift between clock and frame timestamp (ms).
    drift_tolerance_ms: u64,
    /// Metrics emitter for the cluster.
    metrics:         Option<Arc<MetricsEmitter>>,
    /// Last valid frame — resent on Heartbeat (last-valid-never-zeros invariant).
    last_frame:      Mutex<Option<(Vec<PixelColor>, u64)>>,
}

impl SyncedCluster {
    /// Create a cluster with the given segments and clock.
    pub fn new(
        hals:               Vec<Hal>,
        clock:              Arc<SharedClock>,
        drift_tolerance_ms: u64,
    ) -> Self {
        let segments = hals.into_iter().map(|h| (h, SegmentHealth::new())).collect();
        Self {
            segments:           RwLock::new(segments),
            clock,
            drift_tolerance_ms,
            metrics:            None,
            last_frame:         Mutex::new(None),
        }
    }

    /// Attach a metrics emitter.
    pub fn with_metrics(mut self, m: Arc<MetricsEmitter>) -> Self {
        self.metrics = Some(m);
        self
    }

    /// Add a new segment to a running cluster (hot-join).
    pub fn hot_join(&self, hal: Hal) {
        self.segments.write().unwrap().push((hal, SegmentHealth::new()));
    }

    /// Re-enable a failed segment. The segment must have been recovered externally.
    pub fn rejoin_segment(&self, index: usize) {
        let mut segs = self.segments.write().unwrap();
        if let Some((_, health)) = segs.get_mut(index) {
            health.consecutive_fails = 0;
            health.state = SegmentState::Healthy;
            health.last_fail_reason = None;
        }
    }

    /// Health snapshot for all segments.
    pub fn health_snapshot(&self) -> Vec<(SegmentState, u64)> {
        self.segments
            .read()
            .unwrap()
            .iter()
            .map(|(_, h)| (h.state.clone(), h.frames_sent))
            .collect()
    }

    /// Number of healthy or degraded segments (not Failed).
    pub fn active_segment_count(&self) -> usize {
        self.segments
            .read()
            .unwrap()
            .iter()
            .filter(|(_, h)| h.state != SegmentState::Failed)
            .count()
    }

    /// Detect drift between the frame timestamp and the current clock time.
    fn drift_ms(&self, frame_ts: u64) -> u64 {
        let now = self.clock.now_ms();
        now.abs_diff(frame_ts)
    }
}

impl ProtocolOutput for SyncedCluster {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        let t0 = Instant::now();

        // Drift check
        let drift = self.drift_ms(frame.timestamp_ms);
        if drift > self.drift_tolerance_ms {
            // Log drift but continue — don't block on drift alone
            // (could be first frame or clock calibration in progress)
            if let Some(m) = &self.metrics {
                m.record_heartbeat_gap(drift * 1_000); // store as µs
            }
        }

        // Cache the frame for heartbeat resend
        {
            let mut last = self.last_frame.lock().unwrap();
            *last = Some((frame.pixels.clone(), frame.timestamp_ms));
        }

        // Send to all active segments; record health
        let mut any_success = false;
        {
            let mut segs = self.segments.write().unwrap();
            for (hal, health) in segs.iter_mut() {
                if health.state == SegmentState::Failed {
                    continue; // skip failed segments
                }
                match hal.send_frame(frame) {
                    Ok(()) => {
                        health.record_success();
                        any_success = true;
                    }
                    Err(e) => {
                        health.record_failure(&format!("{e:?}"));
                        // Continue to other segments — partial send is better than none
                    }
                }
            }
        }

        if let Some(m) = &self.metrics {
            if any_success {
                m.record_frame(t0.elapsed().as_micros() as u64);
            } else {
                m.record_drop();
            }
        }

        if any_success {
            Ok(())
        } else {
            Err(OutputError::Transport("all cluster segments failed".into()))
        }
    }

    fn universe_count(&self) -> u16 {
        self.segments
            .read()
            .unwrap()
            .iter()
            .map(|(h, _)| h.universe_count())
            .sum()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use led_core::{CompiledLayout, DeviceSpec, LogicalFrame, PixelColor, RgbOrder};
    use crate::{Hal, SimulatorDevice, SharedClock};

    const N: usize = 4;

    fn make_hal(id: u16) -> (Hal, Arc<SimulatorDevice>) {
        let specs = [DeviceSpec { id, universes: 1 }];
        let layout = CompiledLayout::linear(N, &specs, RgbOrder::Rgb);
        let sim = SimulatorDevice::new(id, layout.device_universes(id));
        (Hal::new(layout, vec![sim.clone()]), sim)
    }

    fn frame(t: u64) -> LogicalFrame {
        LogicalFrame::new(vec![PixelColor::rgb(100, 150, 200); N], t)
    }

    fn cluster_with_2_segs() -> (SyncedCluster, Arc<SimulatorDevice>, Arc<SimulatorDevice>) {
        let (h1, s1) = make_hal(1);
        let (h2, s2) = make_hal(2);
        let clock = Arc::new(SharedClock::new());
        let c = SyncedCluster::new(vec![h1, h2], clock, 50);
        (c, s1, s2)
    }

    // ── Basic send ────────────────────────────────────────────────────────────

    #[test]
    fn send_frame_reaches_all_segments() {
        let (cluster, s1, s2) = cluster_with_2_segs();
        cluster.send_frame(&frame(0)).unwrap();
        assert!(s1.frames_sent() >= 1, "segment 1 must receive frame");
        assert!(s2.frames_sent() >= 1, "segment 2 must receive frame");
    }

    #[test]
    fn universe_count_sums_all_segments() {
        let (cluster, _, _) = cluster_with_2_segs();
        assert_eq!(cluster.universe_count(), 2, "2 segments × 1 universe = 2");
    }

    // ── Health tracking ───────────────────────────────────────────────────────

    #[test]
    fn all_segments_start_healthy() {
        let (cluster, _, _) = cluster_with_2_segs();
        let health = cluster.health_snapshot();
        assert!(health.iter().all(|(s, _)| *s == SegmentState::Healthy));
    }

    #[test]
    fn active_segment_count_is_2() {
        let (cluster, _, _) = cluster_with_2_segs();
        assert_eq!(cluster.active_segment_count(), 2);
    }

    #[test]
    fn frames_sent_increments_per_send() {
        let (cluster, _, _) = cluster_with_2_segs();
        for i in 0..5u64 { cluster.send_frame(&frame(i * 50)).unwrap(); }
        let health = cluster.health_snapshot();
        assert!(health.iter().all(|(_, sent)| *sent == 5), "each segment gets 5 frames");
    }

    // ── Hot-join ──────────────────────────────────────────────────────────────

    #[test]
    fn hot_join_adds_segment_and_receives_frames() {
        let (cluster, s1, _) = cluster_with_2_segs();
        let (h3, s3) = make_hal(3);
        cluster.hot_join(h3);
        assert_eq!(cluster.active_segment_count(), 3);
        cluster.send_frame(&frame(0)).unwrap();
        assert!(s1.frames_sent() >= 1);
        assert!(s3.frames_sent() >= 1, "hot-joined segment must receive frame");
    }

    // ── Last frame caching ────────────────────────────────────────────────────

    #[test]
    fn last_frame_is_cached_after_send() {
        let (cluster, _, _) = cluster_with_2_segs();
        let f = frame(1000);
        cluster.send_frame(&f).unwrap();
        let cached = cluster.last_frame.lock().unwrap();
        assert!(cached.is_some(), "last frame must be cached");
        let (pixels, ts) = cached.as_ref().unwrap();
        assert_eq!(*ts, 1000);
        assert_eq!(pixels.len(), N);
    }

    // ── Drift detection ───────────────────────────────────────────────────────

    #[test]
    fn drift_detection_does_not_block_send() {
        let (h1, _) = make_hal(1);
        let clock = Arc::new(SharedClock::new());
        // Large drift tolerance — should still send
        let cluster = SyncedCluster::new(vec![h1], clock, 10_000);
        // Frame with timestamp far from clock — drift is large but tolerated
        let f = LogicalFrame::new(vec![PixelColor::default(); N], 999_999_999);
        cluster.send_frame(&f).unwrap(); // must not fail
    }

    // ── Metrics integration ───────────────────────────────────────────────────

    #[test]
    fn metrics_record_frames_when_attached() {
        let (h1, _) = make_hal(1);
        let clock = Arc::new(SharedClock::new());
        let metrics = Arc::new(crate::metrics::MetricsEmitter::new("cluster-test"));
        let cluster = SyncedCluster::new(vec![h1], clock, 50).with_metrics(metrics.clone());
        for i in 0..10u64 { cluster.send_frame(&frame(i * 50)).unwrap(); }
        assert_eq!(metrics.frame_count(), 10);
    }

    // ── Send + Sync ───────────────────────────────────────────────────────────

    #[test]
    fn synced_cluster_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SyncedCluster>();
    }

    // ── Rejoin ────────────────────────────────────────────────────────────────

    #[test]
    fn rejoin_resets_health_to_healthy() {
        let (cluster, _, _) = cluster_with_2_segs();
        // Manually set segment 0 to failed state
        {
            let mut segs = cluster.segments.write().unwrap();
            let (_, health) = &mut segs[0];
            health.consecutive_fails = 15;
            health.state = SegmentState::Failed;
        }
        assert_eq!(cluster.active_segment_count(), 1, "one failed → one active");
        cluster.rejoin_segment(0);
        assert_eq!(cluster.active_segment_count(), 2, "after rejoin → two active");
    }
}
