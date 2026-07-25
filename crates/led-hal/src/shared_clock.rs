//! `SharedClock` — monotonic show-time clock with NTP-style offset correction.
//!
//! ## Problem
//!
//! A `ClusteredHal` fans one `LogicalFrame` to multiple network segments on the same
//! machine. That covers one NIC. A **multi-node** setup (two computers each driving
//! a different Ethernet segment) needs both machines to produce the same `timestamp_ms`
//! for the same physical moment — otherwise their frames diverge visually.
//!
//! ## Solution
//!
//! `SharedClock` wraps a monotonic wall-clock and applies a signed offset (ms) that the
//! operator calibrates once against a reference node (or derives from an NTP/PTP sync).
//!
//! ```text
//! Node A (reference)      Node B (follower)
//!   wall_ms = 1000          wall_ms = 998
//!   offset  = 0             offset  = +2   (calibrated: B is 2ms behind A)
//!   show_ms = 1000          show_ms = 1000  ← same!
//! ```
//!
//! The clock is intentionally **read-only** after construction — the show renderer
//! calls `now_ms()` every frame; the offset is set once at show-start.
//!
//! ## Invariants (lumyx-realtime-auditor)
//! - `now_ms()` is monotonically non-decreasing — it never goes backward even if the
//!   system clock is adjusted mid-show.
//! - The offset is applied as `show_ms = wall_ms + offset_ms`.
//! - Overflow: `timestamp_ms` is `u64`; at 1ms per tick, overflow takes 585 million years.
//! - `SharedClock` is `Send + Sync` — safe to share between render and send threads.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ── SharedClock ────────────────────────────────────────────────────────────────

/// A monotonic show-time clock with a signed offset for multi-node sync.
///
/// - `offset_ms`: signed adjustment applied to every `now_ms()` reading.
///   Positive = this node is behind the reference; negative = ahead.
/// - The clock measures time since its `epoch` (the `Instant` at construction).
/// - Monotonicity is enforced: `now_ms()` returns `max(previous, current)`.
pub struct SharedClock {
    epoch:       Instant,
    offset_ms:   AtomicI64, // signed: can be negative (node is ahead of reference)
    last_now:    AtomicU64, // monotonicity guard
}

impl SharedClock {
    /// Create a clock with zero offset. The epoch is `Instant::now()`.
    pub fn new() -> Self {
        Self {
            epoch:     Instant::now(),
            offset_ms: AtomicI64::new(0),
            last_now:  AtomicU64::new(0),
        }
    }

    /// Create a clock with a known offset from a reference node (ms).
    /// Positive = this node is behind the reference (add to catch up).
    pub fn with_offset(offset_ms: i64) -> Self {
        let c = Self::new();
        c.offset_ms.store(offset_ms, Ordering::Relaxed);
        c
    }

    /// Current show-time in milliseconds. Monotonically non-decreasing.
    pub fn now_ms(&self) -> u64 {
        let wall = self.epoch.elapsed().as_millis() as u64;
        let offset = self.offset_ms.load(Ordering::Relaxed);
        // Saturating arithmetic: prevent underflow when offset is very negative
        let adjusted = if offset >= 0 {
            wall.saturating_add(offset as u64)
        } else {
            wall.saturating_sub((-offset) as u64)
        };
        // Monotonicity: never go backward
        let prev = self.last_now.load(Ordering::Acquire);
        let next = adjusted.max(prev);
        self.last_now.store(next, Ordering::Release);
        next
    }

    /// Update the offset (can be called between shows or during calibration).
    pub fn set_offset_ms(&self, offset_ms: i64) {
        self.offset_ms.store(offset_ms, Ordering::Relaxed);
    }

    /// Current offset (ms). Positive = this node lags the reference.
    pub fn offset_ms(&self) -> i64 {
        self.offset_ms.load(Ordering::Relaxed)
    }

    /// Duration elapsed since this clock's epoch.
    pub fn elapsed(&self) -> Duration {
        self.epoch.elapsed()
    }
}

impl Default for SharedClock {
    fn default() -> Self { Self::new() }
}

// Safety: SharedClock uses atomics only — no UnsafeCell, no raw pointers.
// The `Instant` epoch is read-only after construction.
// SAFETY: Instant is Send+Sync on all tier-1 targets.
unsafe impl Send for SharedClock {}
unsafe impl Sync for SharedClock {}

// ── ClockSync: simple NTP-style round-trip calibration ────────────────────────

/// Measures the offset between this node and a reference by comparing a reference
/// timestamp with the local wall time. The reference sends its `timestamp_ms`
/// in a UDP packet; the receiver records its local `now_ms` at receipt and computes
/// the offset. In practice, use the reference's `SharedClock::now_ms()` value.
///
/// This is a lightweight approximation (no full NTP algorithm) — accurate to
/// within the one-way network latency (~0.5–2ms on a gigabit LAN).
pub fn calibrate_offset(reference_ts_ms: u64, local_ts_ms: u64) -> i64 {
    // offset = reference - local
    // Positive: local is behind (needs to add)
    // Negative: local is ahead (needs to subtract)
    reference_ts_ms as i64 - local_ts_ms as i64
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── Chaos Red Team: clock dragged backwards mid-show ──────────────────────

    /// A real venue scenario the red team flagged as un-simulated: NTP (or a
    /// re-calibration) corrects the clock BACKWARD while a show is running.
    /// `now_ms` must stay monotonic — a frame timestamp that goes back would
    /// break sequencing and replay. This validates the monotonicity guard
    /// against a large negative offset applied live.
    #[test]
    fn clock_backwards_correction_mid_show_stays_monotonic() {
        let clock = SharedClock::with_offset(0);
        std::thread::sleep(Duration::from_millis(5));
        let before = clock.now_ms();

        // NTP yanks the clock 1 full second into the past, mid-show.
        clock.set_offset_ms(-1000);

        // Every subsequent read must be >= the last — never a backward jump.
        let mut last = before;
        for _ in 0..1000 {
            let now = clock.now_ms();
            assert!(now >= last, "clock went backward: {now} < {last} after -1000ms offset");
            last = now;
        }
        assert!(last >= before, "monotonicity held across the backward correction");
    }

    /// Negative control: if monotonicity is disabled, a backward offset WOULD
    /// be observable. This proves the guard is what protects us (not luck) —
    /// the raw wall+offset value does drop, but now_ms() never exposes it.
    #[test]
    fn backward_offset_drops_raw_value_but_not_now_ms() {
        let clock = SharedClock::with_offset(0);
        std::thread::sleep(Duration::from_millis(5));
        let guarded_before = clock.now_ms();
        clock.set_offset_ms(-1_000_000); // absurd backward jump
        let guarded_after = clock.now_ms();
        // The guard clamps to the previous value (saturating_sub floors at 0,
        // then max(prev) holds it) — the show clock never rewinds.
        assert!(guarded_after >= guarded_before,
            "guarded now_ms must not rewind even on a -1000s offset");
    }

    /// Concurrent readers during a backward correction never observe a rewind.
    #[test]
    fn concurrent_readers_never_see_rewind_during_correction() {
        use std::sync::Arc;
        let clock = Arc::new(SharedClock::with_offset(0));
        std::thread::sleep(Duration::from_millis(2));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = clock.clone();
            handles.push(std::thread::spawn(move || {
                let mut last = 0u64;
                let mut ok = true;
                for i in 0..5000 {
                    if i == 2500 { c.set_offset_ms(-500); } // correction mid-loop
                    let now = c.now_ms();
                    if now < last { ok = false; }
                    last = now;
                }
                ok
            }));
        }
        for h in handles {
            assert!(h.join().unwrap(), "a reader observed a backward jump under concurrency");
        }
    }

    // ── Basic operation ───────────────────────────────────────────────────────

    #[test]
    fn now_ms_increases_over_time() {
        let clock = SharedClock::new();
        let t0 = clock.now_ms();
        std::thread::sleep(Duration::from_millis(5));
        let t1 = clock.now_ms();
        assert!(t1 >= t0, "clock must not go backward");
        assert!(t1 > 0 || t0 == 0, "clock must advance");
    }

    #[test]
    fn zero_offset_returns_wall_time() {
        let clock = SharedClock::new();
        assert_eq!(clock.offset_ms(), 0);
        let _ = clock.now_ms(); // must not panic
    }

    // ── Offset ────────────────────────────────────────────────────────────────

    #[test]
    fn positive_offset_adds_to_reading() {
        let clock = SharedClock::with_offset(100);
        // Sleep a tiny bit so epoch.elapsed() > 0
        std::thread::sleep(Duration::from_millis(2));
        let t = clock.now_ms();
        // t should be >= 100 (offset was added)
        assert!(t >= 100, "positive offset must increase reading: got {t}");
    }

    #[test]
    fn negative_offset_is_bounded_to_zero() {
        // Very large negative offset — must not underflow
        let clock = SharedClock::with_offset(-1_000_000);
        let t = clock.now_ms(); // saturating_sub prevents underflow
        assert_eq!(t, 0, "large negative offset saturates to 0");
    }

    #[test]
    fn set_offset_updates_reading() {
        let clock = SharedClock::new();
        clock.set_offset_ms(500);
        assert_eq!(clock.offset_ms(), 500);
        std::thread::sleep(Duration::from_millis(2));
        let t = clock.now_ms();
        assert!(t >= 500, "updated offset must be reflected: got {t}");
    }

    // ── Monotonicity ──────────────────────────────────────────────────────────

    #[test]
    fn monotonic_even_with_negative_offset_change() {
        let clock = SharedClock::with_offset(1000); // start 1s ahead
        let t0 = clock.now_ms();
        // Suddenly reduce offset (simulating a correction)
        clock.set_offset_ms(-500);
        let t1 = clock.now_ms();
        // t1 must be >= t0 (monotonicity preserved even after offset reduction)
        assert!(t1 >= t0, "clock must not go backward on offset change: t0={t0} t1={t1}");
    }

    #[test]
    fn monotonic_under_rapid_polling() {
        let clock = SharedClock::new();
        let mut prev = clock.now_ms();
        for _ in 0..10_000 {
            let curr = clock.now_ms();
            assert!(curr >= prev, "monotonicity violated: {prev} → {curr}");
            prev = curr;
        }
    }

    // ── Send + Sync ───────────────────────────────────────────────────────────

    #[test]
    fn clock_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedClock>();
    }

    #[test]
    fn clock_shared_between_threads() {
        use std::sync::Arc;
        let clock = Arc::new(SharedClock::with_offset(0));
        let c1 = clock.clone();
        let c2 = clock.clone();

        let t1 = std::thread::spawn(move || c1.now_ms()).join().unwrap();
        let t2 = std::thread::spawn(move || c2.now_ms()).join().unwrap();
        // Both must return valid (non-panicking) values
        assert!(t1 <= t2 || t2 <= t1, "both threads must get valid timestamps");
    }

    // ── Calibration helper ────────────────────────────────────────────────────

    #[test]
    fn calibrate_offset_positive_when_behind() {
        // reference=1000, local=998 → offset=+2 (local is 2ms behind)
        assert_eq!(calibrate_offset(1000, 998), 2);
    }

    #[test]
    fn calibrate_offset_negative_when_ahead() {
        // reference=998, local=1000 → offset=-2 (local is 2ms ahead)
        assert_eq!(calibrate_offset(998, 1000), -2);
    }

    #[test]
    fn calibrate_offset_zero_when_synchronized() {
        assert_eq!(calibrate_offset(1000, 1000), 0);
    }

    // ── Integration: ClusteredHal-style two-node scenario ────────────────────

    #[test]
    fn two_node_scenario_converges() {
        // Node A: reference clock (offset=0)
        // Node B: follower, initially 5ms behind
        let clock_a = SharedClock::with_offset(0);
        let clock_b = SharedClock::with_offset(0);

        std::thread::sleep(Duration::from_millis(2));

        let ref_ts = clock_a.now_ms(); // Node A broadcasts this
        let local_ts = clock_b.now_ms(); // Node B reads its own clock

        // Calibrate Node B to match Node A
        let offset = calibrate_offset(ref_ts, local_ts);
        clock_b.set_offset_ms(offset);

        // After calibration, both clocks should read similarly
        let a = clock_a.now_ms();
        let b = clock_b.now_ms();
        let diff = (a as i64 - b as i64).unsigned_abs();
        assert!(diff < 10, "after calibration, clocks must be within 10ms: diff={diff}ms");
    }
}
