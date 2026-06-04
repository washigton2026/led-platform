//! The audio→light bridge. `AudioShare` holds the latest [`AudioFeatures`] (written by the
//! audio thread, read by the render thread). Reactive effects hold an `Arc<AudioShare>` and
//! are ordinary [`Effect`]s, so the pipeline drives them unchanged.
//!
//! Hot-path discipline: the render side reads only [`AudioScalars`] (Copy, allocation-free).
//! The spectrum stays behind [`AudioShare::with_spectrum`] so it is never cloned per frame.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use led_core::{AudioFeatures, PixelColor};

use crate::color;
use crate::effect::{Effect, Vec3};

/// The scalar audio fields — cheap to copy, safe to read every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioScalars {
    pub sample_rate: u32,
    pub timestamp_ms: u64,
    pub rms: f32,
    pub beat: bool,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
}

struct Inner {
    scalars: AudioScalars,
    spectrum: Vec<f32>,
}

/// Shared latest audio analysis. One writer (audio thread) via [`publish`](Self::publish),
/// many readers (render thread) via [`scalars`](Self::scalars).
pub struct AudioShare {
    inner: Mutex<Inner>,
}

impl Default for AudioShare {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioShare {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner { scalars: AudioScalars::default(), spectrum: Vec::new() }) }
    }

    /// Publish the newest features (called at the audio analysis rate, not per render frame).
    pub fn publish(&self, f: &AudioFeatures) {
        let mut g = self.inner.lock().unwrap();
        g.scalars = AudioScalars {
            sample_rate: f.sample_rate,
            timestamp_ms: f.timestamp_ms,
            rms: f.rms,
            beat: f.beat,
            bass: f.bass,
            mid: f.mid,
            high: f.high,
        };
        if g.spectrum.len() != f.spectrum.len() {
            g.spectrum.resize(f.spectrum.len(), 0.0); // only on size change (rare)
        }
        g.spectrum.copy_from_slice(&f.spectrum);
    }

    /// Latest scalar fields. Copy — allocation-free, safe on the render hot path.
    pub fn scalars(&self) -> AudioScalars {
        self.inner.lock().unwrap().scalars
    }

    /// Borrow the latest spectrum without cloning it.
    pub fn with_spectrum<R>(&self, f: impl FnOnce(&[f32]) -> R) -> R {
        let g = self.inner.lock().unwrap();
        f(&g.spectrum)
    }
}

/// Brightness follows a band's energy: `level = clamp(band * gain)`. Pick the band via `band`.
pub struct BandPulse {
    pub color: PixelColor,
    pub gain: f32,
    pub band: Band,
    pub audio: Arc<AudioShare>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Bass,
    Mid,
    High,
}

impl BandPulse {
    pub fn new(color: PixelColor, band: Band, gain: f32, audio: Arc<AudioShare>) -> Self {
        Self { color, gain, band, audio }
    }
}

impl Effect for BandPulse {
    fn render(&self, _time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let s = self.audio.scalars();
        let energy = match self.band {
            Band::Bass => s.bass,
            Band::Mid => s.mid,
            Band::High => s.high,
        };
        let level = (energy * self.gain).clamp(0.0, 1.0);
        out.fill(color::scale(self.color, level));
    }
}

#[derive(Clone, Copy)]
struct FlashState {
    last_beat_ts: u64,
    flash_start_ms: u64,
    ever: bool,
}

/// Flashes to full on each new beat, then decays linearly over `decay_ms`.
pub struct BeatFlash {
    pub color: PixelColor,
    pub decay_ms: u64,
    pub audio: Arc<AudioShare>,
    state: Cell<FlashState>,
}

impl BeatFlash {
    pub fn new(color: PixelColor, decay_ms: u64, audio: Arc<AudioShare>) -> Self {
        Self {
            color,
            decay_ms,
            audio,
            state: Cell::new(FlashState { last_beat_ts: u64::MAX, flash_start_ms: 0, ever: false }),
        }
    }
}

impl Effect for BeatFlash {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let s = self.audio.scalars();
        let mut st = self.state.get();
        // Trigger only on a NEW beat (a beat with a timestamp we haven't handled).
        if s.beat && s.timestamp_ms != st.last_beat_ts {
            st.flash_start_ms = time_ms;
            st.last_beat_ts = s.timestamp_ms;
            st.ever = true;
            self.state.set(st);
        }
        let level = if !st.ever || self.decay_ms == 0 {
            0.0
        } else {
            let elapsed = time_ms.saturating_sub(st.flash_start_ms) as f32;
            (1.0 - elapsed / self.decay_ms as f32).clamp(0.0, 1.0)
        };
        out.fill(color::scale(self.color, level));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features(beat: bool, ts: u64, bass: f32) -> AudioFeatures {
        AudioFeatures {
            sample_rate: 48_000,
            timestamp_ms: ts,
            rms: 0.5,
            beat,
            bass,
            mid: 0.0,
            high: 0.0,
            spectrum: vec![0.0; 8],
        }
    }

    fn render(fx: &dyn Effect, t: u64) -> PixelColor {
        let mut out = [PixelColor::default(); 2];
        fx.render(t, &[Vec3::ZERO; 2], &mut out);
        out[0]
    }

    #[test]
    fn band_pulse_tracks_energy() {
        let share = Arc::new(AudioShare::new());
        let fx = BandPulse::new(PixelColor::rgb(200, 0, 0), Band::Bass, 1.0, share.clone());

        share.publish(&features(false, 1, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 0), "no bass ⇒ dark");

        share.publish(&features(false, 2, 0.5));
        assert_eq!(render(&fx, 0), PixelColor::rgb(100, 0, 0), "half bass ⇒ half brightness");

        share.publish(&features(false, 3, 5.0)); // gain*energy clamps to 1
        assert_eq!(render(&fx, 0), PixelColor::rgb(200, 0, 0), "clamped to full");
    }

    #[test]
    fn beat_flash_triggers_and_decays() {
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(0, 0, 200), 1000, share.clone());

        // Before any beat: dark.
        share.publish(&features(false, 1, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 0));

        // New beat at ts=2 → full flash at the render time it's first seen.
        share.publish(&features(true, 2, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 200), "beat ⇒ full");
        // Same beat still latched in the share, but no retrigger; decays with render time.
        assert_eq!(render(&fx, 500), PixelColor::rgb(0, 0, 100), "decays to half at t=500");
        assert_eq!(render(&fx, 1000), PixelColor::rgb(0, 0, 0), "fully decayed at t=1000");

        // A new beat (different ts) re-flashes.
        share.publish(&features(true, 3, 0.0));
        assert_eq!(render(&fx, 1200), PixelColor::rgb(0, 0, 200), "new beat ⇒ full again");
    }
}
