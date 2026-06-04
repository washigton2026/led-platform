//! Beat detection by **spectral flux** with a slow-EMA adaptive threshold and a refractory
//! window. Flux = sum of *positive* bin-to-bin increases (onsets push energy up). The
//! threshold is a slow moving average (`avg = avg*0.9 + flux*0.1`) — NOT the other way
//! round — so a beat is "flux clearly above the recent average".

pub struct BeatDetector {
    prev: Vec<f32>,
    flux_avg: f32,
    sensitivity: f32,    // beat when flux > avg * sensitivity
    refractory: u32,     // frames to suppress after a beat (no double-trigger)
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
        Self { prev: Vec::new(), flux_avg: 0.0, sensitivity: 1.5, refractory: 3, cooldown: 0, warmed: false }
    }

    pub fn with_params(sensitivity: f32, refractory: u32) -> Self {
        Self { sensitivity, refractory, ..Self::new() }
    }

    /// Feed one magnitude spectrum; returns true on a detected beat onset.
    pub fn process(&mut self, spectrum: &[f32]) -> bool {
        // Spectral flux: only rising bins count.
        let mut flux = 0.0;
        if self.prev.len() == spectrum.len() {
            for (s, p) in spectrum.iter().zip(&self.prev) {
                let d = s - p;
                if d > 0.0 {
                    flux += d;
                }
            }
        }
        // Update history (clear+extend keeps capacity).
        self.prev.clear();
        self.prev.extend_from_slice(spectrum);

        // A beat needs: history established, not in refractory, flux clearly above the
        // slow average (with a tiny floor so silence→silence never triggers).
        let threshold = (self.flux_avg * self.sensitivity).max(1e-6);
        let is_beat = self.warmed && self.cooldown == 0 && flux > threshold;

        // Slow EMA threshold update (after the comparison).
        self.flux_avg = self.flux_avg * 0.9 + flux * 0.1;
        self.warmed = true;

        if is_beat {
            self.cooldown = self.refractory;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        is_beat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: usize, v: f32) -> Vec<f32> {
        vec![v; n]
    }

    #[test]
    fn fires_on_energy_burst_not_on_silence_or_sustain() {
        let mut d = BeatDetector::with_params(1.5, 2);
        let n = 64;

        // Warm-up on silence: no beats.
        assert!(!d.process(&flat(n, 0.0)));
        assert!(!d.process(&flat(n, 0.0)));

        // Sudden burst → beat (rising flux far above the ~0 average).
        assert!(d.process(&flat(n, 1.0)), "onset should fire a beat");

        // Sustained loud → no beat (no positive flux) and refractory anyway.
        assert!(!d.process(&flat(n, 1.0)));
        assert!(!d.process(&flat(n, 1.0)));

        // Drop to silence → no beat (flux is negative → clamped to 0).
        assert!(!d.process(&flat(n, 0.0)));

        // A second burst later → beat again.
        assert!(!d.process(&flat(n, 0.0)));
        assert!(d.process(&flat(n, 1.0)), "a later onset fires again");
    }

    #[test]
    fn refractory_suppresses_double_triggers() {
        let mut d = BeatDetector::with_params(1.2, 3);
        let n = 32;
        d.process(&flat(n, 0.0));
        // ramp up every frame → flux positive each time, but refractory limits beats
        let mut beats = 0;
        for k in 1..=8 {
            if d.process(&flat(n, k as f32)) {
                beats += 1;
            }
        }
        assert!(beats >= 1, "at least the first onset");
        assert!(beats <= 3, "refractory prevents a beat every single frame, got {beats}");
    }
}
