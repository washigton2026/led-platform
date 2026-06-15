//! `adapt` — the v1→v0 adapter function.
//!
//! ## Mapping
//!
//! | v1 field (audio-core)      | v0 field (led-core)   | Notes                        |
//! |----------------------------|-----------------------|------------------------------|
//! | `timestamp_ms`             | `timestamp_ms`        | direct                       |
//! | `sample_rate`              | `sample_rate`         | direct                       |
//! | `rms`                      | `rms`                 | direct                       |
//! | `beat`                     | `beat`                | direct                       |
//! | `bass_energy`              | `bass`                | rename only                  |
//! | `mid_energy`               | `mid`                 | rename only                  |
//! | `high_energy`              | `high`                | rename only                  |
//! | `spectrum[0..SPECTRUM_LEN]`| `spectrum` (Vec)      | copy fixed array → Vec slice |
//! | `peak`, `onset`, `bpm`, …  | *(dropped)*           | v0 has no field for these    |
//!
//! ## Allocation contract
//!
//! The returned `led_core::AudioFeatures` owns a `Vec<f32>` for `spectrum`.
//! In a hot loop, callers should reuse a pooled `led_core::AudioFeatures` and call
//! `adapt_into` (which writes into a pre-allocated Vec) rather than `adapt` (which
//! allocates a new Vec every call).
//! For the bridge thread this is fine: the `AudioShare::publish` call copies out of the
//! Vec immediately, so no heap pressure accumulates.

use audio_core::contracts::AudioFeatures as V1;
use led_core::AudioFeatures as V0;

/// Convert `audio_core::AudioFeatures` (v1, `Copy`) to `led_core::AudioFeatures` (v0).
///
/// Allocates one `Vec<f32>` for the spectrum on each call. Use [`adapt_into`] in tight
/// loops where you can reuse the Vec.
#[inline]
pub fn adapt(v1: &V1) -> V0 {
    V0 {
        sample_rate:  v1.sample_rate,
        timestamp_ms: v1.timestamp_ms,
        rms:          v1.rms,
        beat:         v1.beat,
        bass:         v1.bass_energy,
        mid:          v1.mid_energy,
        high:         v1.high_energy,
        spectrum:     v1.spectrum.to_vec(),
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::contracts::{AudioFeatures as V1, SPECTRUM_LEN};

    fn make_v1(beat: bool, ts: u64, bass: f32) -> V1 {
        let mut v1 = V1::default();
        v1.timestamp_ms   = ts;
        v1.sample_rate    = 48_000;
        v1.rms            = 0.5;
        v1.peak           = 0.8;
        v1.beat           = beat;
        v1.onset          = beat;
        v1.bpm            = 120.0;
        v1.bass_energy    = bass;
        v1.mid_energy     = 0.3;
        v1.high_energy    = 0.1;
        v1.spectral_flux  = 0.05;
        // Fill spectrum with a recognisable pattern
        for (i, s) in v1.spectrum.iter_mut().enumerate() {
            *s = i as f32 / SPECTRUM_LEN as f32;
        }
        v1
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
        let mut v1 = V1::default();
        v1.rms = f32::NAN;
        v1.bass_energy = f32::INFINITY;
        v1.mid_energy = f32::NEG_INFINITY;
        v1.spectrum.fill(f32::NAN);
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
}
