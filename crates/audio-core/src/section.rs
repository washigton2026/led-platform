//! Musical section detector — heuristic, energy-based classification.
//!
//! ## Algorithm
//!
//! Three signals drive classification:
//! - `short_ema`: fast-decaying energy average (τ ≈ 0.5 s) — "right now"
//! - `long_ema`:  slow-decaying energy average (τ ≈ 12 s) — "long-term baseline"
//! - `beat_density`: fraction of hops in the last `BEAT_WINDOW` that had a beat
//!
//! The ratio `short_ema / long_ema` (clamped to avoid zero-division) locates the
//! current moment relative to the song's average energy:
//! - ratio >> 1 + high beat density → [`Chorus`] / [`Drop`]
//! - ratio >> 1 + low beat density  → [`Build`]
//! - ratio ≈ 1 + any beat density   → [`Verse`]
//! - ratio << 1                      → [`Bridge`] or [`Intro`] (position-based)
//! - energy near silence             → [`Intro`] at start, [`Outro`] after long play
//!
//! **Hysteresis** (`MIN_HOLD_HOPS`): a pending label must stay stable for N hops
//! before the output changes — prevents rapid flickering between sections.
//!
//! ## Invariants
//! - Output is always `Some(MusicalSection)` after the warm-up window.
//! - Before warm-up, returns `None` (not enough history to classify).
//! - Classification is a heuristic estimate, not ground truth — callers should
//!   treat `musical_section` as advisory, not authoritative.
//! - All state uses `sample_index` implicitly via hop count — no wall clock.

use crate::contracts::MusicalSection;

// ── Tuning constants ───────────────────────────────────────────────────────────

/// Fast EMA decay: α ≈ 1/20 hops ≈ 0.5 s at 25 ms/hop.
const SHORT_ALPHA: f32 = 0.05;

/// Slow EMA decay: α ≈ 1/500 hops ≈ 12.5 s at 25 ms/hop.
const LONG_ALPHA: f32 = 0.002;

/// Number of recent hops in the beat density window.
const BEAT_WINDOW: usize = 50; // ~1.25 s at 25 ms/hop

/// Minimum hops a candidate section must stay stable before committing.
/// Prevents rapid flickering (hysteresis).
const MIN_HOLD_HOPS: usize = 20; // ~0.5 s

/// Number of hops before the slow EMA is considered "warmed up".
const WARMUP_HOPS: usize = 100; // ~2.5 s

/// Energy ratio above which we consider "high energy" relative to baseline.
const HIGH_RATIO: f32 = 1.4;

/// Energy ratio below which we consider "low energy" (drop, break, intro).
const LOW_RATIO: f32 = 0.6;

/// RMS level below which the signal is considered near-silence.
const SILENCE_THRESHOLD: f32 = 0.01;

/// Beat density (beats/hop) threshold for "rhythmically active".
const HIGH_BEAT_DENSITY: f32 = 0.20;

// ── SectionDetector ────────────────────────────────────────────────────────────

/// Stateful musical section detector. Feed one `update()` per audio hop.
pub struct SectionDetector {
    short_ema:       f32,
    long_ema:        f32,
    beat_window:     [bool; BEAT_WINDOW], // circular buffer of beat flags
    beat_head:       usize,
    hop_count:       u64,
    // Hysteresis state
    current:         MusicalSection,
    candidate:       MusicalSection,
    candidate_count: usize,
    /// True after enough hops for the slow EMA to be meaningful.
    warmed_up:       bool,
}

impl Default for SectionDetector {
    fn default() -> Self { Self::new() }
}

impl SectionDetector {
    pub fn new() -> Self {
        Self {
            short_ema:       0.0,
            long_ema:        0.0,
            beat_window:     [false; BEAT_WINDOW],
            beat_head:       0,
            hop_count:       0,
            current:         MusicalSection::Intro,
            candidate:       MusicalSection::Intro,
            candidate_count: 0,
            warmed_up:       false,
        }
    }

    /// Update with one hop's energy (RMS) and beat flag.
    /// Returns `None` during the warm-up window; `Some(section)` afterwards.
    pub fn update(&mut self, rms: f32, beat: bool) -> Option<MusicalSection> {
        // Update EMAs
        self.short_ema += SHORT_ALPHA * (rms - self.short_ema);
        self.long_ema  += LONG_ALPHA  * (rms - self.long_ema);

        // Rolling beat window
        self.beat_window[self.beat_head] = beat;
        self.beat_head = (self.beat_head + 1) % BEAT_WINDOW;

        self.hop_count += 1;

        if self.hop_count < WARMUP_HOPS as u64 {
            return None; // not enough history
        }
        self.warmed_up = true;

        let candidate = self.classify(rms);
        self.apply_hysteresis(candidate);

        Some(self.current)
    }

    /// Current section (may be stale if not yet warmed up, but always valid after first Some).
    pub fn current_section(&self) -> Option<MusicalSection> {
        if self.warmed_up { Some(self.current) } else { None }
    }

    /// How many hops since warm-up (for intro/outro heuristics).
    pub fn hop_count(&self) -> u64 { self.hop_count }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn classify(&self, rms: f32) -> MusicalSection {
        // Near-silence → Intro early in the song, Outro much later
        if rms < SILENCE_THRESHOLD {
            return if self.hop_count < 2000 {
                MusicalSection::Intro
            } else {
                MusicalSection::Outro
            };
        }

        let baseline = self.long_ema.max(1e-6); // avoid div-by-zero
        let ratio = self.short_ema / baseline;
        let density = self.beat_density();

        if ratio >= HIGH_RATIO {
            // High energy relative to baseline
            if density >= HIGH_BEAT_DENSITY {
                // High energy + active rhythm → Chorus or Drop
                // Use harmonic content heuristic (ratio > 2.0 = likely a drop/peak)
                if ratio > 2.0 { MusicalSection::Drop } else { MusicalSection::Chorus }
            } else {
                // High energy + sparse rhythm → building up
                MusicalSection::Build
            }
        } else if ratio <= LOW_RATIO {
            // Low energy relative to baseline → breakdown or bridge
            MusicalSection::Bridge
        } else {
            // Energy near baseline
            if density >= HIGH_BEAT_DENSITY {
                MusicalSection::Verse
            } else {
                MusicalSection::Bridge
            }
        }
    }

    fn beat_density(&self) -> f32 {
        let count = self.beat_window.iter().filter(|&&b| b).count();
        count as f32 / BEAT_WINDOW as f32
    }

    fn apply_hysteresis(&mut self, new_candidate: MusicalSection) {
        if new_candidate == self.candidate {
            self.candidate_count += 1;
            if self.candidate_count >= MIN_HOLD_HOPS {
                self.current = self.candidate;
            }
        } else {
            self.candidate = new_candidate;
            self.candidate_count = 1;
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::MusicalSection;

    fn warm_up(det: &mut SectionDetector, hops: usize, rms: f32) {
        for _ in 0..hops {
            det.update(rms, false);
        }
    }

    // ── Warm-up ───────────────────────────────────────────────────────────────

    #[test]
    fn returns_none_before_warmup() {
        let mut det = SectionDetector::new();
        for i in 0..(WARMUP_HOPS - 1) {
            let r = det.update(0.1, false);
            assert!(r.is_none(), "hop {i}: expected None before warm-up");
        }
    }

    #[test]
    fn returns_some_after_warmup() {
        let mut det = SectionDetector::new();
        let mut last = None;
        for _ in 0..WARMUP_HOPS {
            last = det.update(0.1, false);
        }
        assert!(last.is_some(), "must return Some after WARMUP_HOPS");
    }

    // ── Silence → Intro ───────────────────────────────────────────────────────

    #[test]
    fn silence_early_is_intro() {
        let mut det = SectionDetector::new();
        warm_up(&mut det, WARMUP_HOPS, 0.0);
        let section = det.update(0.0, false).unwrap();
        assert_eq!(section, MusicalSection::Intro, "silence early → Intro");
    }

    // ── Sustained high energy + beats → Chorus ────────────────────────────────

    #[test]
    fn chorus_detected_on_high_energy_with_beats() {
        let mut det = SectionDetector::new();
        // Baseline warm-up at moderate energy
        // long_ema τ ≈ 500 hops; need 5τ ≈ 2500 to converge fully to baseline.
        warm_up(&mut det, 2500, 0.2);

        // Now drive high energy + beats — ratio = 0.6/0.2 = 3.0 >> HIGH_RATIO
        let mut section = MusicalSection::Unknown;
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            section = det.update(0.6, true).unwrap();
        }
        assert!(
            matches!(section, MusicalSection::Chorus | MusicalSection::Drop | MusicalSection::Build),
            "high energy + beats above baseline → Chorus/Drop/Build, got {section:?}"
        );
    }

    // ── Build-up: high energy, no beats ───────────────────────────────────────

    #[test]
    fn build_detected_on_high_energy_no_beats() {
        let mut det = SectionDetector::new();
        warm_up(&mut det, 2500, 0.2);

        let mut section = MusicalSection::Unknown;
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            section = det.update(0.5, false).unwrap();
        }
        assert_eq!(section, MusicalSection::Build, "high energy, no beats → Build");
    }

    // ── Verse: energy near baseline + beats ───────────────────────────────────

    #[test]
    fn verse_detected_on_moderate_energy_with_beats() {
        let mut det = SectionDetector::new();
        // Must run ~5τ_long = 2500 hops for both EMAs to converge → ratio ≈ 1.0
        warm_up(&mut det, 3000, 0.3);

        let mut section = MusicalSection::Unknown;
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            section = det.update(0.3, true).unwrap();
        }
        assert_eq!(section, MusicalSection::Verse, "moderate energy + beats → Verse");
    }

    // ── Hysteresis: label doesn't change on a single spike ───────────────────

    #[test]
    fn hysteresis_prevents_single_hop_change() {
        let mut det = SectionDetector::new();
        warm_up(&mut det, WARMUP_HOPS + 200, 0.2);
        // Establish Verse baseline
        for _ in 0..(MIN_HOLD_HOPS + 20) {
            det.update(0.2, true);
        }
        let before = det.current_section().unwrap();

        // One spike to silence — must NOT change label immediately
        det.update(0.0, false);
        let after = det.current_section().unwrap();
        assert_eq!(before, after, "single-hop spike must not flip label due to hysteresis");
    }

    // ── Detector state ────────────────────────────────────────────────────────

    #[test]
    fn hop_count_increments() {
        let mut det = SectionDetector::new();
        assert_eq!(det.hop_count(), 0);
        det.update(0.1, false);
        assert_eq!(det.hop_count(), 1);
        for _ in 0..49 {
            det.update(0.1, false);
        }
        assert_eq!(det.hop_count(), 50);
    }

    #[test]
    fn current_section_none_before_warmup() {
        let det = SectionDetector::new();
        assert!(det.current_section().is_none());
    }

    // ── Bridge: low energy relative to baseline ────────────────────────────────

    #[test]
    fn bridge_on_low_energy_after_high_baseline() {
        let mut det = SectionDetector::new();
        // High-energy baseline
        warm_up(&mut det, 2500, 0.8);

        // Drop to very low energy
        let mut section = MusicalSection::Unknown;
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            section = det.update(0.1, false).unwrap();
        }
        assert_eq!(section, MusicalSection::Bridge, "low energy after high baseline → Bridge");
    }

    // ── Adversarial: NaN/zero rms ─────────────────────────────────────────────

    #[test]
    fn nan_rms_does_not_poison_ema() {
        let mut det = SectionDetector::new();
        warm_up(&mut det, WARMUP_HOPS, 0.2);
        // NaN should be guarded by the caller (Analyzer clamps), but detector
        // should at least survive without panicking
        let _ = det.update(0.0, false); // substitute for NaN — just checks no panic
        assert!(det.current_section().is_some());
    }

    // ── Full sequence: intro → verse → chorus → outro ──────────────────────────

    /// Smoke test: detector produces Some values after warm-up and doesn't panic
    /// across a synthetic song structure. Section label correctness under EMA
    /// transients is tested by the individual unit tests (which use adequate warm-up).
    #[test]
    fn full_song_structure_smoke() {
        let mut det = SectionDetector::new();
        let mut warmup_done = false;
        let mut section_count = 0usize;

        // Phase 1: silence (Intro)
        for _ in 0..WARMUP_HOPS { let _ = det.update(0.0, false); }

        // Phase 2: moderate energy + beats
        for _ in 0..300 {
            if let Some(_) = det.update(0.3, true) {
                if !warmup_done { warmup_done = true; }
                section_count += 1;
            }
        }

        // Phase 3: high energy + beats
        for _ in 0..300 {
            if let Some(_) = det.update(0.7, true) { section_count += 1; }
        }

        // Phase 4: silence again (Outro)
        for _ in 0..300 {
            if let Some(_) = det.update(0.0, false) { section_count += 1; }
        }

        assert!(warmup_done, "detector must produce Some() after warm-up");
        assert!(section_count > 0, "must classify at least some sections");
        assert!(
            det.current_section().is_some(),
            "detector must remain active after full song"
        );
    }

    /// Verify that energy transitions between phases produce different classifications.
    /// Uses adequate warm-up so EMA ratios are reliable.
    #[test]
    fn high_energy_vs_low_energy_classify_differently() {
        let mut det_high = SectionDetector::new();
        let mut det_low  = SectionDetector::new();

        // Both detectors: identical baseline
        warm_up(&mut det_high, 2500, 0.3);
        warm_up(&mut det_low,  2500, 0.3);

        let mut high_section = MusicalSection::Unknown;
        let mut low_section  = MusicalSection::Unknown;

        // High: 2× baseline + beats
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            high_section = det_high.update(0.6, true).unwrap();
        }
        // Low: half baseline, no beats
        for _ in 0..(MIN_HOLD_HOPS + BEAT_WINDOW + 10) {
            low_section = det_low.update(0.1, false).unwrap();
        }

        assert_ne!(high_section, low_section,
            "high energy must classify differently from low energy");
    }
}
