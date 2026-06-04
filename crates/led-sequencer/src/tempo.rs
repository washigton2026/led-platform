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

    /// The time of the beat nearest to `t` (i.e. snap `t` onto the grid).
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
                _ => *v.iter().min_by_key(|&&b| (b as i64 - t as i64).abs()).unwrap(),
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
