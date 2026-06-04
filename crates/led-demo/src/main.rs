//! Renders a short LED show to `show.gif` — a watchable artifact that exercises the real
//! render path end to end: a `led-layout` matrix → a `led-sequencer` Timeline composing a
//! GPU-style `Plasma` compute effect (background) with beat-synced white flashes (Add),
//! sampled frame by frame exactly as the pipeline would, then encoded to GIF.
//!
//! Run: `cargo run -p led-demo --release`  → writes ./show.gif

use std::fs::File;

use led_core::PixelColor;
use led_layout::LayoutBuilder;
use led_pixel_engine::{ComputeEffect, Effect, Plasma, SolidColor, Vec3};
use led_sequencer::{BlendMode, Clip, Timeline, Track, TempoMap};

const W: usize = 32; // matrix columns
const H: usize = 18; // matrix rows
const BLOCK: usize = 12; // pixels per cell in the image
const FPS: u64 = 20;
const DURATION_MS: u64 = 6_000;
const BPM: f32 = 120.0;

fn main() {
    let n = W * H;

    // 1. Layout: a 2D matrix. Pixel id = row*W + col (row-major, non-serpentine).
    let mut b = LayoutBuilder::new();
    b.add_matrix("matrix", W as u32, H as u32, false, 1.0);
    let layout = b.build();
    let positions: Vec<Vec3> = layout.pixels.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();

    // 2. Timeline: Plasma background + beat-synced white flashes (Add) on a 120 BPM grid.
    let tempo = TempoMap::constant(BPM, 0);
    let mut beats = Track::new(BlendMode::Add);
    let beat_ms = (60_000.0 / BPM) as u64; // 500 ms
    let mut t = 0u64;
    while t < DURATION_MS {
        // a short flash that fades out over ~half a beat
        beats.clips.push(
            Clip::new(t, t + beat_ms / 2, Box::new(SolidColor(PixelColor::rgb(110, 110, 130))))
                .with_fades(0, beat_ms / 2),
        );
        t = tempo.beat_time((t / beat_ms) + 1); // next beat
    }

    let show = Timeline::new(n)
        .with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, DURATION_MS, Box::new(ComputeEffect::new(Plasma { scale: 0.55, speed: 1.3 })))),
        )
        .with_track(beats);

    // 3. Render every frame and paint into an RGB image buffer.
    let img_w = (W * BLOCK) as u16;
    let img_h = (H * BLOCK) as u16;
    let frame_count = DURATION_MS * FPS / 1000;
    let dt = 1000 / FPS;

    let mut file = File::create("show.gif").expect("create show.gif");
    let mut encoder = gif::Encoder::new(&mut file, img_w, img_h, &[]).expect("gif encoder");
    encoder.set_repeat(gif::Repeat::Infinite).expect("set repeat");

    let mut logical = vec![PixelColor::default(); n];
    let mut rgb = vec![0u8; img_w as usize * img_h as usize * 3];

    for f in 0..frame_count {
        let time_ms = f * dt;
        show.render(time_ms, &positions, &mut logical);
        paint(&logical, &mut rgb, img_w as usize);

        let mut frame = gif::Frame::from_rgb_speed(img_w, img_h, &rgb, 10);
        frame.delay = (100 / FPS) as u16; // GIF delay in 1/100 s
        encoder.write_frame(&frame).expect("write frame");
    }

    println!(
        "wrote show.gif — {frame_count} frames, {img_w}x{img_h}, {W}x{H} matrix, {} px, {BPM} BPM",
        n
    );
}

/// Paint the logical matrix (id = row*W + col) into the RGB image, each pixel a BLOCK×BLOCK cell.
fn paint(logical: &[PixelColor], rgb: &mut [u8], img_w: usize) {
    for (id, c) in logical.iter().enumerate() {
        let col = id % W;
        let row = id / W;
        for dy in 0..BLOCK {
            for dx in 0..BLOCK {
                let x = col * BLOCK + dx;
                let y = row * BLOCK + dy;
                let o = (y * img_w + x) * 3;
                rgb[o] = c.r;
                rgb[o + 1] = c.g;
                rgb[o + 2] = c.b;
            }
        }
    }
}
