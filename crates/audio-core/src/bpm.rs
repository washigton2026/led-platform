//! BPM tracking from beat-to-beat intervals. `AudioFeatures::bpm` is documented as
//! "current estimated BPM (smoothed, not instantaneous)" — [`BpmTracker`] keeps a small
//! fixed-size ring of recent inter-beat intervals (no allocation) and EMA-smooths the
//! resulting tempo estimate.

/// Number of recent inter-beat intervals averaged for one tempo estimate.
const HISTORY: usize = 8;

/// Plausible tempo range: ~30-240 BPM. Intervals outside this range (false beats, long
/// silences) are ignored so they don't corrupt the estimate.
const MIN_INTERVAL_MS: f32 = 250.0;
const MAX_INTERVAL_MS: f32 = 2000.0;

/// EMA weight kept for the previous BPM estimate when blending in a new one.
const BPM_DECAY: f32 = 0.8;
const BPM_GAIN: f32 = 0.2;

pub struct BpmTracker {
    last_beat_ms: Option<u64>,
    intervals: [f32; HISTORY],
    len: usize,
    idx: usize,
    bpm: f32,
}

impl Default for BpmTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BpmTracker {
    pub fn new() -> Self {
        Self { last_beat_ms: None, intervals: [0.0; HISTORY], len: 0, idx: 0, bpm: 0.0 }
    }

    /// Call once per analysis frame. `beat` is the [`crate::beat::BeatDetector`] output for
    /// this frame. Returns the current smoothed BPM (0.0 until at least two beats have
    /// landed within the plausible tempo range).
    pub fn update(&mut self, beat: bool, timestamp_ms: u64) -> f32 {
        if beat {
            if let Some(last) = self.last_beat_ms {
                let interval = timestamp_ms.saturating_sub(last) as f32;
                if (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&interval) {
                    self.intervals[self.idx] = interval;
                    self.idx = (self.idx + 1) % HISTORY;
                    self.len = (self.len + 1).min(HISTORY);

                    let avg: f32 = self.intervals[..self.len].iter().sum::<f32>() / self.len as f32;
                    let instant_bpm = 60_000.0 / avg;
                    self.bpm =
                        if self.bpm == 0.0 { instant_bpm } else { self.bpm * BPM_DECAY + instant_bpm * BPM_GAIN };
                }
            }
            self.last_beat_ms = Some(timestamp_ms);
        }
        self.bpm
    }

    pub fn bpm(&self) -> f32 {
        self.bpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_120_bpm_on_a_steady_beat() {
        let mut t = BpmTracker::new();
        let mut bpm = 0.0;
        // 120 BPM = 500 ms per beat.
        for i in 0..16u64 {
            bpm = t.update(true, i * 500);
        }
        assert!((bpm - 120.0).abs() < 1.0, "expected ~120 BPM, got {bpm}");
    }

    #[test]
    fn ignores_implausible_intervals() {
        let mut t = BpmTracker::new();
        // First "beat" then a tiny 10ms gap (way below MIN_INTERVAL_MS) should be ignored.
        t.update(true, 0);
        let bpm = t.update(true, 10);
        assert_eq!(bpm, 0.0, "implausibly short interval must not seed a BPM estimate");
    }

    #[test]
    fn no_beats_means_zero_bpm() {
        let mut t = BpmTracker::new();
        assert_eq!(t.update(false, 1000), 0.0);
    }
}
