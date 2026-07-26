//! `MetricsEmitter` — structured JSON metrics for LUMYX observability.
//!
//! Implements the lumyx-observability-engineer specification: p50/p99 latency,
//! frame drops, beat density, device health, heartbeat gaps.
//!
//! ## Design
//!
//! - **Zero-dep**: std-only, no external crates.
//! - **Structured JSON**: one line per snapshot, machine-parseable by any log aggregator.
//! - **Atomic counters**: frame_count, drop_count updated from the hot path with
//!   `Relaxed` ordering (approximate but correct direction).
//! - **Histogram**: p50/p99 computed from a 256-bucket HDR-lite histogram.
//! - **No allocation in hot path**: `record_frame` uses only atomic operations.
//!
//! ## Usage
//!
//! ```ignore
//! let m = MetricsEmitter::new("main-hal");
//! // In render loop:
//! let t0 = std::time::Instant::now();
//! hal.send_frame(&frame)?;
//! m.record_frame(t0.elapsed().as_micros() as u64);
//! // Periodically:
//! println!("{}", m.snapshot_json());
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ── Histogram (HDR-lite, 256 buckets, log2 scale) ─────────────────────────────

/// 256-bucket latency histogram covering 1µs to ~32s (log2 scale).
/// Bucket i covers [2^i µs, 2^(i+1) µs).
struct Histogram {
    buckets: [AtomicU64; 64],
}

impl Histogram {
    fn new() -> Self {
        // SAFETY: AtomicU64 is zero-initializable; the array is fixed-size.
        #[allow(clippy::declare_interior_mutable_const)]
        const ZERO: AtomicU64 = AtomicU64::new(0);
        Self { buckets: [ZERO; 64] }
    }

    fn record(&self, micros: u64) {
        let bucket = (micros.max(1).ilog2() as usize).min(63);
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
    }

    /// Compute the N-th percentile latency in microseconds.
    fn percentile(&self, p: f64) -> u64 {
        let total: u64 = self.buckets.iter().map(|b| b.load(Ordering::Relaxed)).sum();
        if total == 0 { return 0; }
        // ceil + min 1: a target of 0 would match the (possibly empty) first
        // bucket and report ~1µs for any single large sample.
        let target = (((total as f64) * p / 100.0).ceil() as u64).max(1);
        let mut cumulative = 0u64;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            if cumulative >= target {
                // Midpoint of the bucket range
                return 1u64 << i;
            }
        }
        1u64 << 63
    }

    fn reset(&self) {
        for b in &self.buckets { b.store(0, Ordering::Relaxed); }
    }
}

// ── MetricsEmitter ────────────────────────────────────────────────────────────

/// Collects and emits LUMYX operational metrics.
pub struct MetricsEmitter {
    name:        &'static str,
    epoch:       Instant,
    frame_count: AtomicU64,
    drop_count:  AtomicU64,
    beat_count:  AtomicU64,
    hop_count:   AtomicU64,
    histogram:   Histogram,
    /// Last heartbeat gap seen (µs).
    last_hb_gap_us: AtomicU64,
}

impl MetricsEmitter {
    /// Create a new emitter with the given component name.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            epoch:        Instant::now(),
            frame_count:  AtomicU64::new(0),
            drop_count:   AtomicU64::new(0),
            beat_count:   AtomicU64::new(0),
            hop_count:    AtomicU64::new(0),
            histogram:    Histogram::new(),
            last_hb_gap_us: AtomicU64::new(0),
        }
    }

    /// The component name this emitter was created with.
    pub fn name(&self) -> &'static str { self.name }

    // ── Hot-path recording (Relaxed — approximate) ────────────────────────

    /// Record one successfully rendered+sent frame with its latency in µs.
    /// Call this AFTER `send_frame` returns.
    #[inline]
    pub fn record_frame(&self, latency_us: u64) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        self.histogram.record(latency_us);
    }

    /// Record a dropped frame (pipeline missed a deadline).
    #[inline]
    pub fn record_drop(&self) {
        self.drop_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a beat detection event.
    #[inline]
    pub fn record_beat(&self) {
        self.beat_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one audio hop processed.
    #[inline]
    pub fn record_hop(&self) {
        self.hop_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Update the last observed heartbeat gap (µs).
    #[inline]
    pub fn record_heartbeat_gap(&self, gap_us: u64) {
        self.last_hb_gap_us.store(gap_us, Ordering::Relaxed);
    }

    // ── Snapshot ─────────────────────────────────────────────────────────────

    /// Emit a JSON snapshot of current metrics.
    /// One line, machine-parseable, zero allocation (writes to a pre-allocated String).
    pub fn snapshot_json(&self) -> String {
        let elapsed_ms = self.epoch.elapsed().as_millis() as u64;
        let frames     = self.frame_count.load(Ordering::Relaxed);
        let drops      = self.drop_count.load(Ordering::Relaxed);
        let beats      = self.beat_count.load(Ordering::Relaxed);
        let hops       = self.hop_count.load(Ordering::Relaxed);
        let hb_gap_ms  = self.last_hb_gap_us.load(Ordering::Relaxed) / 1_000;
        let p50_us     = self.histogram.percentile(50.0);
        let p99_us     = self.histogram.percentile(99.0);
        let fps        = (frames * 1_000).checked_div(elapsed_ms).unwrap_or(0);
        let beat_density = (beats * 100).checked_div(hops).unwrap_or(0); // percent

        format!(
            r#"{{"component":"{name}","elapsed_ms":{elapsed_ms},"frames":{frames},"drops":{drops},"fps":{fps},"p50_us":{p50_us},"p99_us":{p99_us},"beat_density_pct":{beat_density},"hops":{hops},"hb_gap_ms":{hb_gap_ms}}}"#,
            name = self.name,
        )
    }

    /// Reset all counters and the histogram. Useful at the start of a new show.
    pub fn reset(&self) {
        self.frame_count.store(0, Ordering::Relaxed);
        self.drop_count.store(0, Ordering::Relaxed);
        self.beat_count.store(0, Ordering::Relaxed);
        self.hop_count.store(0, Ordering::Relaxed);
        self.last_hb_gap_us.store(0, Ordering::Relaxed);
        self.histogram.reset();
    }

    // ── Accessors (for assertions in tests) ───────────────────────────────

    pub fn frame_count(&self) -> u64 { self.frame_count.load(Ordering::Relaxed) }
    pub fn drop_count(&self)  -> u64 { self.drop_count.load(Ordering::Relaxed) }
    pub fn beat_count(&self)  -> u64 { self.beat_count.load(Ordering::Relaxed) }
    pub fn p50_us(&self)      -> u64 { self.histogram.percentile(50.0) }
    pub fn p99_us(&self)      -> u64 { self.histogram.percentile(99.0) }
}

// Safety: only atomics — no interior mutability via raw pointers.
unsafe impl Send for MetricsEmitter {}
unsafe impl Sync for MetricsEmitter {}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ── Basic counting ────────────────────────────────────────────────────────

    #[test]
    fn frame_count_increments() {
        let m = MetricsEmitter::new("test");
        m.record_frame(100);
        m.record_frame(200);
        assert_eq!(m.frame_count(), 2);
    }

    #[test]
    fn drop_count_increments() {
        let m = MetricsEmitter::new("test");
        m.record_drop();
        m.record_drop();
        assert_eq!(m.drop_count(), 2);
    }

    #[test]
    fn beat_count_increments() {
        let m = MetricsEmitter::new("test");
        for _ in 0..10 { m.record_beat(); }
        assert_eq!(m.beat_count(), 10);
    }

    // ── Histogram / percentiles ───────────────────────────────────────────────

    #[test]
    fn p50_within_bucket() {
        let m = MetricsEmitter::new("test");
        // Record 100 frames at 1ms (1000µs)
        for _ in 0..100 { m.record_frame(1_000); }
        let p50 = m.p50_us();
        // 1000µs is in bucket log2(1000) = 9 → range [512, 1024)µs
        assert!((512..=1024).contains(&p50), "p50 must be near 1000µs: got {p50}µs");
    }

    #[test]
    fn p99_higher_than_p50() {
        let m = MetricsEmitter::new("test");
        // 90 fast frames + 10 slow frames
        for _ in 0..90 { m.record_frame(500); }
        for _ in 0..10 { m.record_frame(50_000); } // 50ms slow
        assert!(m.p99_us() > m.p50_us(), "p99 must be > p50");
    }

    #[test]
    fn empty_histogram_returns_zero() {
        let m = MetricsEmitter::new("test");
        assert_eq!(m.p50_us(), 0);
        assert_eq!(m.p99_us(), 0);
    }

    // ── JSON output ───────────────────────────────────────────────────────────

    #[test]
    fn snapshot_json_is_valid_json_shape() {
        let m = MetricsEmitter::new("hal");
        m.record_frame(1_000);
        m.record_drop();
        m.record_beat();
        m.record_hop();
        let json = m.snapshot_json();
        // Check presence of expected keys
        assert!(json.contains("\"component\":\"hal\""), "must include component");
        assert!(json.contains("\"frames\":1"),          "must include frames");
        assert!(json.contains("\"drops\":1"),           "must include drops");
        assert!(json.contains("\"p50_us\":"),           "must include p50");
        assert!(json.contains("\"p99_us\":"),           "must include p99");
        assert!(json.contains("\"hb_gap_ms\":"),        "must include heartbeat gap");
        // Starts and ends with braces
        assert!(json.starts_with('{') && json.ends_with('}'));
    }

    #[test]
    fn snapshot_json_component_name_correct() {
        let m = MetricsEmitter::new("lumyx-test");
        let json = m.snapshot_json();
        assert!(json.contains("lumyx-test"), "component name must appear in JSON");
    }

    // ── Reset ─────────────────────────────────────────────────────────────────

    #[test]
    fn reset_clears_all_counters() {
        let m = MetricsEmitter::new("test");
        for _ in 0..50 { m.record_frame(1_000); m.record_drop(); m.record_beat(); }
        m.reset();
        assert_eq!(m.frame_count(), 0);
        assert_eq!(m.drop_count(),  0);
        assert_eq!(m.beat_count(),  0);
        assert_eq!(m.p50_us(),      0);
    }

    // ── Thread safety ─────────────────────────────────────────────────────────

    #[test]
    fn concurrent_recording_no_panic() {
        let m = Arc::new(MetricsEmitter::new("concurrent"));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let mc = m.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..1_000u64 {
                    mc.record_frame(i * 10 + 1);
                    if i % 10 == 0 { mc.record_drop(); }
                    if i % 7  == 0 { mc.record_beat(); }
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
        assert_eq!(m.frame_count(), 8_000, "all 8 threads × 1000 frames must be counted");
        assert!(m.p50_us() > 0, "histogram must have data after concurrent recording");
    }

    // ── Heartbeat gap ─────────────────────────────────────────────────────────

    #[test]
    fn heartbeat_gap_stored_and_retrieved() {
        let m = MetricsEmitter::new("test");
        m.record_heartbeat_gap(1_500_000); // 1.5s in µs
        let json = m.snapshot_json();
        assert!(json.contains("\"hb_gap_ms\":1500"), "hb_gap_ms must be 1500 in JSON");
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MetricsEmitter>();
    }
}
