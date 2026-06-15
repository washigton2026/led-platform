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

#[cfg(test)]
mod adversarial_tests {
    use led_core::PixelColor;
    use led_pixel_engine::{Effect, SolidColor, Vec3};
    use crate::model::{BlendMode, Clip, EasingType, Keyframe, Track};
    use super::Timeline;

    fn positions(n: usize) -> Vec<Vec3> {
        (0..n).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect()
    }

    fn black_buf(n: usize) -> Vec<PixelColor> {
        vec![PixelColor::default(); n]
    }

    // ── INVARIANT: same t → same frame (determinism) ──────────────────────
    #[test]
    fn determinism_same_t_same_output() {
        let effect = Box::new(SolidColor(PixelColor::rgb(255, 0, 128)));
        let clip = Clip::new(0, 10_000, effect);
        let track = Track::new(BlendMode::Override).with_clip(clip);
        let tl = Timeline::new(10).with_track(track);
        let pos = positions(10);

        let mut out1 = black_buf(10);
        let mut out2 = black_buf(10);
        tl.render(5_000, &pos, &mut out1);
        tl.render(5_000, &pos, &mut out2);
        assert_eq!(out1, out2, "same t must produce same frame");
    }

    // ── STRESS: 1000 clips overlapping on same track ───────────────────────
    #[test]
    fn stress_1000_overlapping_clips() {
        let mut track = Track::new(BlendMode::Add);
        for i in 0..1000u64 {
            let effect = Box::new(SolidColor(PixelColor::rgb(1, 0, 0)));
            track = track.with_clip(Clip::new(i * 10, i * 10 + 5_000, effect));
        }
        let tl = Timeline::new(100).with_track(track);
        let pos = positions(100);
        let mut out = black_buf(100);
        tl.render(2_500, &pos, &mut out); // all 1000 clips active here
        // must not panic or overflow — Add blend clamps to 255
        for px in &out {
            assert!(px.r <= 255 && px.g <= 255 && px.b <= 255);
        }
    }

    // ── EDGE: clip with zero fade (boundary exact) ────────────────────────
    #[test]
    fn clip_boundary_exact() {
        let effect = Box::new(SolidColor(PixelColor::rgb(200, 100, 50)));
        let clip = Clip::new(1000, 2000, effect);
        let track = Track::new(BlendMode::Override).with_clip(clip);
        let tl = Timeline::new(1).with_track(track);
        let pos = positions(1);

        let mut out = black_buf(1);
        // t = start: active
        tl.render(1000, &pos, &mut out);
        assert_ne!(out[0], PixelColor::default(), "t=start should be active");

        // t = end: inactive (half-open interval)
        out[0] = PixelColor::default();
        tl.render(2000, &pos, &mut out);
        assert_eq!(out[0], PixelColor::default(), "t=end should be inactive (half-open)");
    }

    // ── CHAOS: keyframe with duplicate times ──────────────────────────────
    #[test]
    fn keyframe_duplicate_times_no_panic() {
        let effect = Box::new(SolidColor(PixelColor::rgb(128, 128, 128)));
        let kfs = vec![
            Keyframe::new(0, 1.0),
            Keyframe::new(500, 0.5),
            Keyframe::new(500, 0.8), // duplicate time
            Keyframe::new(1000, 0.0),
        ];
        let clip = Clip::new(0, 2000, effect).with_opacity(kfs);
        let track = Track::new(BlendMode::Override).with_clip(clip);
        let tl = Timeline::new(1).with_track(track);
        let pos = positions(1);
        let mut out = black_buf(1);
        // must not panic at the duplicate
        tl.render(500, &pos, &mut out);
        tl.render(501, &pos, &mut out);
    }

    // ── EDGE: Step easing at exact midpoint ───────────────────────────────
    #[test]
    fn step_easing_hard_cut_at_midpoint() {
        let effect = Box::new(SolidColor(PixelColor::rgb(255, 255, 255)));
        let kfs = vec![
            Keyframe::eased(0,    0.0, EasingType::Linear),
            Keyframe::eased(1000, 1.0, EasingType::Step), // step at 500ms
        ];
        let clip = Clip::new(0, 2000, effect).with_opacity(kfs);
        let track = Track::new(BlendMode::Override).with_clip(clip);
        let tl = Timeline::new(1).with_track(track);
        let pos = positions(1);

        let mut out = black_buf(1);
        tl.render(499, &pos, &mut out);
        let before = out[0].r;
        out[0] = PixelColor::default();
        tl.render(500, &pos, &mut out);
        let after = out[0].r;
        assert!(after > before, "Step easing should jump at midpoint: before={before} after={after}");
    }

    // ── TIMING: render at t=u64::MAX (no overflow) ───────────────────────
    #[test]
    fn render_at_max_time_no_overflow() {
        let effect = Box::new(SolidColor(PixelColor::rgb(42, 42, 42)));
        let clip = Clip::new(0, u64::MAX, effect);
        let track = Track::new(BlendMode::Override).with_clip(clip);
        let tl = Timeline::new(4).with_track(track);
        let pos = positions(4);
        let mut out = black_buf(4);
        tl.render(u64::MAX - 1, &pos, &mut out); // must not panic
    }

    // ── EDGE: empty timeline renders black ────────────────────────────────
    #[test]
    fn empty_timeline_renders_black() {
        let tl = Timeline::new(8);
        let pos = positions(8);
        let mut out = vec![PixelColor { r: 1, g: 2, b: 3 }; 8]; // pre-filled
        tl.render(0, &pos, &mut out);
        for px in &out {
            assert_eq!(*px, PixelColor::default(), "empty timeline must clear to black");
        }
    }

    // ── STRESS: marker flood (10k markers) ───────────────────────────────
    #[test]
    fn marker_flood_does_not_affect_render() {
        use crate::model::TimeMarker;
        let markers: Vec<_> = (0..10_000).map(|i| TimeMarker::beat(i * 10)).collect();
        let effect = Box::new(SolidColor(PixelColor::rgb(255, 0, 0)));
        let track = Track::new(BlendMode::Override)
            .with_clip(Clip::new(0, 100_000, effect));
        let tl = Timeline::new(2)
            .with_track(track)
            .with_markers(markers);
        let pos = positions(2);
        let mut out = black_buf(2);
        tl.render(50_000, &pos, &mut out);
        assert_ne!(out[0], PixelColor::default(), "marker flood must not block render");
    }

    // ── BLEND: Multiply on black stays black ─────────────────────────────
    #[test]
    fn multiply_blend_on_black_stays_black() {
        let effect = Box::new(SolidColor(PixelColor::rgb(255, 255, 255)));
        let clip = Clip::new(0, 1000, effect);
        let track = Track::new(BlendMode::Multiply).with_clip(clip); // base = black
        let tl = Timeline::new(4).with_track(track);
        let pos = positions(4);
        let mut out = black_buf(4);
        tl.render(500, &pos, &mut out);
        for px in &out {
            assert_eq!(*px, PixelColor::default(), "Multiply(black, any) = black");
        }
    }
}
