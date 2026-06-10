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
