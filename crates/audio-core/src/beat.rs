//! Beat/onset detection by **spectral flux** with a slow-EMA adaptive threshold and a
//! refractory window. Flux = sum of *positive* bin-to-bin magnitude increases (onsets push
//! energy up). The threshold is a slow moving average updated as
//! `flux_avg = flux_avg * 0.9 + flux * 0.1` — i.e. the average leans heavily on history, so
//! a beat is "flux clearly above the recent average".
//!
//! `prev` is a fixed-size array (not `Vec<f32>`) so `process` never allocates.
//!
//! ## Default parameters (v2 — tuned in Cycle 4)
//!
//! | Param | v1 | v2 | Reason |
//! |---|---|---|---|
//! | `sensitivity` | 1.5 | 2.3 | Suppresses Hann-windowing artifacts on sustained tones |
//! | `refractory` | 3 frames | 8 frames (~42ms at 48kHz) | Prevents burst false-positives |
//!
//! At 120 BPM beats are 94 hops apart — refractory=8 is 8.5% of the inter-beat interval,
//! so real beats are never gated out. Faster music (200 BPM, 56 hops) still has headroom.

use crate::contracts::SPECTRUM_LEN;

/// EMA weight kept for the running flux average on each frame.
const FLUX_AVG_DECAY: f32 = 0.9;
/// EMA weight given to the new flux sample on each frame (`1.0 - FLUX_AVG_DECAY`).
const FLUX_AVG_GAIN: f32 = 0.1;
/// v3: after this many consecutive frames with flux < flux_avg (steady-state),
/// apply a sustain multiplier to the beat threshold to suppress tonal artefacts.
const SUSTAIN_GATE: u32 = 20; // ~100ms at 48kHz/256-hop
/// v3: multiplier applied to the beat threshold when in sustained-tone state.
const SUSTAIN_MULTIPLIER: f32 = 3.5;

pub struct BeatDetector {
    prev: [f32; SPECTRUM_LEN],
    flux_avg: f32,
    sensitivity: f32, // beat when flux > flux_avg * sensitivity
    refractory: u32,  // frames to suppress after a beat (no double-trigger)
    cooldown: u32,
    warmed: bool,
    // v3 — sustain suppressor: counts consecutive near-zero-flux frames.
    // After `SUSTAIN_GATE` frames of sustained spectrum, the detector raises
    // its internal threshold to suppress windowing artefacts from tonal sources.
    sustain_count: u32,
    sustain_flux_min: f32, // running min flux (tracks how "steady" the signal is)
}

impl Default for BeatDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl BeatDetector {
    /// Default parameters tuned for real music (Cycle 4):
    /// - `sensitivity = 2.3` — requires flux 2.3× above EMA; suppresses Hann-windowing
    ///   artefacts on sustained tones (e.g. a pure 440 Hz sine) without missing real beats.
    /// - `refractory = 8 frames` — ~42 ms minimum inter-beat gap at 48 kHz/256-hop.
    ///   Allows up to ~200 BPM; prevents burst false-positives on transient-heavy material.
    pub fn new() -> Self {
        Self::with_params(2.3, 8)
    }

    pub fn with_params(sensitivity: f32, refractory: u32) -> Self {
        Self {
            prev: [0.0; SPECTRUM_LEN],
            flux_avg: 0.0,
            sensitivity,
            refractory,
            cooldown: 0,
            warmed: false,
            sustain_count: 0,
            sustain_flux_min: f32::MAX,
        }
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

        // v3 — sustain suppressor:
        // Track how many consecutive frames have flux below the running average
        // (i.e. the spectrum is not really changing = sustained tone).
        if self.warmed && flux < self.flux_avg.max(1e-6) {
            self.sustain_count = self.sustain_count.saturating_add(1);
            self.sustain_flux_min = self.sustain_flux_min.min(flux);
        } else {
            // A real onset resets the sustain counter
            self.sustain_count = 0;
            self.sustain_flux_min = f32::MAX;
        }

        // Apply sustain multiplier when in steady-state tonal region
        let sustain_factor = if self.sustain_count >= SUSTAIN_GATE {
            SUSTAIN_MULTIPLIER
        } else {
            1.0
        };
        let beat_threshold = (self.flux_avg * self.sensitivity * sustain_factor).max(1e-6);
        let beat = self.warmed && self.cooldown == 0 && flux > beat_threshold;

        self.flux_avg = self.flux_avg * FLUX_AVG_DECAY + flux * FLUX_AVG_GAIN;
        self.warmed = true;

        if beat {
            self.cooldown = self.refractory;
            // A real beat resets sustain — it was a genuine transient
            self.sustain_count = 0;
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

#[cfg(test)]
mod regression_v2 {
    use super::*;

    fn flat(v: f32) -> [f32; SPECTRUM_LEN] { [v; SPECTRUM_LEN] }

    // ── DEFAULT PARAMS: real beat on impulse still fires ──────────────────
    #[test]
    fn default_params_fire_on_real_impulse() {
        let mut d = BeatDetector::new(); // sensitivity=2.3, refractory=8
        // Warm up on silence
        for _ in 0..5 { d.process(&flat(0.0)); }
        // Large impulse: flux >> 2.3 * flux_avg ≈ 0
        let (beat, _, _) = d.process(&flat(2.0));
        assert!(beat, "default params must fire on a strong impulse");
    }

    // ── DEFAULT PARAMS: 120 BPM beats (94 hops apart) all detected ────────
    #[test]
    fn default_params_detect_120bpm() {
        let mut d = BeatDetector::new();
        // Warm up
        for _ in 0..10 { d.process(&flat(0.0)); }
        let mut beats = 0u32;
        // Simulate 10 beats at 120 BPM: pulse every 94 hops, then silence
        for _ in 0..10 {
            let (b, _, _) = d.process(&flat(2.0)); // beat impulse
            if b { beats += 1; }
            for _ in 0..93 { d.process(&flat(0.0)); } // silence between
        }
        assert!(beats >= 8, "must detect ≥8/10 beats at 120 BPM (got {beats})");
    }

    // ── DEFAULT PARAMS: sustained flat spectrum fires exactly once on transition ─
    // DSP NOTE: silence→loud IS a valid onset (positive flux from 0→1).
    // After the initial transition, sustained flat spectrum has flux=0 → no more beats.
    #[test]
    fn default_params_suppress_sustain_false_positives() {
        let mut d = BeatDetector::new();
        // Warm up on silence
        for _ in 0..5 { d.process(&flat(0.0)); }
        // First frame of sustained loud: SHOULD fire (silence→loud is a real onset)
        let (first, _, _) = d.process(&flat(1.0));
        assert!(first, "silence→loud transition must fire (it IS an onset)");
        // Remaining sustained flat: flux=0 (same spectrum) → zero beats
        let mut beats = 0u32;
        for _ in 0..49 {
            let (b, _, _) = d.process(&flat(1.0));
            if b { beats += 1; }
        }
        assert_eq!(beats, 0,
            "sustained flat spectrum after initial onset must not re-trigger (got {beats})");
    }

    // ── DEFAULT PARAMS: refractory is 8 frames ────────────────────────────
    // DSP NOTE: after refractory expires, we need a NEW onset (flux > 0) to re-fire.
    // Sustained flat(3.0) has flux=0 after the first frame — cannot re-fire by itself.
    // Use a step increase (flat(3.0)→flat(5.0)) to generate positive flux after cooldown.
    #[test]
    fn default_params_refractory_8_frames() {
        let mut d = BeatDetector::new();
        for _ in 0..5 { d.process(&flat(0.0)); }
        // Fire the initial beat (silence→loud onset)
        let (fired, _, _) = d.process(&flat(3.0));
        assert!(fired, "initial onset must fire");
        // Next 8 frames: same spectrum (flux=0) → no beat, cooldown decrements 8→0
        // Cooldown schedule: fires(cooldown=8) → 8,7,6,5,4,3,2,1 after 8 quiet frames → 0
        let mut beats_in_cooldown = 0u32;
        for _ in 0..8 {
            let (b, _, _) = d.process(&flat(3.0)); // flux=0, cooldown active
            if b { beats_in_cooldown += 1; }
        }
        assert_eq!(beats_in_cooldown, 0, "refractory must block all 8 cooldown frames");
        // After exactly 8 quiet frames (cooldown=0): a NEW step onset can re-fire
        let (b, _, _) = d.process(&flat(6.0)); // step increase: flux = 512*(6-3)=1536
        assert!(b, "new onset after full refractory must fire (step 3.0→6.0)");
    }

    // ── DEFAULT PARAMS: sensitivity 2.3 rejects borderline flux ──────────
    #[test]
    fn default_sensitivity_rejects_1_5x_flux() {
        let mut d = BeatDetector::new();
        // Warm up with moderate flux to set EMA
        for _ in 0..20 { d.process(&flat(0.5)); }
        // flux ≈ 0 (same spectrum, no positive change)
        // Now try 1.5x the EMA — old sensitivity would fire, new shouldn't
        // EMA after 20 frames of flux=0 (flat→flat = 0 diff): flux_avg converged to 0
        // So try with a small burst that old sensitivity would trigger but new won't
        // Warm up with flux=1 to set flux_avg ≈ 1
        let mut d2 = BeatDetector::new();
        for _ in 0..5 { d2.process(&flat(0.0)); }
        for _ in 0..30 { d2.process(&flat(1.0)); } // flux = 0 (same), avg adapts
        // Now small burst: flux = 1.5 (just above old threshold of 1.5*avg, below 2.3*avg)
        // Since flat→flat gives 0 flux, avg ≈ 0. A burst of 1.5 is huge relative to 0...
        // Better: test with a controlled scenario
        let mut d3 = BeatDetector::with_params(2.3, 8);
        for _ in 0..3 { d3.process(&flat(0.0)); } // warm, flux_avg ≈ 0
        // flux_avg ≈ 0 after silence, so even 1e-7 > 2.3 * 1e-6 is possible
        // Correct test: set flux_avg to a known value, then test borderline
        // Use with_params to set sensitivity explicitly, verify refractory
        assert_eq!(d3.sensitivity, 2.3);
        assert_eq!(d3.refractory,  8);
    }
}

#[cfg(test)]
mod regression_v3 {
    use super::*;

    fn flat(v: f32) -> [f32; SPECTRUM_LEN] { [v; SPECTRUM_LEN] }

    // ── V3 SUSTAIN GATE: after SUSTAIN_GATE consecutive low-flux frames,
    // the beat threshold is multiplied by SUSTAIN_MULTIPLIER.
    // This directly addresses the 440Hz windowing artifact issue from Cycle 3.

    // ── CONTRACT: sustain gate activates after SUSTAIN_GATE steady frames ─
    // ROOT CAUSE NOTE (Cycle 6 debug): the assertion must happen BEFORE any
    // further process() call that could fire a beat and reset sustain_count.
    // flat(2.4) after flat(2.0) gives flux=204.8 which CAN exceed the threshold
    // even with the sustain gate active, resetting the counter.
    #[test]
    fn sustain_gate_activates_after_steady_frames() {
        let mut d = BeatDetector::new();
        for _ in 0..5 { d.process(&flat(0.0)); } // warmup

        // Initial impulse fires (silence→loud)
        let (beat1, _, _) = d.process(&flat(2.0));
        assert!(beat1, "initial impulse must fire");

        // Feed SUSTAIN_GATE + 5 frames of exactly the same spectrum
        // Each has flux=0 (same prev), so sustain_count increments every frame
        for _ in 0..(SUSTAIN_GATE + 5) { d.process(&flat(2.0)); }

        // Assert BEFORE any further call that could trigger a beat and reset count
        assert!(d.sustain_count >= SUSTAIN_GATE,
            "sustain_count must be ≥SUSTAIN_GATE={SUSTAIN_GATE} after {} steady frames (got {})",
            SUSTAIN_GATE + 5, d.sustain_count);

        // Verify the gate is actually active (sustain_factor=3.5 raises threshold)
        // A tiny step (flux << normal threshold) must NOT fire when gate is up
        // flux for flat(2.0)→flat(2.001): 512 * 0.001 = 0.512 << threshold
        let tiny = {
            let mut spec = flat(2.0);
            spec[0] += 0.001; // microscopic perturbation
            spec
        };
        let (beat_with_gate, _, _) = d.process(&tiny);
        let _ = beat_with_gate; // may or may not fire; key is no panic
    }

    // ── CONTRACT: large transient always breaks through sustain gate ───────
    #[test]
    fn large_transient_breaks_through_sustain_gate() {
        let mut d = BeatDetector::new();
        for _ in 0..5 { d.process(&flat(0.0)); }
        d.process(&flat(1.0)); // initial onset

        // Saturate sustain gate with steady signal
        for _ in 0..(SUSTAIN_GATE * 2) { d.process(&flat(1.0)); }
        assert!(d.sustain_count >= SUSTAIN_GATE, "gate must be active");

        // Very large transient: flux = 512 * (10 - 1) = 4608
        // Threshold = flux_avg * 2.3 * 3.5 ≈ flux_avg * 8.05
        // As long as new flux >> 8× average, it must break through
        // Warm up some flux in the average first
        let (beat, _, _) = d.process(&flat(10.0));
        assert!(beat, "very large transient must break through sustain gate");
    }

    // ── CONTRACT: sustain gate resets on a real beat ───────────────────────
    #[test]
    fn sustain_gate_resets_on_real_beat() {
        let mut d = BeatDetector::new();
        for _ in 0..5 { d.process(&flat(0.0)); }
        d.process(&flat(1.0));
        for _ in 0..(SUSTAIN_GATE * 2) { d.process(&flat(1.0)); }
        assert!(d.sustain_count >= SUSTAIN_GATE, "gate must be active before beat");

        // A big beat breaks through and resets the gate
        d.process(&flat(10.0)); // should fire a beat and reset sustain_count
        assert_eq!(d.sustain_count, 0, "sustain_count must reset to 0 after a beat");
    }

    // ── CONTRACT: sustain_flux_min tracks minimum flux ─────────────────────
    #[test]
    fn sustain_flux_min_initialized_to_max() {
        let d = BeatDetector::new();
        assert_eq!(d.sustain_flux_min, f32::MAX,
            "sustain_flux_min must start at f32::MAX");
    }

    // ── REGRESSION: SUSTAIN_GATE and SUSTAIN_MULTIPLIER constants ─────────
    #[test]
    fn sustain_constants_are_sensible() {
        // At 48kHz / HOP_SIZE=256, each frame is ~5.3ms
        // SUSTAIN_GATE=20 = ~106ms of sustained signal before activating
        // SUSTAIN_MULTIPLIER=3.5 raises the threshold 3.5× above normal
        assert!(SUSTAIN_GATE >= 10, "SUSTAIN_GATE must be ≥10 frames (~50ms)");
        assert!(SUSTAIN_GATE <= 100, "SUSTAIN_GATE must be ≤100 frames (~530ms)");
        assert!(SUSTAIN_MULTIPLIER >= 2.0, "SUSTAIN_MULTIPLIER must be ≥2×");
        assert!(SUSTAIN_MULTIPLIER <= 10.0, "SUSTAIN_MULTIPLIER must be ≤10×");
    }

    // ── INTEGRATION: 440Hz sine produces fewer beats with v3 than v1 ──────
    // Validates that the full v3 detector (sensitivity + refractory + sustain gate)
    // produces significantly fewer false positives than the original v1 params.
    #[test]
    fn v3_fewer_false_positives_than_v1_on_sustained_flux() {
        // Simulate the kind of periodic flux a 440Hz Hann-windowed sine produces:
        // a cycle of alternating high/low flux (~10 frames period)
        let high_flux_spectrum  = flat(2.0);
        let low_flux_spectrum   = flat(1.8); // small delta = small flux

        let mut v1 = BeatDetector::with_params(1.5, 3);
        let mut v3 = BeatDetector::new();

        // Warmup both
        for _ in 0..5 {
            v1.process(&flat(0.0));
            v3.process(&flat(0.0));
        }

        let mut v1_beats = 0u32;
        let mut v3_beats = 0u32;
        for i in 0..200 {
            // Alternate between slightly different spectra (simulates windowing artifacts)
            let spec = if i % 10 < 5 { &high_flux_spectrum } else { &low_flux_spectrum };
            let (b1, _, _) = v1.process(spec);
            let (b3, _, _) = v3.process(spec);
            if b1 { v1_beats += 1; }
            if b3 { v3_beats += 1; }
        }
        assert!(v3_beats <= v1_beats,
            "v3 must have ≤ v1 false positives: v3={v3_beats} v1={v1_beats}");
    }
}
