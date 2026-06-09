//! The Timeline: composes tracks/clips into one frame at time `t`. It **implements
//! [`Effect`]** so the existing render→send pipeline (and triple buffer) drive it directly.
//! Composition is allocation-free: it reuses a pre-sized per-pixel scratch buffer.
//!
//! ## Rendering contract
//! - `render` is **read-only on the data model** — never mutates clips/keyframes.
//! - The same `t` always produces the same frame (non-destructive, deterministic).
//! - Fades and blends are computed in the same linear-ish space the effects work in
//!   (gamma correction happens once, at the output edge in `led-protocols`).

use std::cell::RefCell;

use led_core::PixelColor;
use led_pixel_engine::{Effect, Vec3};

use crate::model::{BlendMode, Clip, Keyframe, TimeMarker, Track};

// ─── Scratch buffer ──────────────────────────────────────────────────────────

struct Scratch {
    clip_buf: Vec<PixelColor>,
}

// ─── Timeline ────────────────────────────────────────────────────────────────

/// Non-destructive timeline. Add tracks/markers at build time; call `render` at any `t`.
pub struct Timeline {
    tracks: Vec<Track>,
    n: usize,
    /// Optional total duration (ms). `None` = unbounded; informational only.
    pub duration_ms: Option<u64>,
    /// Beat / section / cue markers — advisory metadata, never read on the hot-path.
    pub markers: Vec<TimeMarker>,
    scratch: RefCell<Scratch>,
}

impl Timeline {
    /// Create a timeline for `pixel_count` logical pixels.
    pub fn new(pixel_count: usize) -> Self {
        Self {
            tracks: Vec::new(),
            n: pixel_count,
            duration_ms: None,
            markers: Vec::new(),
            scratch: RefCell::new(Scratch { clip_buf: vec![PixelColor::default(); pixel_count] }),
        }
    }

    pub fn with_track(mut self, track: Track) -> Self {
        self.tracks.push(track);
        self
    }

    /// Set an explicit total duration (informational; does not gate rendering).
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    /// Attach beat/section/cue markers (from led-audio or TempoMap).
    pub fn with_markers(mut self, markers: Vec<TimeMarker>) -> Self {
        self.markers = markers;
        self
    }

    pub fn pixel_count(&self) -> usize {
        self.n
    }
}

// ─── Keyframe interpolation ──────────────────────────────────────────────────

/// Evaluate opacity at timeline time `t`, applying each keyframe's `easing` curve.
///
/// The easing of keyframe **B** is applied to the factor that moves from A toward B —
/// so a `Step` easing on B means an instant jump to B's value at the midpoint between
/// A and B (exactly what the SKILL.md "hard cut" behaviour requires).
fn sample_opacity(kfs: &[Keyframe], t: u64) -> f32 {
    match kfs {
        []     => 1.0,
        [only] => only.value,
        _      => {
            if t <= kfs[0].time_ms {
                return kfs[0].value;
            }
            let last = &kfs[kfs.len() - 1];
            if t >= last.time_ms {
                return last.value;
            }
            for w in kfs.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if t >= a.time_ms && t < b.time_ms {
                    let span = (b.time_ms - a.time_ms).max(1) as f32;
                    let raw   = (t - a.time_ms) as f32 / span;
                    let eased = b.easing.apply(raw); // b's easing: how we arrive at b
                    return a.value + (b.value - a.value) * eased;
                }
            }
            last.value
        }
    }
}

// ─── Clip alpha ──────────────────────────────────────────────────────────────

/// Effective alpha of a clip at `t`: fade-in × fade-out × keyframe opacity.
/// Returns 0.0 if the clip is not active at `t`.
fn clip_alpha(clip: &Clip, t: u64) -> f32 {
    if t < clip.start_ms || t >= clip.end_ms {
        return 0.0;
    }
    let local     = t - clip.start_ms;
    let remaining = clip.end_ms - t;
    let mut a = 1.0_f32;

    if clip.fade_in_ms  > 0 && local     < clip.fade_in_ms  {
        a *= local     as f32 / clip.fade_in_ms  as f32;
    }
    if clip.fade_out_ms > 0 && remaining < clip.fade_out_ms {
        a *= remaining as f32 / clip.fade_out_ms as f32;
    }

    a * sample_opacity(&clip.opacity, t)
}

// ─── Blend ───────────────────────────────────────────────────────────────────

/// Blend `src` (with alpha `a`) onto `out` using `mode`.
/// All arithmetic is in the linear-ish 8-bit space that effects produce;
/// gamma is applied once at the output edge (led-protocols / Hal), never here.
#[inline]
fn lerp_u8(dst: u8, src: f32, a: f32) -> u8 {
    (src * a + dst as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8
}

fn blend(mode: BlendMode, out: &mut [PixelColor], src: &[PixelColor], a: f32) {
    match mode {
        BlendMode::Override => {
            for (o, s) in out.iter_mut().zip(src) {
                o.r = lerp_u8(o.r, s.r as f32, a);
                o.g = lerp_u8(o.g, s.g as f32, a);
                o.b = lerp_u8(o.b, s.b as f32, a);
            }
        }
        BlendMode::Add => {
            for (o, s) in out.iter_mut().zip(src) {
                o.r = (o.r as f32 + s.r as f32 * a).min(255.0) as u8;
                o.g = (o.g as f32 + s.g as f32 * a).min(255.0) as u8;
                o.b = (o.b as f32 + s.b as f32 * a).min(255.0) as u8;
            }
        }
        BlendMode::Multiply => {
            for (o, s) in out.iter_mut().zip(src) {
                let mul = |d: u8, sc: u8| d as f32 * sc as f32 / 255.0;
                o.r = lerp_u8(o.r, mul(o.r, s.r), a);
                o.g = lerp_u8(o.g, mul(o.g, s.g), a);
                o.b = lerp_u8(o.b, mul(o.b, s.b), a);
            }
        }
    }
}

// ─── Effect impl ─────────────────────────────────────────────────────────────

impl Effect for Timeline {
    /// Compose all active clips at timeline time `time_ms` into `out`.
    /// Starts from black; clips blend on top in track order (bottom → top).
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        out.fill(PixelColor::default()); // black base
        let mut sc = self.scratch.borrow_mut();
        let clip_buf = sc.clip_buf.as_mut_slice();

        for track in &self.tracks {
            for clip in &track.clips {
                let a = clip_alpha(clip, time_ms);
                if a <= 0.0 {
                    continue; // skip inactive clips — O(active clips) in the common case
                }
                // Clip-local time: the effect always starts at t=0 when the clip begins.
                let local = time_ms - clip.start_ms;
                clip.effect.render(local, positions, clip_buf);
                blend(track.blend, out, clip_buf, a);
            }
        }
    }
}
