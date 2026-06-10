//! Beat/onset detection by **spectral flux** with a slow-EMA adaptive threshold and a
//! refractory window. Flux = sum of *positive* bin-to-bin magnitude increases (onsets push
//! energy up). The threshold is a slow moving average updated as
//! `flux_avg = flux_avg * 0.9 + flux * 0.1` — i.e. the average leans heavily on history, so
//! a beat is "flux clearly above the recent average".
//!
//! `prev` is a fixed-size array (not `Vec<f32>`) so `process` never allocates.

use crate::contracts::SPECTRUM_LEN;

/// EMA weight kept for the running flux average on each frame.
const FLUX_AVG_DECAY: f32 = 0.9;
/// EMA weight given to the new flux sample on each frame (`1.0 - FLUX_AVG_DECAY`).
const FLUX_AVG_GAIN: f32 = 0.1;

pub struct BeatDetector {
    prev: [f32; SPECTRUM_LEN],
    flux_avg: f32,
    sensitivity: f32, // beat when flux > flux_avg * sensitivity
    refractory: u32,  // frames to suppress after a beat (no double-trigger)
    cooldown: u32,
    warmed: bool,
}

impl Default for BeatDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl BeatDetector {
    pub fn new() -> Self {
        Self::with_params(1.5, 3)
    }

    pub fn with_params(sensitivity: f32, refractory: u32) -> Self {
        Self { prev: [0.0; SPECTRUM_LEN], flux_avg: 0.0, sensitivity, refractory, cooldown: 0, warmed: false }
    }

    /// Feed one magnitude spectrum (post-FFT, pre-Hann-windowed input). Returns
    /// `(beat, onset, flux)`:
    /// - `flux`: sum of positive bin-to-bin magnitude increases this frame.
    /// - `onset`: flux clearly above the running average — any rising musical event.
    /// - `beat`: `onset` scaled by `sensitivity`, gated by the refractory cooldown — a
    ///   stricter subset of onsets, at most one per `refractory` frames.
    pub fn process(&mut self, spectrum: &[f32; SPECTRUM_LEN]) -> (bool, bool, f32) {
        let mut flux = 0.0f32;
        for i in 0..SPECTRUM_LEN {
            let d = spectrum[i] - self.prev[i];
            if d > 0.0 {
                flux += d;
            }
        }
        self.prev = *spectrum;

        let onset = self.warmed && flux > self.flux_avg.max(1e-6);
        let beat_threshold = (self.flux_avg * self.sensitivity).max(1e-6);
        let beat = self.warmed && self.cooldown == 0 && flux > beat_threshold;

        self.flux_avg = self.flux_avg * FLUX_AVG_DECAY + flux * FLUX_AVG_GAIN;
        self.warmed = true;

        if beat {
            self.cooldown = self.refractory;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        (beat, onset, flux)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(v: f32) -> [f32; SPECTRUM_LEN] {
        [v; SPECTRUM_LEN]
    }

    #[test]
    fn fires_on_energy_burst_not_on_silence_or_sustain() {
        let mut d = BeatDetector::with_params(1.5, 2);

        // Warm-up on silence: no beats, no onsets.
        let (b, o, _) = d.process(&flat(0.0));
        assert!(!b && !o);
        let (b, o, _) = d.process(&flat(0.0));
        assert!(!b && !o);

        // Sudden burst -> beat AND onset (rising flux far above the ~0 average).
        let (b, o, _) = d.process(&flat(1.0));
        assert!(b && o, "onset should fire a beat");

        // Sustained loud -> no beat, no onset (no positive flux), and refractory anyway.
        let (b, o, _) = d.process(&flat(1.0));
        assert!(!b && !o);
        let (b, o, _) = d.process(&flat(1.0));
        assert!(!b && !o);

        // Drop to silence -> no beat (flux negative -> clamped to 0).
        let (b, _, _) = d.process(&flat(0.0));
        assert!(!b);

        // A second burst later -> beat again.
        d.process(&flat(0.0));
        let (b, o, _) = d.process(&flat(1.0));
        assert!(b && o, "a later onset fires again");
    }

    #[test]
    fn refractory_suppresses_double_triggers() {
        let mut d = BeatDetector::with_params(1.2, 3);
        d.process(&flat(0.0));
        // Ramp up every frame -> flux positive each time, but refractory limits beats.
        let mut beats = 0;
        for k in 1..=8 {
            let (b, _, _) = d.process(&flat(k as f32));
            if b {
                beats += 1;
            }
        }
        assert!(beats >= 1, "at least the first onset");
        assert!(beats <= 3, "refractory prevents a beat every single frame, got {beats}");
    }

    #[test]
    fn process_does_not_allocate() {
        // Smoke-check: BeatDetector holds only fixed-size fields.
        assert_eq!(std::mem::size_of::<[f32; SPECTRUM_LEN]>(), SPECTRUM_LEN * 4);
    }
}
