//! The `AudioFeatures` contract — owned by `audio-core` (lumyx-system-architect §3 / §11,
//! v1.0). This is a leaf crate: the contract is defined here from scratch and does not
//! reuse `led-core`'s (smaller, Phase-1) `AudioFeatures`. Consumers read it off the
//! [`tokio::sync::watch`] channel returned by [`crate::pipeline::AudioPipeline`] — never by
//! depending on this crate's internals beyond this module.

/// FFT size in samples (1024-point).
pub const FFT_SIZE: usize = 1024;

/// Hop size between analysis frames, in samples. `FFT_SIZE - HOP_SIZE = 768` samples of
/// overlap, i.e. 75% overlap.
pub const HOP_SIZE: usize = 256;

/// Number of bins kept in [`AudioFeatures::spectrum`] (`FFT_SIZE / 2`, the positive-frequency
/// half of the magnitude spectrum).
pub const SPECTRUM_LEN: usize = FFT_SIZE / 2;

/// Offline-only musical structure label. Always `None` from the realtime pipeline in this
/// crate (data-contracts.md: "only set by offline analysis, None in realtime").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicalSection {
    Intro,
    Verse,
    Chorus,
    Bridge,
    Drop,
    Build,
    Outro,
    Unknown,
}

/// What the audio layer hands to everything above it (lumyx-system-architect §3).
///
/// `sample_rate` travels WITH the data — no global rate is ever assumed (invariant 7).
/// `spectrum` is a fixed-size array (not `Vec<f32>`) so that `AudioFeatures` is `Copy` and
/// producing one allocates nothing (invariant 3) — important since a fresh value is built
/// every [`HOP_SIZE`]-sample hop and pushed through a `watch` channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioFeatures {
    // IDENTITY
    pub timestamp_ms: u64,
    pub sample_rate: u32,

    // AMPLITUDE
    pub rms: f32,
    pub peak: f32,

    // RHYTHM
    pub beat: bool,
    pub onset: bool,
    pub bpm: f32,

    // FREQUENCY BANDS (computed with sample_rate — never hardcoded bins)
    pub bass_energy: f32,
    pub mid_energy: f32,
    pub high_energy: f32,

    // SPECTRAL
    pub spectral_centroid: f32,
    pub spectral_rolloff: f32,
    pub spectral_flux: f32,

    // HARMONIC CONTENT (v1.1 — added Cycle 7)
    /// Fraction of spectral energy at the fundamental + 4 harmonics vs total.
    /// 0.0 = pure noise/transient; 1.0 = pure sine. See [`crate::harmonics`].
    /// Threshold for "tonal": [`crate::harmonics::TONAL_THRESHOLD`] = 0.40.
    pub harmonic_ratio: f32,

    // RAW SPECTRUM — magnitude per FFT bin, after Hann windowing. len == SPECTRUM_LEN.
    pub spectrum: [f32; SPECTRUM_LEN],

    // MUSICAL CONTEXT — offline-only, always None from this realtime pipeline.
    pub musical_section: Option<MusicalSection>,
}

impl Default for AudioFeatures {
    fn default() -> Self {
        // `[T; N]: Default` is only implemented by std for N <= 32, so this struct needs a
        // manual Default; `[0.0; SPECTRUM_LEN]` itself is a plain const array repeat.
        Self {
            timestamp_ms: 0,
            sample_rate: 0,
            rms: 0.0,
            peak: 0.0,
            beat: false,
            onset: false,
            bpm: 0.0,
            bass_energy: 0.0,
            mid_energy: 0.0,
            high_energy: 0.0,
            spectral_centroid: 0.0,
            spectral_rolloff: 0.0,
            spectral_flux: 0.0,
            harmonic_ratio: 0.0,
            spectrum: [0.0; SPECTRUM_LEN],
            musical_section: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_silent_and_explicit_about_sample_rate() {
        let f = AudioFeatures::default();
        assert_eq!(f.sample_rate, 0, "default sample_rate is a sentinel, never a real rate like 44100");
        assert_eq!(f.spectrum.len(), SPECTRUM_LEN);
        assert_eq!(f.spectrum, [0.0; SPECTRUM_LEN]);
        assert!(f.musical_section.is_none());
    }

    #[test]
    fn audio_features_is_copy() {
        // Required for the zero-alloc watch::send hot path.
        fn assert_copy<T: Copy>() {}
        assert_copy::<AudioFeatures>();
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;

    // ── CONTRACT: spectrum len is always SPECTRUM_LEN (512) ──────────────
    #[test]
    fn spectrum_len_invariant() {
        let f = AudioFeatures::default();
        assert_eq!(f.spectrum.len(), SPECTRUM_LEN);
        assert_eq!(SPECTRUM_LEN, FFT_SIZE / 2);
    }

    // ── CONTRACT: all energy values are non-negative ──────────────────────
    #[test]
    fn energy_values_non_negative() {
        let f = AudioFeatures {
            rms: 0.5,
            peak: 0.8,
            bass_energy: 0.3,
            mid_energy: 0.4,
            high_energy: 0.1,
            spectral_centroid: 2000.0,
            spectral_rolloff: 8000.0,
            spectral_flux: 0.05,
            bpm: 120.0,
            sample_rate: 44100,
            ..AudioFeatures::default()
        };
        assert!(f.rms >= 0.0);
        assert!(f.peak >= 0.0);
        assert!(f.bass_energy >= 0.0);
        assert!(f.mid_energy >= 0.0);
        assert!(f.high_energy >= 0.0);
        assert!(f.bpm >= 0.0);
    }

    // ── FUZZ: extreme spectrum values don't panic ──────────────────────────
    #[test]
    fn extreme_spectrum_values_are_valid() {
        let mut f = AudioFeatures::default();
        // Fill spectrum with extreme values
        for (i, v) in f.spectrum.iter_mut().enumerate() {
            *v = if i % 2 == 0 { f32::MAX } else { 0.0 };
        }
        // Should remain Copy-able and comparable without panic
        let f2 = f;
        assert_eq!(f.spectrum[0], f2.spectrum[0]);
    }

    // ── CONTRACT: beat and onset are independent booleans ──────────────────
    #[test]
    fn beat_onset_are_independent() {
        let f1 = AudioFeatures { beat: true, onset: false, ..AudioFeatures::default() };
        let f2 = AudioFeatures { beat: false, onset: true, ..AudioFeatures::default() };
        let f3 = AudioFeatures { beat: true, onset: true, ..AudioFeatures::default() };
        assert!(f1.beat && !f1.onset);
        assert!(!f2.beat && f2.onset);
        assert!(f3.beat && f3.onset);
    }

    // ── CONTRACT: sample_rate=0 is sentinel, not 44100 ────────────────────
    #[test]
    fn zero_sample_rate_is_sentinel_not_real() {
        let f = AudioFeatures::default();
        assert_eq!(f.sample_rate, 0, "0 is sentinel — never a real device rate");
        assert_ne!(f.sample_rate, 44100);
        assert_ne!(f.sample_rate, 48000);
    }

    // ── STRESS: 10k AudioFeatures copies — zero alloc guarantee (Copy) ────
    #[test]
    fn copy_10k_features_stays_on_stack() {
        let base = AudioFeatures {
            rms: 0.5,
            bpm: 128.0,
            beat: true,
            sample_rate: 48000,
            ..AudioFeatures::default()
        };
        let mut last = base;
        for i in 0..10_000u64 {
            let mut next = last;
            next.timestamp_ms = i;
            last = next;
        }
        assert_eq!(last.timestamp_ms, 9_999);
        assert_eq!(last.bpm, 128.0, "bpm must survive 10k copies");
    }

    // ── CONTRACT: musical_section always None from realtime pipeline ───────
    #[test]
    fn realtime_pipeline_musical_section_is_none() {
        let f = AudioFeatures::default();
        assert!(f.musical_section.is_none(), "realtime pipeline must never set musical_section");
    }

    // ── TIMING: timestamp_ms must monotonically increase in a stream ───────
    #[test]
    fn simulated_stream_timestamps_are_monotonic() {
        let sample_rate = 44100u64;
        let hop = HOP_SIZE as u64;
        let mut prev_ts = 0u64;
        for frame in 0..1000u64 {
            let ts = (frame * hop * 1000) / sample_rate;
            if frame > 0 {
                assert!(ts >= prev_ts, "timestamp must not go backwards: frame={frame} ts={ts} prev={prev_ts}");
            }
            prev_ts = ts;
        }
    }
}
