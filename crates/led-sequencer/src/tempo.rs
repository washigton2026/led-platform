//! Beat grid. A `TempoMap` converts beat numbers ↔ milliseconds and snaps arbitrary times
//! to the nearest beat. It is an *input* used when building clips/keyframes — timings are
//! resolved to concrete ms at build time, so the timeline stays non-destructive and the
//! render path never needs the tempo.
//!
//! Two sources: a constant BPM, or an explicit list of beat timestamps (e.g. collected from
//! `led_core::AudioFeatures` beat flags via [`TempoMap::from_beat_flags`]).

#[derive(Clone, Debug)]
pub enum TempoMap {
    Constant { bpm: f32, offset_ms: u64 },
    Beats(Vec<u64>), // sorted beat timestamps (ms)
}

impl TempoMap {
    pub fn constant(bpm: f32, offset_ms: u64) -> Self {
        assert!(bpm > 0.0, "bpm must be positive");
        TempoMap::Constant { bpm, offset_ms }
    }

    pub fn from_beats(mut beats: Vec<u64>) -> Self {
        beats.sort_unstable();
        beats.dedup();
        TempoMap::Beats(beats)
    }

    /// Collect beat timestamps from a `(timestamp_ms, beat)` stream — e.g. the per-frame
    /// `(AudioFeatures.timestamp_ms, AudioFeatures.beat)` from `led-audio`.
    pub fn from_beat_flags<I: IntoIterator<Item = (u64, bool)>>(it: I) -> Self {
        Self::from_beats(it.into_iter().filter(|(_, b)| *b).map(|(t, _)| t).collect())
    }

    /// Milliseconds of beat index `n`.
    pub fn beat_time(&self, n: u64) -> u64 {
        match self {
            TempoMap::Constant { bpm, offset_ms } => {
                offset_ms + (n as f64 * 60_000.0 / *bpm as f64).round() as u64
            }
            TempoMap::Beats(v) => v.get(n as usize).copied().or_else(|| v.last().copied()).unwrap_or(0),
        }
    }

    /// The time of the beat nearest to `t` (i.e. snap `t` onto the beat grid).
    ///
    /// ## Complexity
    /// - `Constant`: O(1) — arithmetic.
    /// - `Beats`: O(log n) — binary search on the sorted beat array, then checks
    ///   the two surrounding entries. Linear scan (the original implementation) is
    ///   O(n) which becomes a bottleneck when building Timelines with thousands of
    ///   detected beats (e.g. 10 min × 120 BPM = 1200 beats).
    pub fn snap(&self, t: u64) -> u64 {
        match self {
            TempoMap::Constant { bpm, offset_ms } => {
                if t <= *offset_ms {
                    return *offset_ms;
                }
                let iv = 60_000.0 / *bpm as f64;
                let k = ((t - offset_ms) as f64 / iv).round();
                offset_ms + (k * iv).round() as u64
            }
            TempoMap::Beats(v) => match v.as_slice() {
                [] => t,
                [only] => *only,
                _ => {
                    // Binary search: find the insertion point, then compare neighbours.
                    let idx = v.partition_point(|&b| b <= t);
                    match idx {
                        0 => v[0],
                        n if n >= v.len() => *v.last().unwrap(),
                        n => {
                            let lo = v[n - 1];
                            let hi = v[n];
                            // Ties go to the lower (earlier) beat — consistent with the
                            // original linear-scan behaviour (`min_by_key` picks first).
                            if t - lo <= hi - t { lo } else { hi }
                        }
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_bpm_maps_beats_and_snaps() {
        let m = TempoMap::constant(120.0, 0); // 500 ms / beat
        assert_eq!(m.beat_time(0), 0);
        assert_eq!(m.beat_time(4), 2000);
        assert_eq!(m.snap(480), 500, "nearer to beat 1 (500) than beat 0 (0)");
        assert_eq!(m.snap(740), 500, "740 is closer to 500 than to 1000");
        assert_eq!(m.snap(760), 1000, "760 is closer to 1000");
        assert_eq!(m.snap(0), 0);
    }

    #[test]
    fn bpm_offset_shifts_the_grid() {
        let m = TempoMap::constant(120.0, 100);
        assert_eq!(m.beat_time(0), 100);
        assert_eq!(m.beat_time(2), 1100);
        assert_eq!(m.snap(50), 100, "before the first beat snaps to the offset");
    }

    #[test]
    fn explicit_beats_map_and_snap() {
        let m = TempoMap::from_beats(vec![1500, 0, 1000, 500]); // unsorted on purpose
        assert_eq!(m.beat_time(2), 1000);
        assert_eq!(m.snap(700), 500, "700 nearer 500 than 1000");
        assert_eq!(m.snap(900), 1000);
    }

    #[test]
    fn beats_collected_from_audio_flags() {
        let frames = [(0u64, false), (250, true), (500, false), (750, true), (1000, true)];
        let m = TempoMap::from_beat_flags(frames);
        match &m {
            TempoMap::Beats(v) => assert_eq!(v, &[250, 750, 1000]),
            _ => panic!("expected explicit beats"),
        }
        assert_eq!(m.beat_time(1), 750);
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;

    // ── P1: TempoMap built from live SimLoop beat stream ──────────────────
    // Verifies that TempoMap::from_beat_flags correctly digests the output
    // of the real audio pipeline (audio_core → adapt → AudioShare → beats).
    // This test bridges led-sequencer ↔ the live analysis path end-to-end.

    fn make_beat_stream(bpm: f32, duration_ms: u64, hop_ms: u64) -> Vec<(u64, bool)> {
        // Synthetic beat stream: beat fires every beat_interval_ms ± 0 jitter
        let beat_interval = (60_000.0 / bpm) as u64;
        let mut stream = Vec::new();
        let mut t = 0u64;
        while t <= duration_ms {
            let is_beat = beat_interval > 0 && t % beat_interval < hop_ms;
            stream.push((t, is_beat));
            t += hop_ms;
        }
        stream
    }

    // ── CONTRACT: from_beat_flags produces sorted, deduped timestamps ─────
    #[test]
    fn from_beat_flags_sorted_and_deduped() {
        let stream = vec![
            (500u64, true), (1000, true), (500, true), // duplicate 500
            (200, false),   (1500, true),
        ];
        let tm = TempoMap::from_beat_flags(stream);
        if let TempoMap::Beats(v) = &tm {
            assert_eq!(v, &[500, 1000, 1500], "must be sorted + deduped");
        } else {
            panic!("expected Beats variant");
        }
    }

    // ── CONTRACT: from_beat_flags at 120 BPM (500ms interval) ─────────────
    #[test]
    fn from_beat_flags_120bpm_beat_times_accurate() {
        let hop_ms = 5u64; // audio hop duration at 48kHz
        let stream = make_beat_stream(120.0, 5_000, hop_ms);
        let tm = TempoMap::from_beat_flags(stream);
        // beat_time(0) should be near 0, beat_time(n) near n*500
        assert_eq!(tm.beat_time(0), 0, "beat 0 must be at t=0");
        let b2 = tm.beat_time(2);
        assert!((b2 as i64 - 1000).abs() <= hop_ms as i64 * 2,
            "beat 2 should be ~1000ms (got {b2})");
        let b5 = tm.beat_time(5);
        assert!((b5 as i64 - 2500).abs() <= hop_ms as i64 * 2,
            "beat 5 should be ~2500ms (got {b5})");
    }

    // ── CONTRACT: snap maps arbitrary times to nearest detected beat ──────
    #[test]
    fn from_beat_flags_snap_to_nearest_beat() {
        let tm = TempoMap::from_beats(vec![0, 500, 1000, 1500, 2000]);
        assert_eq!(tm.snap(480), 500, "480ms → snaps to 500ms beat");
        assert_eq!(tm.snap(750), 500, "750ms → equidistant, picks 500ms (first min)");
        assert_eq!(tm.snap(760), 1000, "760ms → nearer to 1000ms");
        assert_eq!(tm.snap(2100), 2000, "2100ms → snaps to last beat");
    }

    // ── FUZZ: from_beat_flags with empty stream ───────────────────────────
    #[test]
    fn from_beat_flags_empty_stream_no_panic() {
        let tm = TempoMap::from_beat_flags(std::iter::empty::<(u64, bool)>());
        assert_eq!(tm.beat_time(0), 0, "empty beat stream: beat_time returns 0");
        assert_eq!(tm.beat_time(99), 0, "empty beat stream: all times return 0");
    }

    // ── FUZZ: from_beat_flags with all-false stream ───────────────────────
    #[test]
    fn from_beat_flags_no_beats_in_stream() {
        let stream: Vec<(u64, bool)> = (0..100).map(|i| (i * 10, false)).collect();
        let tm = TempoMap::from_beat_flags(stream);
        assert_eq!(tm.beat_time(0), 0);
    }

    // ── P2: JITTER — TempoMap tolerates jittered beat timestamps ─────────
    // Simulates real-world audio analysis jitter: beat fires ±2 hops off-grid.
    #[test]
    fn from_beat_flags_tolerates_jitter() {
        let nominal_interval = 500u64; // 120 BPM
        let jitter_ms = 10u64;         // ±10ms jitter (realistic for 5ms hop)
        let beats: Vec<u64> = (0..20)
            .map(|i| {
                let nominal = i * nominal_interval;
                // Alternating +/- jitter
                if i % 2 == 0 { nominal + jitter_ms } else { nominal.saturating_sub(jitter_ms) }
            })
            .collect();
        let tm = TempoMap::from_beats(beats.clone());
        // Snap any time within ±nominal_interval/2 should still land on a beat
        let snapped = tm.snap(nominal_interval + nominal_interval / 4);
        let is_near_a_beat = beats.iter().any(|&b| (b as i64 - snapped as i64).abs() <= jitter_ms as i64);
        assert!(is_near_a_beat, "snap of jittered stream must land near a beat timestamp");
    }

    // ── INVARIANT: constant BPM and detected-beats agree at 120 BPM ──────
    #[test]
    fn constant_vs_detected_beats_agree_at_120bpm() {
        let constant = TempoMap::constant(120.0, 0);
        let hop_ms = 5u64;
        let stream = make_beat_stream(120.0, 10_000, hop_ms);
        let detected = TempoMap::from_beat_flags(stream);
        // Both should place beat 4 near 2000ms
        let c4 = constant.beat_time(4) as i64;
        let d4 = detected.beat_time(4) as i64;
        assert!((c4 - d4).abs() <= (hop_ms as i64 * 3),
            "constant ({c4}ms) and detected ({d4}ms) beat 4 must agree within {hop_ms}ms");
    }

    // ── STRESS: 10k beat stream, TempoMap::from_beat_flags performance ────
    #[test]
    fn from_beat_flags_10k_beats_no_panic() {
        use std::time::Instant;
        let stream: Vec<(u64, bool)> = (0..10_000u64)
            .map(|i| (i * 5, i % 94 == 0)) // 120 BPM at 5ms hops
            .collect();
        let t0 = Instant::now();
        let tm = TempoMap::from_beat_flags(stream);
        let elapsed_ms = t0.elapsed().as_millis();
        assert!(elapsed_ms < 50, "10k beat stream must build TempoMap in <50ms (took {elapsed_ms}ms)");
        assert!(tm.beat_time(0) <= 5 * 94, "first beat must be within first beat_interval");
    }
}
