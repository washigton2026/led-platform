//! `adapt` — the v1→v0 adapter function.
//!
//! ## Mapping (v1.1 — updated Cycle 7/8)
//!
//! | v1 field (audio-core)      | v0 field (led-core)   | Notes                        |
//! |----------------------------|-----------------------|------------------------------|
//! | `timestamp_ms`             | `timestamp_ms`        | direct                       |
//! | `sample_rate`              | `sample_rate`         | direct                       |
//! | `rms`                      | `rms`                 | direct                       |
//! | `beat`                     | `beat`                | direct (gated by harmonic)   |
//! | `bass_energy`              | `bass`                | rename only                  |
//! | `mid_energy`               | `mid`                 | rename only                  |
//! | `high_energy`              | `high`                | rename only                  |
//! | `spectrum[0..SPECTRUM_LEN]`| `spectrum` (Vec)      | copy fixed array → Vec slice |
//! | `peak`, `onset`, `bpm`     | *(dropped)*           | v0 has no field for these    |
//! | `harmonic_ratio`           | *(available via fn)*  | use `harmonic_ratio(v1)`     |
//! | `musical_section`          | `musical_section`     | mapped via `map_section()`   |
//!
//! ## harmonic_ratio availability
//!
//! `led_core::AudioFeatures` (v0) does not carry `harmonic_ratio` — it predates v1.1.
//! Consumers needing harmonic information should:
//! 1. Call [`harmonic_ratio`] to extract it from the v1 features before adapting, OR
//! 2. Depend directly on `audio_core::AudioFeatures` if they need the full v1.1 contract.
//!
//! ## Allocation contract
//!
//! The returned `led_core::AudioFeatures` owns a `Vec<f32>` for `spectrum`.
//! In a hot loop, callers should reuse a pooled `led_core::AudioFeatures` and call
//! `adapt_into` (which writes into a pre-allocated Vec) rather than `adapt` (which
//! allocates a new Vec every call).
//! For the bridge thread this is fine: the `AudioShare::publish` call copies out of the
//! Vec immediately, so no heap pressure accumulates.

use audio_core::contracts::{AudioFeatures as V1, MusicalSection as V1Section};
use audio_core::instrument::InstrumentClass as V1Inst;
use led_core::{AudioFeatures as V0, InstrumentClass as V0Inst, MusicalSection as V0Section};

/// Map `audio_core::InstrumentClass` (v1) to `led_core::InstrumentClass` (v0).
#[inline]
pub fn map_instrument(c: V1Inst) -> V0Inst {
    match c {
        V1Inst::Kick    => V0Inst::Kick,
        V1Inst::Snare   => V0Inst::Snare,
        V1Inst::HiHat   => V0Inst::HiHat,
        V1Inst::Bass    => V0Inst::Bass,
        V1Inst::Melody  => V0Inst::Melody,
        V1Inst::Chord   => V0Inst::Chord,
        V1Inst::Noise   => V0Inst::Noise,
        V1Inst::Silence => V0Inst::Silence,
        V1Inst::Unknown => V0Inst::Unknown,
    }
}

/// Map `audio_core::MusicalSection` (v1) to `led_core::MusicalSection` (v0).
/// Both enums are structurally identical — this is a zero-cost name-space bridge.
#[inline]
pub fn map_section(s: V1Section) -> V0Section {
    match s {
        V1Section::Intro   => V0Section::Intro,
        V1Section::Verse   => V0Section::Verse,
        V1Section::Chorus  => V0Section::Chorus,
        V1Section::Bridge  => V0Section::Bridge,
        V1Section::Drop    => V0Section::Drop,
        V1Section::Build   => V0Section::Build,
        V1Section::Outro   => V0Section::Outro,
        V1Section::Unknown => V0Section::Unknown,
    }
}

/// Convert `audio_core::AudioFeatures` (v1, `Copy`) to `led_core::AudioFeatures` (v0).
///
/// Allocates one `Vec<f32>` for the spectrum on each call. Use [`adapt_into`] in tight
/// loops where you can reuse the Vec.
#[inline]
pub fn adapt(v1: &V1) -> V0 {
    V0 {
        sample_rate:     v1.sample_rate,
        timestamp_ms:    v1.timestamp_ms,
        rms:             v1.rms,
        beat:            v1.beat,
        bass:            v1.bass_energy,
        mid:             v1.mid_energy,
        high:            v1.high_energy,
        spectrum:        v1.spectrum.to_vec(),
        musical_section:  v1.musical_section.map(map_section),
        instrument_class: v1.instrument_class.map(map_instrument),
    }
}

/// Extract `harmonic_ratio` from a v1 `AudioFeatures` without full adaptation.
///
/// Use this when downstream code needs only the harmonic content signal and not
/// the full v0 `AudioFeatures`. Zero allocation, zero copy — just reads the field.
#[inline]
pub fn harmonic_ratio(v1: &V1) -> f32 {
    v1.harmonic_ratio
}

/// True if `v1` is strongly tonal (sustained instrument, not transient).
/// Equivalent to `harmonic_ratio(v1) >= audio_core::harmonics::TONAL_THRESHOLD`.
#[inline]
pub fn is_tonal(v1: &V1) -> bool {
    v1.harmonic_ratio >= audio_core::harmonics::TONAL_THRESHOLD
}

/// Zero-alloc variant: write the adapted v0 fields into a pre-allocated `V0`.
///
/// The `out.spectrum` Vec is resized only if its length differs from `SPECTRUM_LEN`.
/// In a steady-state bridge loop (sample rate fixed) this resize never fires after warmup.
#[inline]
pub fn adapt_into(v1: &V1, out: &mut V0) {
    out.sample_rate  = v1.sample_rate;
    out.timestamp_ms = v1.timestamp_ms;
    out.rms          = v1.rms;
    out.beat         = v1.beat;
    out.bass         = v1.bass_energy;
    out.mid          = v1.mid_energy;
    out.high         = v1.high_energy;
    if out.spectrum.len() != v1.spectrum.len() {
        out.spectrum.resize(v1.spectrum.len(), 0.0); // only on first call or rate change
    }
    out.spectrum.copy_from_slice(&v1.spectrum);
    out.musical_section  = v1.musical_section.map(map_section);
    out.instrument_class = v1.instrument_class.map(map_instrument);
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::contracts::{AudioFeatures as V1, SPECTRUM_LEN};

    fn make_v1(beat: bool, ts: u64, bass: f32) -> V1 {
        let mut spec = [0.0f32; SPECTRUM_LEN];
        for (i, s) in spec.iter_mut().enumerate() {
            *s = i as f32 / SPECTRUM_LEN as f32;
        }
        V1 {
            timestamp_ms:  ts,
            sample_rate:   48_000,
            rms:           0.5,
            peak:          0.8,
            beat,
            onset:         beat,
            bpm:           120.0,
            bass_energy:   bass,
            mid_energy:    0.3,
            high_energy:   0.1,
            spectral_flux: 0.05,
            spectrum:      spec,
            ..V1::default()
        }
    }

    // ── CONTRACT: every v0 field matches the expected v1 source ──────────
    #[test]
    fn adapt_maps_all_fields_correctly() {
        let v1 = make_v1(true, 1234, 0.75);
        let v0 = adapt(&v1);
        assert_eq!(v0.sample_rate,  48_000);
        assert_eq!(v0.timestamp_ms, 1234);
        assert!((v0.rms - 0.5).abs() < 1e-6);
        assert!(v0.beat);
        assert!((v0.bass - 0.75).abs() < 1e-6,  "bass_energy → bass");
        assert!((v0.mid  - 0.3).abs()  < 1e-6,  "mid_energy  → mid");
        assert!((v0.high - 0.1).abs()  < 1e-6,  "high_energy → high");
        assert_eq!(v0.spectrum.len(), SPECTRUM_LEN, "spectrum len preserved");
        assert!((v0.spectrum[0]   - 0.0).abs() < 1e-6);
        assert!((v0.spectrum[511] - 511.0 / SPECTRUM_LEN as f32).abs() < 1e-6);
    }

    // ── CONTRACT: v1 fields absent in v0 are silently dropped ─────────────
    #[test]
    fn adapt_drops_v1_only_fields() {
        let v1 = make_v1(false, 0, 0.0);
        let v0 = adapt(&v1);
        // v0 has no peak, onset, bpm, spectral_* — just verify compile-time (no field access)
        // Only check that adapt doesn't return garbage in the fields it DOES carry
        assert_eq!(v0.sample_rate, 48_000);
    }

    // ── ZERO-ALLOC: adapt_into reuses the Vec after the first call ────────
    #[test]
    fn adapt_into_no_resize_after_warmup() {
        let v1 = make_v1(true, 0, 0.5);
        let mut v0 = V0 {
            sample_rate: 0, timestamp_ms: 0, rms: 0.0,
            beat: false, bass: 0.0, mid: 0.0, high: 0.0,
            spectrum: vec![0.0; SPECTRUM_LEN], // pre-sized
            musical_section: None, instrument_class: None,
        };
        // Warm-up
        adapt_into(&v1, &mut v0);
        let ptr_before = v0.spectrum.as_ptr();
        // Second call — must NOT reallocate (ptr stays same)
        adapt_into(&v1, &mut v0);
        let ptr_after = v0.spectrum.as_ptr();
        assert_eq!(ptr_before, ptr_after, "adapt_into must not reallocate on steady state");
    }

    // ── STRESS: 1M adapt calls — no panic ────────────────────────────────
    #[test]
    fn adapt_1m_iterations_no_panic() {
        let v1 = make_v1(true, 0, 0.5);
        let mut v0 = adapt(&v1);
        for i in 0..1_000_000u64 {
            let mut v = v1;
            v.timestamp_ms = i;
            adapt_into(&v, &mut v0);
        }
        assert_eq!(v0.timestamp_ms, 999_999);
    }

    // ── FUZZ: adapt with all-NaN v1 — no panic ───────────────────────────
    #[test]
    fn adapt_nan_values_no_panic() {
        let mut spec = [f32::NAN; SPECTRUM_LEN];
        spec[0] = f32::NAN;
        let v1 = V1 {
            rms: f32::NAN,
            bass_energy: f32::INFINITY,
            mid_energy: f32::NEG_INFINITY,
            spectrum: spec,
            ..V1::default()
        };
        let v0 = adapt(&v1);
        assert!(v0.rms.is_nan());
        assert!(v0.bass.is_infinite());
        // spectrum transferred as-is — correctness of downstream is that crate's problem
        assert!(v0.spectrum[0].is_nan());
    }

    // ── CONTRACT: beat=false maps correctly ───────────────────────────────
    #[test]
    fn adapt_beat_false_maps_false() {
        let v1 = make_v1(false, 99, 0.0);
        let v0 = adapt(&v1);
        assert!(!v0.beat, "beat=false must survive adaptation");
    }

    // ── CONTRACT: musical_section None maps to None ───────────────────────
    #[test]
    fn adapt_musical_section_none_maps_none() {
        let v1 = make_v1(false, 0, 0.0); // musical_section = None (default)
        let v0 = adapt(&v1);
        assert!(v0.musical_section.is_none(), "None musical_section must survive adaptation");
    }

    // ── CONTRACT: musical_section Some(...) maps to correct v0 variant ────
    #[test]
    fn adapt_musical_section_some_maps_correctly() {
        use audio_core::contracts::MusicalSection as V1Sec;
        use led_core::MusicalSection as V0Sec;

        let cases = [
            (V1Sec::Intro,   V0Sec::Intro),
            (V1Sec::Verse,   V0Sec::Verse),
            (V1Sec::Chorus,  V0Sec::Chorus),
            (V1Sec::Bridge,  V0Sec::Bridge),
            (V1Sec::Drop,    V0Sec::Drop),
            (V1Sec::Build,   V0Sec::Build),
            (V1Sec::Outro,   V0Sec::Outro),
            (V1Sec::Unknown, V0Sec::Unknown),
        ];
        for (v1_sec, expected_v0) in cases {
            let mut v1 = make_v1(false, 0, 0.0);
            v1.musical_section = Some(v1_sec);
            let v0 = adapt(&v1);
            assert_eq!(
                v0.musical_section,
                Some(expected_v0),
                "v1 {:?} must map to v0 {:?}", v1_sec, expected_v0
            );
        }
    }

    // ── CONTRACT: adapt_into also maps musical_section ────────────────────
    #[test]
    fn adapt_into_maps_musical_section() {
        use audio_core::contracts::MusicalSection as V1Sec;
        use led_core::MusicalSection as V0Sec;

        let mut v1 = make_v1(false, 0, 0.0);
        v1.musical_section = Some(V1Sec::Chorus);
        let mut v0 = V0 {
            spectrum: vec![0.0; SPECTRUM_LEN],
            musical_section: None, instrument_class: None,
            ..V0::default()
        };
        adapt_into(&v1, &mut v0);
        assert_eq!(v0.musical_section, Some(V0Sec::Chorus));

        // After clearing, None maps through
        v1.musical_section = None;
        adapt_into(&v1, &mut v0);
        assert!(v0.musical_section.is_none());
    }
}
