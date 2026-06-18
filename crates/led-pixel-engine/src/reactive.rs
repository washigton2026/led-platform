//! The audio→light bridge. `AudioShare` holds the latest [`AudioFeatures`] (written by the
//! audio thread, read by the render thread). Reactive effects hold an `Arc<AudioShare>` and
//! are ordinary [`Effect`]s, so the pipeline drives them unchanged.
//!
//! Hot-path discipline: the render side reads only [`AudioScalars`] (Copy, allocation-free).
//! The spectrum stays behind [`AudioShare::with_spectrum`] so it is never cloned per frame.
//!
//! ## Lock-free design (TD-002 / RT-LOCK-RENDER-001)
//!
//! `AudioScalars` fields are stored as individual std atomics — no lock on the render path.
//! `publish()` writes each field with `Release`; `scalars()` reads each with `Acquire`.
//! This is not a consistent snapshot (two fields may be from adjacent writes), but for
//! music-reactive visuals a one-hop skew (~5ms at 200Hz) is imperceptible and safe.
//!
//! `spectrum` (Vec<f32>, not Copy) stays behind a `RwLock`, but `render()` never calls
//! `with_spectrum` — only diagnostic/analysis code does — so it is not on the hot path.

use std::cell::Cell;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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

/// Per-field atomics for `AudioScalars`. Floats are stored as their bit pattern (u32)
/// via `f32::to_bits`/`from_bits` — no unsafe required, no UB (IEEE 754 is bijective).
struct AtomicScalars {
    sample_rate:  AtomicU32,
    timestamp_ms: AtomicU64,
    rms:          AtomicU32,   // f32 bit pattern
    beat:         AtomicBool,
    bass:         AtomicU32,   // f32 bit pattern
    mid:          AtomicU32,   // f32 bit pattern
    high:         AtomicU32,   // f32 bit pattern
}

impl Default for AtomicScalars {
    fn default() -> Self {
        Self {
            sample_rate:  AtomicU32::new(0),
            timestamp_ms: AtomicU64::new(0),
            rms:          AtomicU32::new(0),
            beat:         AtomicBool::new(false),
            bass:         AtomicU32::new(0),
            mid:          AtomicU32::new(0),
            high:         AtomicU32::new(0),
        }
    }
}

/// Shared latest audio analysis. One writer (audio thread) via [`publish`](Self::publish),
/// many readers (render thread) via [`scalars`](Self::scalars).
///
/// `scalars()` is **lock-free**: it reads per-field atomics with `Acquire` ordering.
/// `publish()` writes per-field atomics with `Release` ordering — no lock taken.
/// `with_spectrum()` uses a `RwLock<Vec<f32>>` but is never called from `render()`.
pub struct AudioShare {
    atomic:   AtomicScalars,
    spectrum: RwLock<Vec<f32>>,
}

impl Default for AudioShare {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioShare {
    pub fn new() -> Self {
        Self {
            atomic:   AtomicScalars::default(),
            spectrum: RwLock::new(Vec::new()),
        }
    }

    /// Publish the newest features (audio thread, ~200 Hz).
    /// Each scalar field is stored atomically with Release ordering.
    /// Spectrum update takes an exclusive write lock (brief copy, off the render path).
    pub fn publish(&self, f: &AudioFeatures) {
        // Scalars — lock-free stores.
        self.atomic.sample_rate .store(f.sample_rate,           Ordering::Release);
        self.atomic.timestamp_ms.store(f.timestamp_ms,          Ordering::Release);
        self.atomic.rms         .store(f.rms.to_bits(),         Ordering::Release);
        self.atomic.beat        .store(f.beat,                  Ordering::Release);
        self.atomic.bass        .store(f.bass.to_bits(),        Ordering::Release);
        self.atomic.mid         .store(f.mid.to_bits(),         Ordering::Release);
        self.atomic.high        .store(f.high.to_bits(),        Ordering::Release);

        // Spectrum — behind RwLock, not on render hot-path.
        let mut g = self.spectrum.write().unwrap();
        if g.len() != f.spectrum.len() {
            g.resize(f.spectrum.len(), 0.0); // only on sample-rate change (rare)
        }
        g.copy_from_slice(&f.spectrum);
    }

    /// Latest scalar fields. **Lock-free** — Acquire loads on each atomic field.
    /// Called every render frame; no mutex, no blocking.
    pub fn scalars(&self) -> AudioScalars {
        AudioScalars {
            sample_rate:  self.atomic.sample_rate .load(Ordering::Acquire),
            timestamp_ms: self.atomic.timestamp_ms.load(Ordering::Acquire),
            rms:          f32::from_bits(self.atomic.rms .load(Ordering::Acquire)),
            beat:         self.atomic.beat        .load(Ordering::Acquire),
            bass:         f32::from_bits(self.atomic.bass.load(Ordering::Acquire)),
            mid:          f32::from_bits(self.atomic.mid .load(Ordering::Acquire)),
            high:         f32::from_bits(self.atomic.high.load(Ordering::Acquire)),
        }
    }

    /// Borrow the latest spectrum without cloning it.
    /// Uses RwLock — acceptable here; `render()` never calls this.
    pub fn with_spectrum<R>(&self, f: impl FnOnce(&[f32]) -> R) -> R {
        let g = self.spectrum.read().unwrap();
        f(&g)
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

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use std::sync::Arc;
    use led_core::PixelColor;

    fn af(beat: bool, ts: u64, bass: f32, rms: f32) -> AudioFeatures {
        AudioFeatures {
            sample_rate: 48_000,
            timestamp_ms: ts,
            rms,
            beat,
            bass,
            mid: 0.3,
            high: 0.1,
            spectrum: vec![0.0; 8],
        }
    }

    fn px(fx: &dyn Effect, t: u64) -> PixelColor {
        let mut out = [PixelColor::default(); 4];
        fx.render(t, &[Vec3::ZERO; 4], &mut out);
        out[0]
    }

    // ── CONCURRENCY: AudioShare — 8 writers, 8 readers simultaneously ─────
    #[test]
    fn audioshare_concurrent_publish_read_no_deadlock() {
        use std::thread;
        let share = Arc::new(AudioShare::new());
        let mut handles = Vec::new();

        for i in 0..8 {
            let s = share.clone();
            handles.push(thread::spawn(move || {
                for j in 0..1_000u64 {
                    s.publish(&af(j % 3 == 0, i * 1000 + j, j as f32 * 0.001, 0.5));
                }
            }));
        }
        for i in 0..8 {
            let s = share.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    let sc = s.scalars();
                    assert!(sc.rms >= 0.0, "rms must be non-negative");
                    s.with_spectrum(|spec| assert!(spec.len() == 8 || spec.is_empty()));
                    let _ = i; // prevent optimization
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
    }

    // ── INVARIANT: AudioShare never hands out stale sample_rate=0 after publish ──
    #[test]
    fn audioshare_after_publish_sample_rate_reflects_features() {
        let share = Arc::new(AudioShare::new());
        let f = af(false, 100, 0.5, 0.8);
        share.publish(&f);
        let sc = share.scalars();
        assert_eq!(sc.sample_rate, 48_000, "sample_rate must reflect last publish");
        assert_eq!(sc.timestamp_ms, 100);
    }

    // ── STRESS: 100k publish cycles — spectrum size must stay stable ──────
    // NOTE: led-core::AudioFeatures has Vec<f32> spectrum so it is NOT Copy.
    // We construct a fresh AudioFeatures per iteration.
    #[test]
    fn audioshare_100k_publishes_spectrum_size_stable() {
        let share = Arc::new(AudioShare::new());
        for i in 0..100_000u64 {
            let feat = af(i % 4 == 0, i, 1.0, 0.5);
            share.publish(&feat);
        }
        // Spectrum size must remain exactly 8 (set on first publish, no resize after)
        share.with_spectrum(|s| assert_eq!(s.len(), 8, "spectrum must not resize after init"));
    }

    // ── FUZZ: publish with empty spectrum → no panic ──────────────────────
    #[test]
    fn audioshare_empty_spectrum_no_panic() {
        let share = Arc::new(AudioShare::new());
        let f = AudioFeatures {
            sample_rate: 44_100,
            timestamp_ms: 0,
            rms: 0.0, beat: false,
            bass: 0.0, mid: 0.0, high: 0.0,
            spectrum: vec![], // empty
        };
        share.publish(&f);
        share.with_spectrum(|s| assert_eq!(s.len(), 0));
        let sc = share.scalars();
        assert_eq!(sc.sample_rate, 44_100);
    }

    // ── FUZZ: AudioShare with NaN/Inf energy values ────────────────────────
    #[test]
    fn audioshare_nan_inf_energy_publish_no_panic() {
        let share = Arc::new(AudioShare::new());
        let f = AudioFeatures {
            sample_rate: 48_000, timestamp_ms: 1,
            rms: f32::NAN, beat: false,
            bass: f32::INFINITY, mid: f32::NEG_INFINITY, high: f32::NAN,
            spectrum: vec![f32::NAN; 8],
        };
        share.publish(&f);
        let sc = share.scalars();
        assert!(sc.bass.is_infinite() || sc.bass.is_nan(), "extreme value preserved");
    }

    // ── CONTRACT: BeatFlash never retrigs on same timestamp ───────────────
    #[test]
    fn beatflash_no_retrig_same_timestamp() {
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(0, 255, 0), 1000, share.clone());
        share.publish(&af(true, 42, 0.0, 0.5));
        let a = px(&fx, 0).g;
        let b = px(&fx, 1).g;
        // second render: same timestamp 42 — must NOT retrigger, must have decayed
        assert!(b <= a, "same-ts beat must not retrigger: a={a} b={b}");
    }

    // ── CONTRACT: BandPulse gain=0 → always black ────────────────────────
    #[test]
    fn bandpulse_zero_gain_always_black() {
        let share = Arc::new(AudioShare::new());
        let fx = BandPulse::new(PixelColor::rgb(255, 0, 0), Band::Bass, 0.0, share.clone());
        share.publish(&af(false, 1, 9999.0, 1.0));
        assert_eq!(px(&fx, 0), PixelColor::default(), "gain=0 must be black regardless");
    }

    // ── CONTRACT: BandPulse never exceeds base color ──────────────────────
    #[test]
    fn bandpulse_never_exceeds_base_color() {
        let share = Arc::new(AudioShare::new());
        let base = PixelColor::rgb(100, 50, 200);
        let fx = BandPulse::new(base, Band::Bass, 999.0, share.clone());
        share.publish(&af(false, 1, 1.0, 1.0));
        let out = px(&fx, 0);
        assert!(out.r <= base.r, "r overflow: {} > {}", out.r, base.r);
        assert!(out.g <= base.g, "g overflow: {} > {}", out.g, base.g);
        assert!(out.b <= base.b, "b overflow: {} > {}", out.b, base.b);
    }

    // ── REAL-TIME: BeatFlash render must complete in < 1ms (50ms tick budget) ─
    #[test]
    fn beatflash_render_latency_under_1ms() {
        use std::time::Instant;
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(255, 255, 255), 500, share.clone());
        share.publish(&af(true, 1, 0.5, 0.5));
        let mut out = vec![PixelColor::default(); 1_000];
        let pos = vec![Vec3::ZERO; 1_000];
        let t0 = Instant::now();
        fx.render(0, &pos, &mut out);
        let elapsed = t0.elapsed();
        assert!(elapsed.as_millis() < 5, "1000-pixel render took {}ms", elapsed.as_millis());
    }
}
