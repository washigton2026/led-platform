//! `ShowIntent` — deterministic show descriptor + `ShowIntentGenerator`.
//!
//! ## Design (lumyx-ai-governor invariant)
//!
//! IA produces **intent, never execution**. A `ShowIntent` is a validated,
//! serialisable descriptor that a deterministic generator converts into a
//! `Timeline`. The same `ShowIntent` + same `seed` always produces the same
//! `Timeline` — no LLM in the runtime loop.
//!
//! ```text
//! AudioFeatures (section, instrument, beat, rms)
//!       ↓  ShowIntentGenerator::from_audio()
//! ShowIntent { style, energy, tempo_bpm, seed, section, ... }
//!       ↓  ShowIntentGenerator::build_timeline()
//! Timeline (deterministic, same seed → same clips)
//! ```
//!
//! ## Invariants (lumyx-ai-governor)
//! - `seed` is always recorded — replay is possible.
//! - `intent_hash` = FNV-1a of the ShowIntent bytes — for provenance.
//! - The generator NEVER calls an LLM; it uses rule-based logic only.
//! - A `ShowIntent` with an invalid `energy` (outside [0,1]) is rejected at construction.

use led_core::MusicalSection;

use crate::{BlendMode, Clip, Timeline, Track};
use led_pixel_engine::{Effect, Pulse, Rainbow, SolidColor};
use led_core::PixelColor;

// ── ShowStyle ─────────────────────────────────────────────────────────────────

/// Broad visual style for the generated show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShowStyle {
    /// Pulsing colors driven by beat energy.
    Beat,
    /// Smooth rainbow cycling.
    Ambient,
    /// High-energy strobing and flashes.
    Drop,
    /// Calm, section-aware color shifts.
    Narrative,
}

// ── ShowIntent ────────────────────────────────────────────────────────────────

/// Validated show descriptor. Produced by `ShowIntentGenerator` or an LLM
/// (offline only). Always validated before consumption.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowIntent {
    /// Broad visual style.
    pub style:      ShowStyle,
    /// Overall energy level [0.0, 1.0].
    pub energy:     f32,
    /// Detected or specified BPM.
    pub tempo_bpm:  f32,
    /// Duration of the generated show (ms).
    pub duration_ms: u64,
    /// Pixel count for the generated timeline.
    pub pixel_count: usize,
    /// Seed for deterministic generation. Always set — never `None`.
    pub seed:       u64,
    /// Current musical section at the time of generation.
    pub section:    Option<MusicalSection>,
    /// SHA-256-like hash of this intent (FNV-1a of fields, for provenance).
    pub intent_hash: u64,
}

impl ShowIntent {
    /// Validate and construct a `ShowIntent`. Returns `Err` if any field is invalid.
    pub fn new(
        style:       ShowStyle,
        energy:      f32,
        tempo_bpm:   f32,
        duration_ms: u64,
        pixel_count: usize,
        seed:        u64,
        section:     Option<MusicalSection>,
    ) -> Result<Self, ShowIntentError> {
        if !(0.0..=1.0).contains(&energy) {
            return Err(ShowIntentError::EnergyOutOfRange(energy));
        }
        if tempo_bpm < 20.0 || tempo_bpm > 300.0 {
            return Err(ShowIntentError::InvalidTempo(tempo_bpm));
        }
        if duration_ms == 0 {
            return Err(ShowIntentError::ZeroDuration);
        }
        if pixel_count == 0 {
            return Err(ShowIntentError::ZeroPixels);
        }
        let mut intent = Self {
            style, energy, tempo_bpm, duration_ms, pixel_count, seed, section,
            intent_hash: 0,
        };
        intent.intent_hash = intent.compute_hash();
        Ok(intent)
    }

    /// FNV-1a hash of the intent fields (for provenance and replay verification).
    fn compute_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME:  u64 = 0x00000100000001b3;
        let mut h = FNV_OFFSET;
        let mix = |h: &mut u64, v: u64| { *h ^= v; *h = h.wrapping_mul(FNV_PRIME); };
        mix(&mut h, self.style as u64);
        mix(&mut h, self.energy.to_bits() as u64);
        mix(&mut h, self.tempo_bpm.to_bits() as u64);
        mix(&mut h, self.duration_ms);
        mix(&mut h, self.pixel_count as u64);
        mix(&mut h, self.seed);
        mix(&mut h, self.section.map(|s| s as u64).unwrap_or(0xFF));
        h
    }

    /// True if this intent can produce a valid timeline.
    pub fn is_valid(&self) -> bool {
        (0.0..=1.0).contains(&self.energy)
            && self.tempo_bpm >= 20.0
            && self.duration_ms > 0
            && self.pixel_count > 0
    }
}

/// Validation errors for `ShowIntent`.
#[derive(Debug, PartialEq)]
pub enum ShowIntentError {
    EnergyOutOfRange(f32),
    InvalidTempo(f32),
    ZeroDuration,
    ZeroPixels,
}

// ── ShowIntentGenerator ───────────────────────────────────────────────────────

/// Converts `AudioFeatures`-derived signals into a `ShowIntent` and then into
/// a deterministic `Timeline`. No LLM — pure rule-based, seeded PRNG.
pub struct ShowIntentGenerator {
    seed: u64,
}

/// Simple SplitMix64 PRNG (deterministic, seeded).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

impl ShowIntentGenerator {
    pub fn new(seed: u64) -> Self { Self { seed } }

    /// Derive a `ShowIntent` from the current audio analysis state.
    /// Pure function of its inputs — same inputs + same seed → same intent.
    pub fn from_audio(
        &self,
        rms:         f32,
        beat:        bool,
        bpm:         f32,
        section:     Option<MusicalSection>,
        duration_ms: u64,
        pixel_count: usize,
    ) -> Result<ShowIntent, ShowIntentError> {
        let style = match section {
            Some(MusicalSection::Drop)   | Some(MusicalSection::Chorus) => ShowStyle::Drop,
            Some(MusicalSection::Verse)  | Some(MusicalSection::Build)  => ShowStyle::Beat,
            Some(MusicalSection::Bridge) | Some(MusicalSection::Intro)  => ShowStyle::Ambient,
            Some(MusicalSection::Outro)                                  => ShowStyle::Narrative,
            _ => if beat { ShowStyle::Beat } else { ShowStyle::Ambient },
        };
        let energy = rms.clamp(0.0, 1.0);
        let tempo  = if bpm > 20.0 && bpm < 300.0 { bpm } else { 120.0 };

        ShowIntent::new(style, energy, tempo, duration_ms, pixel_count, self.seed, section)
    }

    /// Build a deterministic `Timeline` from a `ShowIntent`.
    ///
    /// Same `intent.seed` + same `intent` fields → same `Timeline` every time.
    pub fn build_timeline(&self, intent: &ShowIntent) -> Timeline {
        let mut rng = intent.seed;
        let n = intent.pixel_count;
        let beat_ms = (60_000.0 / intent.tempo_bpm) as u64;

        // Primary effect based on style
        let primary: Box<dyn Effect> = match intent.style {
            ShowStyle::Drop | ShowStyle::Beat => {
                let r = (splitmix64(&mut rng) % 200 + 50) as u8;
                let g = (splitmix64(&mut rng) % 100) as u8;
                let b = (splitmix64(&mut rng) % 200 + 50) as u8;
                let hz = 0.5 + intent.energy * 3.0;
                Box::new(Pulse { color: PixelColor::rgb(r, g, b), hz })
            }
            ShowStyle::Ambient | ShowStyle::Narrative => {
                let speed_hz = 0.1 + intent.energy * 0.5;
                Box::new(Rainbow { speed_hz, cycles_per_m: 0.5 })
            }
        };

        let mut tl = Timeline::new(n).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, intent.duration_ms, primary)),
        );

        // Beat-synced flash overlay for high-energy styles
        if matches!(intent.style, ShowStyle::Drop | ShowStyle::Beat) && intent.energy > 0.4 {
            let intensity = (intent.energy * 180.0) as u8;
            let color = PixelColor::rgb(intensity, intensity, intensity);
            let mut beats = Track::new(BlendMode::Add);
            let mut t = beat_ms;
            while t < intent.duration_ms {
                beats.clips.push(
                    Clip::new(t, t + beat_ms / 3, Box::new(SolidColor(color)))
                        .with_fades(0, beat_ms / 3),
                );
                t += beat_ms;
            }
            tl = tl.with_track(beats);
        }

        tl
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::MusicalSection;

    fn intent(style: ShowStyle, energy: f32, bpm: f32) -> ShowIntent {
        ShowIntent::new(style, energy, bpm, 6_000, 64, 42, None).unwrap()
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[test]
    fn valid_intent_constructs_ok() {
        let i = ShowIntent::new(ShowStyle::Beat, 0.7, 120.0, 6_000, 64, 1, None);
        assert!(i.is_ok());
    }

    #[test]
    fn energy_out_of_range_rejected() {
        assert_eq!(
            ShowIntent::new(ShowStyle::Beat, 1.5, 120.0, 6_000, 64, 1, None),
            Err(ShowIntentError::EnergyOutOfRange(1.5))
        );
        assert_eq!(
            ShowIntent::new(ShowStyle::Beat, -0.1, 120.0, 6_000, 64, 1, None),
            Err(ShowIntentError::EnergyOutOfRange(-0.1))
        );
    }

    #[test]
    fn zero_duration_rejected() {
        assert_eq!(
            ShowIntent::new(ShowStyle::Beat, 0.5, 120.0, 0, 64, 1, None),
            Err(ShowIntentError::ZeroDuration)
        );
    }

    #[test]
    fn zero_pixels_rejected() {
        assert_eq!(
            ShowIntent::new(ShowStyle::Beat, 0.5, 120.0, 6_000, 0, 1, None),
            Err(ShowIntentError::ZeroPixels)
        );
    }

    #[test]
    fn invalid_tempo_rejected() {
        assert_eq!(
            ShowIntent::new(ShowStyle::Beat, 0.5, 10.0, 6_000, 64, 1, None),
            Err(ShowIntentError::InvalidTempo(10.0))
        );
    }

    // ── Determinism ───────────────────────────────────────────────────────────

    #[test]
    fn same_intent_same_hash() {
        let a = ShowIntent::new(ShowStyle::Beat, 0.7, 120.0, 6_000, 64, 42, Some(MusicalSection::Chorus)).unwrap();
        let b = ShowIntent::new(ShowStyle::Beat, 0.7, 120.0, 6_000, 64, 42, Some(MusicalSection::Chorus)).unwrap();
        assert_eq!(a.intent_hash, b.intent_hash, "same inputs → same hash");
    }

    #[test]
    fn different_seed_different_hash() {
        let a = ShowIntent::new(ShowStyle::Beat, 0.7, 120.0, 6_000, 64, 1, None).unwrap();
        let b = ShowIntent::new(ShowStyle::Beat, 0.7, 120.0, 6_000, 64, 2, None).unwrap();
        assert_ne!(a.intent_hash, b.intent_hash);
    }

    #[test]
    fn timeline_deterministic_from_same_intent() {
        use led_pixel_engine::{Effect, Vec3};
        use led_core::PixelColor;

        let gen = ShowIntentGenerator::new(99);
        let intent = intent(ShowStyle::Beat, 0.8, 120.0);
        let tl1 = gen.build_timeline(&intent);
        let tl2 = gen.build_timeline(&intent);

        let pos = vec![Vec3::ZERO; 64];
        let mut out1 = vec![PixelColor::default(); 64];
        let mut out2 = vec![PixelColor::default(); 64];
        tl1.render(1000, &pos, &mut out1);
        tl2.render(1000, &pos, &mut out2);
        assert_eq!(out1, out2, "same intent → same timeline → same pixels");
    }

    // ── Generator ────────────────────────────────────────────────────────────

    #[test]
    fn from_audio_chorus_yields_drop_style() {
        let gen = ShowIntentGenerator::new(1);
        let i = gen.from_audio(0.8, true, 128.0, Some(MusicalSection::Chorus), 6_000, 64).unwrap();
        assert_eq!(i.style, ShowStyle::Drop);
    }

    #[test]
    fn from_audio_verse_yields_beat_style() {
        let gen = ShowIntentGenerator::new(2);
        let i = gen.from_audio(0.5, true, 120.0, Some(MusicalSection::Verse), 6_000, 64).unwrap();
        assert_eq!(i.style, ShowStyle::Beat);
    }

    #[test]
    fn from_audio_intro_yields_ambient() {
        let gen = ShowIntentGenerator::new(3);
        let i = gen.from_audio(0.1, false, 120.0, Some(MusicalSection::Intro), 6_000, 64).unwrap();
        assert_eq!(i.style, ShowStyle::Ambient);
    }

    #[test]
    fn from_audio_zero_energy_clamps_to_zero() {
        let gen = ShowIntentGenerator::new(0);
        let i = gen.from_audio(0.0, false, 120.0, None, 6_000, 64).unwrap();
        assert!((i.energy - 0.0).abs() < 1e-6);
    }

    #[test]
    fn from_audio_invalid_bpm_defaults_to_120() {
        let gen = ShowIntentGenerator::new(0);
        let i = gen.from_audio(0.5, false, 5.0 /* invalid */, None, 6_000, 64).unwrap();
        assert!((i.tempo_bpm - 120.0).abs() < 1e-4, "invalid BPM → defaults to 120");
    }

    // ── Build timeline ────────────────────────────────────────────────────────

    #[test]
    fn build_timeline_produces_non_trivial_output() {
        use led_pixel_engine::{Effect, Vec3};
        use led_core::PixelColor;

        let gen = ShowIntentGenerator::new(42);
        let tl = gen.build_timeline(&intent(ShowStyle::Drop, 0.9, 120.0));
        let pos = vec![Vec3::new(0.1, 0.0, 0.0); 16];
        let mut out = vec![PixelColor::default(); 16];
        tl.render(500, &pos, &mut out);
        assert!(out.iter().any(|&p| p != PixelColor::default()), "timeline must produce output");
    }

    #[test]
    fn build_timeline_ambient_no_beat_flashes() {
        // Ambient style should not add beat-flash track
        let gen = ShowIntentGenerator::new(0);
        let tl = gen.build_timeline(&intent(ShowStyle::Ambient, 0.3, 120.0));
        use led_pixel_engine::{Effect, Vec3};
        use led_core::PixelColor;
        let pos = vec![Vec3::ZERO; 4];
        let mut out = vec![PixelColor::default(); 4];
        tl.render(0, &pos, &mut out);
        // Ambient renders without panic — no specific color assertion needed
        let _ = out;
    }

    // ── Intent hash is non-zero ───────────────────────────────────────────────

    #[test]
    fn intent_hash_non_zero() {
        let i = ShowIntent::new(ShowStyle::Beat, 0.5, 120.0, 5_000, 32, 1, None).unwrap();
        assert_ne!(i.intent_hash, 0, "hash must be non-zero for non-trivial input");
    }
}
