//! `SectionClip` — an [`Effect`] that switches sub-effects based on the current
//! [`MusicalSection`] label carried in [`led_core::AudioFeatures`].
//!
//! ## How it works
//!
//! ```text
//! AudioShare (render thread) → SectionClip.render()
//!     reads AudioScalars.musical_section (via a shared SectionReceiver)
//!             ↓
//!     matches section → selects sub-effect
//!             ↓
//!     delegates render() to the selected sub-effect
//! ```
//!
//! ## Design invariants
//! - `SectionClip` implements [`Effect`] — it plugs directly into any `Timeline` track.
//! - It is **pure**: given the same `time_ms` and the same `musical_section` it produces
//!   the same pixels. The section label comes from outside (AudioShare) and is applied
//!   at render time only.
//! - A `default_effect` handles `None` (pre-warm-up) and any section with no mapping.
//! - Sub-effects are stored in a `HashMap` by section variant — O(1) lookup per frame.
//! - The clip does NOT own a sequencer timeline — it composes at the Effect level,
//!   one layer below. This is intentional: it keeps the Timeline non-destructive.

use std::collections::HashMap;
use std::sync::Arc;

use led_core::{MusicalSection, PixelColor};
use led_pixel_engine::{AudioShare, Effect, Vec3};

// ── SectionClip ───────────────────────────────────────────────────────────────

/// An [`Effect`] that routes rendering to a sub-effect chosen by the current
/// musical section. Falls back to `default_effect` when section is `None`
/// (pre-warm-up) or has no mapping.
pub struct SectionClip {
    /// Effect used when section is `None` or has no explicit mapping.
    default_effect: Box<dyn Effect>,
    /// Per-section overrides.
    section_effects: HashMap<MusicalSection, Box<dyn Effect>>,
    /// Source of the live musical section label.
    audio: Arc<AudioShare>,
}

impl SectionClip {
    /// Create a `SectionClip` with the given default effect and audio share.
    pub fn new(default_effect: Box<dyn Effect>, audio: Arc<AudioShare>) -> Self {
        Self {
            default_effect,
            section_effects: HashMap::new(),
            audio,
        }
    }

    /// Register an effect for a specific section. Returns `self` for chaining.
    pub fn with_section(mut self, section: MusicalSection, effect: Box<dyn Effect>) -> Self {
        self.section_effects.insert(section, effect);
        self
    }
}

impl Effect for SectionClip {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let section = self.audio.scalars().musical_section;
        let fx: &dyn Effect = section
            .and_then(|s| self.section_effects.get(&s).map(|e| e.as_ref()))
            .unwrap_or(self.default_effect.as_ref());
        fx.render(time_ms, positions, out);
    }
}

// ── AudioScalars extension ────────────────────────────────────────────────────
// AudioShare::scalars() returns AudioScalars which doesn't carry musical_section
// (it's Copy + alloc-free). We add it via an extension on AudioShare.

/// A thin wrapper that lets SectionClip read the last published musical_section.
/// Stored separately from AudioScalars to avoid making AudioScalars non-Copy.
pub struct SectionReceiver(Arc<AudioShare>);

impl SectionReceiver {
    pub fn new(share: Arc<AudioShare>) -> Self { Self(share) }
    pub fn section(&self) -> Option<MusicalSection> {
        self.0.scalars().musical_section
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::{AudioFeatures, MusicalSection, PixelColor};
    use led_pixel_engine::{AudioShare, Effect, SolidColor, Vec3};
    use std::sync::Arc;

    fn pos(n: usize) -> Vec<Vec3> { vec![Vec3::ZERO; n] }

    fn make_share_with_section(section: Option<MusicalSection>) -> Arc<AudioShare> {
        let share = Arc::new(AudioShare::new());
        share.publish(&AudioFeatures {
            sample_rate:     44100,
            timestamp_ms:    1,
            rms:             0.5,
            beat:            false,
            bass:            0.3,
            mid:             0.2,
            high:            0.1,
            spectrum:        vec![0.0; 8],
            musical_section: section, instrument_class: None,
        });
        share
    }

    fn render_first(fx: &SectionClip, t: u64) -> PixelColor {
        let mut out = vec![PixelColor::default(); 4];
        fx.render(t, &pos(4), &mut out);
        out[0]
    }

    // ── Default effect when section is None ────────────────────────────────

    #[test]
    fn uses_default_when_section_is_none() {
        let share = make_share_with_section(None);
        let clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(100, 0, 0))),
            share,
        );
        let px = render_first(&clip, 0);
        assert_eq!(px, PixelColor::rgb(100, 0, 0), "None section → default effect");
    }

    // ── Mapped section uses correct sub-effect ─────────────────────────────

    #[test]
    fn uses_mapped_effect_for_chorus() {
        let share = make_share_with_section(Some(MusicalSection::Chorus));
        let clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(0, 0, 0))),    // default = black
            share,
        )
        .with_section(MusicalSection::Chorus, Box::new(SolidColor(PixelColor::rgb(255, 200, 0)))); // chorus = gold

        let px = render_first(&clip, 0);
        assert_eq!(px, PixelColor::rgb(255, 200, 0), "Chorus section → chorus effect");
    }

    // ── Unmapped section falls back to default ─────────────────────────────

    #[test]
    fn unmapped_section_falls_back_to_default() {
        let share = make_share_with_section(Some(MusicalSection::Bridge));
        let clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(50, 50, 50))), // default = grey
            share,
        )
        .with_section(MusicalSection::Chorus, Box::new(SolidColor(PixelColor::rgb(255, 0, 0))));
        // Bridge has no mapping → default
        let px = render_first(&clip, 0);
        assert_eq!(px, PixelColor::rgb(50, 50, 50), "Bridge (unmapped) → default");
    }

    // ── Section switch: Chorus → Verse → default ───────────────────────────

    #[test]
    fn section_switch_changes_output() {
        let share = Arc::new(AudioShare::new());

        let clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(10, 10, 10))),
            share.clone(),
        )
        .with_section(MusicalSection::Verse,  Box::new(SolidColor(PixelColor::rgb(0, 100, 0))))
        .with_section(MusicalSection::Chorus, Box::new(SolidColor(PixelColor::rgb(0, 0, 200))));

        let publish = |section: Option<MusicalSection>| {
            share.publish(&AudioFeatures {
                sample_rate: 44100, timestamp_ms: 1, rms: 0.5, beat: false,
                bass: 0.0, mid: 0.0, high: 0.0, spectrum: vec![0.0; 8],
                musical_section: section, instrument_class: None,
            });
        };

        publish(Some(MusicalSection::Verse));
        let px_verse = render_first(&clip, 0);
        assert_eq!(px_verse, PixelColor::rgb(0, 100, 0), "Verse section");

        publish(Some(MusicalSection::Chorus));
        let px_chorus = render_first(&clip, 0);
        assert_eq!(px_chorus, PixelColor::rgb(0, 0, 200), "Chorus section");

        publish(None);
        let px_none = render_first(&clip, 0);
        assert_eq!(px_none, PixelColor::rgb(10, 10, 10), "None → default");
    }

    // ── All 8 sections can be mapped ──────────────────────────────────────

    #[test]
    fn all_sections_can_be_mapped() {
        let share = Arc::new(AudioShare::new());
        let sections = [
            (MusicalSection::Intro,   PixelColor::rgb(10, 0, 0)),
            (MusicalSection::Verse,   PixelColor::rgb(0, 10, 0)),
            (MusicalSection::Chorus,  PixelColor::rgb(0, 0, 10)),
            (MusicalSection::Bridge,  PixelColor::rgb(10, 10, 0)),
            (MusicalSection::Drop,    PixelColor::rgb(0, 10, 10)),
            (MusicalSection::Build,   PixelColor::rgb(10, 0, 10)),
            (MusicalSection::Outro,   PixelColor::rgb(5, 5, 5)),
            (MusicalSection::Unknown, PixelColor::rgb(1, 1, 1)),
        ];

        let mut clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(0, 0, 0))),
            share.clone(),
        );
        for (sec, color) in &sections {
            clip = clip.with_section(*sec, Box::new(SolidColor(*color)));
        }

        for (sec, expected) in &sections {
            share.publish(&AudioFeatures {
                sample_rate: 44100, timestamp_ms: 1, rms: 0.0, beat: false,
                bass: 0.0, mid: 0.0, high: 0.0, spectrum: vec![0.0; 8],
                musical_section: Some(*sec), instrument_class: None,
            });
            let px = render_first(&clip, 0);
            assert_eq!(px, *expected, "{sec:?} must route to its effect");
        }
    }

    // ── Deterministic: same section + same time → same output ─────────────

    #[test]
    fn same_section_same_time_deterministic() {
        let share = make_share_with_section(Some(MusicalSection::Verse));
        let clip = SectionClip::new(
            Box::new(SolidColor(PixelColor::rgb(0, 0, 0))),
            share,
        )
        .with_section(MusicalSection::Verse, Box::new(SolidColor(PixelColor::rgb(77, 88, 99))));

        let a = render_first(&clip, 1000);
        let b = render_first(&clip, 1000);
        assert_eq!(a, b, "same time + same section → deterministic output");
    }

    // ── SectionReceiver convenience ────────────────────────────────────────

    #[test]
    fn section_receiver_returns_current_section() {
        let share = make_share_with_section(Some(MusicalSection::Build));
        let rx = SectionReceiver::new(share.clone());
        assert_eq!(rx.section(), Some(MusicalSection::Build));

        share.publish(&AudioFeatures {
            musical_section: None, instrument_class: None,
            sample_rate: 44100, timestamp_ms: 2, rms: 0.0, beat: false,
            bass: 0.0, mid: 0.0, high: 0.0, spectrum: vec![],
        });
        assert!(rx.section().is_none(), "receiver must reflect latest publish");
    }
}
