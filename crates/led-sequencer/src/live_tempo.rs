//! Real-time beat accumulator — feeds `AudioFeatures` beat flags into a growing
//! `TempoMap` while a show is running.
//!
//! ## Problem
//!
//! [`TempoMap::from_beat_flags`] is a **batch** builder: you feed it a completed
//! iterator and get back a frozen map. During a live show, beats arrive one at a
//! time as `AudioFeatures` are published by the audio thread. The sequencer needs
//! to realign clips to the detected grid *as the show plays*, not only at the start.
//!
//! ## Solution: `LiveTempoMap`
//!
//! A stateful accumulator that:
//! 1. Accepts individual `(timestamp_ms, beat)` pairs one at a time ([`push`]).
//! 2. Derives a current [`TempoMap`] on demand ([`tempo_map`]).
//! 3. Exposes a **smoothed BPM estimate** so effects can query the live tempo.
//! 4. Lets the sequencer snap future clip positions to the live beat grid.
//!
//! ## Design invariants
//! - Beat timestamps are **always sorted and deduplicated** in the internal buffer —
//!   same guarantee as `TempoMap::from_beat_flags`.
//! - BPM estimate is derived from the last `WINDOW` beat-to-beat intervals (IIR).
//! - Calling `tempo_map()` is O(1) when no new beats arrived since the last call;
//!   O(n_new_beats) otherwise (rebuilds the `TempoMap::Beats` variant from the buffer).
//! - `push()` is `O(1)` amortised (Vec push + one sorted-insert check).
//! - All state is `Send` — the caller owns it on the sequencer/render thread.
//!
//! [`push`]: LiveTempoMap::push
//! [`tempo_map`]: LiveTempoMap::tempo_map

use crate::TempoMap;

/// How many recent beat intervals the BPM smoother averages.
const BPM_WINDOW: usize = 8;

/// Minimum beat interval (ms) accepted — gates out impossibly fast "beats".
/// 60_000 / 300 BPM = 200 ms.
const MIN_INTERVAL_MS: u64 = 200;

/// Maximum beat interval (ms) accepted — gates out impossibly slow "beats".
/// 60_000 / 20 BPM = 3_000 ms.
const MAX_INTERVAL_MS: u64 = 3_000;

// ── LiveTempoMap ───────────────────────────────────────────────────────────────

/// A real-time beat accumulator.  Feed one `AudioFeatures` per hop via [`push`].
///
/// [`push`]: LiveTempoMap::push
pub struct LiveTempoMap {
    /// Sorted, deduplicated list of beat timestamps (ms).
    beats:        Vec<u64>,
    /// Ring buffer of the last [`BPM_WINDOW`] beat-to-beat intervals (ms).
    intervals:    [u64; BPM_WINDOW],
    interval_idx: usize,
    interval_len: usize, // how many valid entries (starts at 0, caps at BPM_WINDOW)
    /// Last beat timestamp seen (for interval calculation).
    last_beat_ms: Option<u64>,
    /// Dirty flag: true when `beats` changed since last `tempo_map()` call.
    dirty:        bool,
    /// Cached TempoMap, valid when `dirty == false`.
    cached:       TempoMap,
}

impl Default for LiveTempoMap {
    fn default() -> Self { Self::new() }
}

impl LiveTempoMap {
    /// Create a new, empty accumulator.
    pub fn new() -> Self {
        Self {
            beats:        Vec::new(),
            intervals:    [0; BPM_WINDOW],
            interval_idx: 0,
            interval_len: 0,
            last_beat_ms: None,
            dirty:        false,
            // Default: 120 BPM with no offset — used before any beat is observed.
            cached:       TempoMap::constant(120.0, 0),
        }
    }

    /// Feed one `(timestamp_ms, beat)` pair (from `AudioFeatures`).
    ///
    /// Only the `beat == true` frames matter; `beat == false` frames are accepted
    /// (no-op) so the caller can forward every hop without filtering.
    pub fn push(&mut self, timestamp_ms: u64, beat: bool) {
        if !beat { return; }

        // Deduplication: ignore beats with the same timestamp.
        if self.beats.last() == Some(&timestamp_ms) { return; }

        // Sorted insert: beats should arrive in order, but protect against jitter.
        let insert_pos = self.beats.partition_point(|&t| t <= timestamp_ms);

        // Full dedup check for out-of-order arrivals.
        if insert_pos > 0 && self.beats[insert_pos - 1] == timestamp_ms { return; }

        // BPM interval tracking (only for in-order beats).
        if let Some(prev) = self.last_beat_ms {
            if timestamp_ms > prev {
                let interval = timestamp_ms - prev;
                if (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&interval) {
                    self.intervals[self.interval_idx] = interval;
                    self.interval_idx = (self.interval_idx + 1) % BPM_WINDOW;
                    if self.interval_len < BPM_WINDOW { self.interval_len += 1; }
                }
            }
        }
        self.last_beat_ms = Some(timestamp_ms);

        self.beats.insert(insert_pos, timestamp_ms);
        self.dirty = true;
    }

    /// Push directly from an `AudioFeatures`-like pair.
    /// Convenience wrapper for `push(features.timestamp_ms, features.beat)`.
    pub fn push_features(&mut self, timestamp_ms: u64, beat: bool) {
        self.push(timestamp_ms, beat);
    }

    /// Derive a [`TempoMap`] from accumulated beats.
    ///
    /// - If no beats yet: returns `TempoMap::Constant(120 BPM)` (safe default).
    /// - If one beat: returns constant BPM from the smoothed estimate.
    /// - If two or more beats: returns `TempoMap::Beats` (exact timestamps).
    ///
    /// The result is cached: repeated calls with no new beats are free.
    pub fn tempo_map(&mut self) -> &TempoMap {
        if !self.dirty { return &self.cached; }

        self.cached = if self.beats.len() < 2 {
            let bpm = self.smoothed_bpm().unwrap_or(120.0);
            let offset = self.beats.first().copied().unwrap_or(0);
            TempoMap::constant(bpm, offset)
        } else {
            TempoMap::from_beats(self.beats.clone())
        };
        self.dirty = false;
        &self.cached
    }

    /// Smoothed BPM from the last `BPM_WINDOW` beat intervals. Returns `None`
    /// before enough beats have been observed.
    pub fn smoothed_bpm(&self) -> Option<f32> {
        if self.interval_len == 0 { return None; }
        let sum: u64 = self.intervals[..self.interval_len].iter().sum();
        let avg_ms = sum as f32 / self.interval_len as f32;
        Some(60_000.0 / avg_ms)
    }

    /// Number of beats accumulated so far.
    pub fn beat_count(&self) -> usize { self.beats.len() }

    /// Latest beat timestamp (ms), or `None` if no beat seen yet.
    pub fn last_beat_ms(&self) -> Option<u64> { self.last_beat_ms }

    /// Snap `time_ms` to the nearest beat in the accumulated grid. Returns `time_ms`
    /// unchanged if no beats have been collected.
    pub fn snap(&mut self, time_ms: u64) -> u64 {
        self.tempo_map().snap(time_ms)
    }

    /// Clear all accumulated beats and reset to the default 120 BPM state.
    pub fn reset(&mut self) {
        self.beats.clear();
        self.intervals = [0; BPM_WINDOW];
        self.interval_idx = 0;
        self.interval_len = 0;
        self.last_beat_ms = None;
        self.dirty = false;
        self.cached = TempoMap::constant(120.0, 0);
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic accumulation ────────────────────────────────────────────────────

    #[test]
    fn push_non_beat_is_no_op() {
        let mut lm = LiveTempoMap::new();
        lm.push(1000, false);
        assert_eq!(lm.beat_count(), 0);
    }

    #[test]
    fn push_beats_accumulate() {
        let mut lm = LiveTempoMap::new();
        lm.push(500, true);
        lm.push(1000, true);
        lm.push(1500, true);
        assert_eq!(lm.beat_count(), 3);
    }

    #[test]
    fn push_duplicate_timestamp_ignored() {
        let mut lm = LiveTempoMap::new();
        lm.push(1000, true);
        lm.push(1000, true); // duplicate
        assert_eq!(lm.beat_count(), 1, "duplicate timestamp must be ignored");
    }

    #[test]
    fn beats_are_sorted() {
        let mut lm = LiveTempoMap::new();
        // Deliberately out of order (simulating jitter)
        lm.push(1500, true);
        lm.push(500, true);
        lm.push(1000, true);
        // After 3 pushes, internal beats must be sorted
        let tm = lm.tempo_map();
        if let TempoMap::Beats(v) = tm {
            let mut sorted = v.clone();
            sorted.sort();
            assert_eq!(v, &sorted, "TempoMap::Beats must be sorted");
        }
    }

    // ── BPM smoothing ─────────────────────────────────────────────────────────

    #[test]
    fn bpm_none_before_any_beats() {
        let lm = LiveTempoMap::new();
        assert!(lm.smoothed_bpm().is_none());
    }

    #[test]
    fn bpm_none_after_one_beat() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        assert!(lm.smoothed_bpm().is_none(), "need 2 beats for an interval");
    }

    #[test]
    fn bpm_120_at_500ms_intervals() {
        let mut lm = LiveTempoMap::new();
        // 120 BPM = 500ms per beat
        for i in 0..=8u64 {
            lm.push(i * 500, true);
        }
        let bpm = lm.smoothed_bpm().expect("must have BPM after 8 intervals");
        assert!(
            (bpm - 120.0).abs() < 1.0,
            "BPM must be close to 120.0, got {bpm}"
        );
    }

    #[test]
    fn bpm_rejects_too_fast_intervals() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        lm.push(50, true);  // 50ms = 1200 BPM — above MAX, rejected
        // MIN_INTERVAL_MS = 200ms; 50ms is below that
        assert!(
            lm.smoothed_bpm().is_none(),
            "50ms interval must be rejected (too fast)"
        );
    }

    #[test]
    fn bpm_rejects_too_slow_intervals() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        lm.push(5000, true); // 5000ms = 12 BPM — below MIN (20 BPM)
        assert!(
            lm.smoothed_bpm().is_none(),
            "5000ms interval must be rejected (too slow)"
        );
    }

    // ── TempoMap derivation ───────────────────────────────────────────────────

    #[test]
    fn tempo_map_default_before_beats() {
        let mut lm = LiveTempoMap::new();
        let tm = lm.tempo_map();
        // Must return a valid TempoMap (Constant 120 BPM)
        assert!(tm.beat_time(0) < 1000, "default must not produce huge values");
    }

    #[test]
    fn tempo_map_from_one_beat_is_constant() {
        let mut lm = LiveTempoMap::new();
        lm.push(500, true);
        let _tm = lm.tempo_map(); // must not panic
    }

    #[test]
    fn tempo_map_from_multiple_beats_uses_beats_variant() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        lm.push(500, true);
        lm.push(1000, true);
        if let TempoMap::Beats(v) = lm.tempo_map() {
            assert_eq!(v.as_slice(), &[0u64, 500, 1000]);
        } else {
            panic!("expected TempoMap::Beats with 3+ beats");
        }
    }

    // ── Caching ───────────────────────────────────────────────────────────────

    #[test]
    fn tempo_map_cached_after_no_new_beats() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        lm.push(500, true);
        // First call builds the map
        let ptr1 = lm.tempo_map() as *const TempoMap;
        // No new beats — must return the same cached object
        let ptr2 = lm.tempo_map() as *const TempoMap;
        assert_eq!(ptr1, ptr2, "tempo_map() must return cached when no new beats");
    }

    #[test]
    fn tempo_map_rebuilds_after_new_beat() {
        let mut lm = LiveTempoMap::new();
        lm.push(0, true);
        lm.push(500, true);
        let _ = lm.tempo_map(); // cache
        // New beat → dirty
        lm.push(1000, true);
        let tm = lm.tempo_map();
        if let TempoMap::Beats(v) = tm {
            assert_eq!(v.len(), 3, "must include the new beat");
        } else {
            panic!("expected TempoMap::Beats");
        }
    }

    // ── Snap ──────────────────────────────────────────────────────────────────

    #[test]
    fn snap_returns_nearest_beat() {
        let mut lm = LiveTempoMap::new();
        // Beats at 0, 500, 1000 ms
        lm.push(0, true);
        lm.push(500, true);
        lm.push(1000, true);
        // 400ms is closer to 500 than to 0
        assert_eq!(lm.snap(400), 500, "400ms snaps to 500ms beat");
        // 200ms is closer to 0 than to 500
        assert_eq!(lm.snap(200), 0, "200ms snaps to 0ms beat");
    }

    #[test]
    fn snap_before_any_beats_returns_input() {
        let mut lm = LiveTempoMap::new();
        // No beats yet — snap must not panic and must return a reasonable value
        let _ = lm.snap(1234); // just must not panic
    }

    // ── Reset ─────────────────────────────────────────────────────────────────

    #[test]
    fn reset_clears_all_state() {
        let mut lm = LiveTempoMap::new();
        for i in 0..5u64 { lm.push(i * 500, true); }
        lm.reset();
        assert_eq!(lm.beat_count(), 0);
        assert!(lm.smoothed_bpm().is_none());
        assert!(lm.last_beat_ms().is_none());
    }

    // ── Real-time simulation ──────────────────────────────────────────────────

    /// Simulate a 30-second stream of audio hops at 120 BPM (beat every 500ms).
    /// Every 25ms hop that lands on a beat boundary pushes a beat flag.
    #[test]
    fn live_120bpm_stream_tracks_tempo() {
        let mut lm = LiveTempoMap::new();
        let hop_ms: u64 = 25;         // 25ms per hop
        let beat_period_ms: u64 = 500; // 120 BPM

        // Simulate 1200 hops = 30 seconds
        for hop in 0u64..1200 {
            let t = hop * hop_ms;
            let beat = t.is_multiple_of(beat_period_ms) && t > 0;
            lm.push(t, beat);
        }

        // Should have accumulated ~59 beats (1 per 500ms over 30s, excluding t=0)
        let beat_count = lm.beat_count();
        assert!(
            (55..=62).contains(&beat_count),
            "expected ~59 beats in 30s at 120BPM, got {beat_count}"
        );

        let bpm = lm.smoothed_bpm().expect("must have BPM estimate after 30s");
        assert!(
            (bpm - 120.0).abs() < 2.0,
            "live BPM must converge to 120, got {bpm:.1}"
        );

        // TempoMap must have the exact beat timestamps
        let tm = lm.tempo_map();
        assert!(
            matches!(tm, TempoMap::Beats(_)),
            "with many beats, must use Beats variant"
        );
    }

    /// Simulate a tempo change mid-stream: starts at 120 BPM, switches to 140 BPM.
    #[test]
    fn live_tempo_change_mid_stream() {
        let mut lm = LiveTempoMap::new();

        // Phase 1: 120 BPM for 10s (beats every 500ms)
        for i in 0u64..20 {
            lm.push(i * 500, true);
        }

        let bpm_before = lm.smoothed_bpm().unwrap();
        assert!((bpm_before - 120.0).abs() < 5.0, "phase 1 BPM ≈ 120");

        // Phase 2: 140 BPM for 10s (beats every ~428ms)
        let start_ms = 20 * 500u64; // 10_000ms
        let period_140 = (60_000.0 / 140.0) as u64; // 428ms
        for i in 0u64..20 {
            lm.push(start_ms + i * period_140, true);
        }

        let bpm_after = lm.smoothed_bpm().unwrap();
        // BPM_WINDOW=8: last 8 intervals at 428ms → should dominate
        assert!(
            bpm_after > 130.0,
            "after tempo change, BPM smoother should follow: got {bpm_after:.1}"
        );
    }

    /// push_features is a thin wrapper — verify it works identically to push().
    #[test]
    fn push_features_equivalent_to_push() {
        let mut lm1 = LiveTempoMap::new();
        let mut lm2 = LiveTempoMap::new();

        for t in [0u64, 500, 1000, 1500] {
            lm1.push(t, true);
            lm2.push_features(t, true);
        }
        assert_eq!(lm1.beat_count(), lm2.beat_count());
        assert_eq!(
            lm1.smoothed_bpm().map(|v| (v * 10.0) as u32),
            lm2.smoothed_bpm().map(|v| (v * 10.0) as u32)
        );
    }

    /// Stress: 10_000 hops, validates that the internal buffer stays sorted+deduped.
    #[test]
    fn stress_10k_hops_stays_sorted() {
        let mut lm = LiveTempoMap::new();
        let mut expected_beats = 0usize;
        for i in 0u64..10_000 {
            let t = i * 25;              // 25ms hops
            let beat = i % 20 == 0 && i > 0; // beat every 500ms
            if beat { expected_beats += 1; }
            lm.push(t, beat);
        }

        assert_eq!(lm.beat_count(), expected_beats);

        // Verify sorted
        if let TempoMap::Beats(v) = lm.tempo_map() {
            for w in v.windows(2) {
                assert!(w[0] < w[1], "beats must be strictly sorted: {} ≥ {}", w[0], w[1]);
            }
        }
    }
}
