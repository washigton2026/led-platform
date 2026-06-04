//! The Timeline: composes tracks/clips into one frame at time `t`. It **implements
//! [`Effect`]** so the existing render→send pipeline (and triple buffer) drive it directly.
//! Composition is allocation-free: it reuses a pre-sized clip scratch buffer.

use std::cell::RefCell;

use led_core::PixelColor;
use led_pixel_engine::{Effect, Vec3};

use crate::model::{BlendMode, Clip, Keyframe, Track};

struct Scratch {
    clip_buf: Vec<PixelColor>,
}

pub struct Timeline {
    tracks: Vec<Track>,
    n: usize,
    scratch: RefCell<Scratch>,
}

impl Timeline {
    /// A timeline for `pixel_count` logical pixels.
    pub fn new(pixel_count: usize) -> Self {
        Self {
            tracks: Vec::new(),
            n: pixel_count,
            scratch: RefCell::new(Scratch { clip_buf: vec![PixelColor::default(); pixel_count] }),
        }
    }

    pub fn with_track(mut self, track: Track) -> Self {
        self.tracks.push(track);
        self
    }

    pub fn pixel_count(&self) -> usize {
        self.n
    }
}

/// Linear-interpolated opacity at `t` (1.0 if no keyframes).
fn sample_opacity(kfs: &[Keyframe], t: u64) -> f32 {
    match kfs {
        [] => 1.0,
        [only] => only.value,
        _ => {
            if t <= kfs[0].time_ms {
                return kfs[0].value;
            }
            let last = kfs[kfs.len() - 1];
            if t >= last.time_ms {
                return last.value;
            }
            for w in kfs.windows(2) {
                let (a, b) = (w[0], w[1]);
                if t >= a.time_ms && t < b.time_ms {
                    let span = (b.time_ms - a.time_ms) as f32;
                    let f = (t - a.time_ms) as f32 / span;
                    return a.value + (b.value - a.value) * f;
                }
            }
            last.value
        }
    }
}

/// Effective alpha of a clip at `t`: fades × opacity, or 0 if the clip is inactive.
fn clip_alpha(clip: &Clip, t: u64) -> f32 {
    if t < clip.start_ms || t >= clip.end_ms {
        return 0.0;
    }
    let local = t - clip.start_ms;
    let mut a = 1.0;
    if clip.fade_in_ms > 0 && local < clip.fade_in_ms {
        a *= local as f32 / clip.fade_in_ms as f32;
    }
    let remaining = clip.end_ms - t;
    if clip.fade_out_ms > 0 && remaining < clip.fade_out_ms {
        a *= remaining as f32 / clip.fade_out_ms as f32;
    }
    a * sample_opacity(&clip.opacity, t)
}

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
                let m = |d: u8, sc: u8| d as f32 * sc as f32 / 255.0;
                o.r = lerp_u8(o.r, m(o.r, s.r), a);
                o.g = lerp_u8(o.g, m(o.g, s.g), a);
                o.b = lerp_u8(o.b, m(o.b, s.b), a);
            }
        }
    }
}

impl Effect for Timeline {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        out.fill(PixelColor::default()); // start from black; clips blend on top
        let mut sc = self.scratch.borrow_mut();
        let clip_buf = sc.clip_buf.as_mut_slice();

        for track in &self.tracks {
            for clip in &track.clips {
                let a = clip_alpha(clip, time_ms);
                if a <= 0.0 {
                    continue;
                }
                // Clip-local time so each clip's effect starts fresh when it begins.
                let local = time_ms - clip.start_ms;
                clip.effect.render(local, positions, clip_buf);
                blend(track.blend, out, clip_buf, a);
            }
        }
    }
}
