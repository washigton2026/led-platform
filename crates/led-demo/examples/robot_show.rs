//! The user's real xLights robot project, end-to-end through LUMYX:
//!
//! import (conflict-gated) → CompiledLayout → Plasma over the robots' real
//! world positions → Hal → 5 simulated WLED controllers → `.lumyx` recording
//! → replay verification → animated GIF preview.
//!
//! ```text
//! cargo run -p led-demo --example robot_show -- "<show dir>" [rgbeffects-file]
//! ```
//!
//! The rgbeffects file defaults to `xlights_rgbeffects.LUMYX-FIXED.xml` (the
//! conflict-free copy written by `led-xlights --fix`); the import gate refuses
//! the original file while its 2,701 channel conflicts stand.

use std::io::Cursor;
use std::path::Path;

use led_core::{compute_pixel_hash, CompiledLayout, LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{ComputeEffect, Effect, Plasma, Vec3};
use led_show_recorder::replay::ReplayManifest;
use led_show_recorder::{finalise_seekable, ShowReader, ShowRecord, ShowWriter};
use led_xlights::import_strings;

const FPS: u64 = 40;
const DURATION_MS: u64 = 6_000;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let effects_file = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "xlights_rgbeffects.LUMYX-FIXED.xml".into());
    let base = Path::new(&dir);

    // ── Import (gate) ─────────────────────────────────────────────────────
    let networks = std::fs::read_to_string(base.join("xlights_networks.xml"))
        .expect("xlights_networks.xml not found");
    let rgbeffects = std::fs::read_to_string(base.join(&effects_file))
        .unwrap_or_else(|_| panic!("{effects_file} not found — run led-xlights --fix first"));
    let report = import_strings(&networks, &rgbeffects);
    println!("import: {}", report.to_json());

    let assigns = report.assignments().expect("conflict gate must pass on the FIXED file");
    println!("gate: OK — {} physical assignments across {} controllers",
        assigns.len(), report.controllers.len());

    // Pixel positions in model order — the same order assignments() emits.
    let positions: Vec<Vec3> = report
        .models
        .iter()
        .flat_map(|m| m.pixel_positions())
        .map(|(x, y, z)| Vec3::new(x, y, z))
        .collect();
    assert_eq!(positions.len(), assigns.len(), "one position per assignment");

    // ── Build the output: 5 simulated WLED controllers ────────────────────
    let layout = CompiledLayout::compile(&assigns);
    let sims: Vec<_> = (0..report.controllers.len() as u16)
        .map(|id| SimulatorDevice::new(id, layout.device_universes(id)))
        .collect();
    let hal = Hal::new(
        CompiledLayout::compile(&assigns),
        sims.iter().map(|s| s.clone() as _).collect(),
    );

    // ── Render + record ───────────────────────────────────────────────────
    // Plasma scaled to the rig's world coordinates (robots span ~3000 units).
    let effect = ComputeEffect::new(Plasma { scale: 0.004, speed: 1.2 });
    let px = positions.len();
    let frame_ms = 1000 / FPS;
    let frames = DURATION_MS / frame_ms;

    let mut backing = Cursor::new(Vec::<u8>::new());
    let mut records: Vec<ShowRecord> = Vec::new();
    {
        let mut writer = ShowWriter::new(&mut backing, px as u32).expect("writer");
        let mut buf = vec![PixelColor::default(); px];
        for f in 0..frames {
            let t = f * frame_ms;
            buf.fill(PixelColor::default());
            effect.render(t, &positions, &mut buf);
            hal.send_frame(&LogicalFrame::new(buf.clone(), t)).expect("send");
            let rec = ShowRecord { timestamp_ms: t, pixels: buf.clone(), audio: None };
            writer.write_frame(&rec).expect("record");
            records.push(rec);
        }
        finalise_seekable(&mut writer).expect("finalise");
    }

    for (i, sim) in sims.iter().enumerate() {
        assert_eq!(sim.frames_sent(), frames, "controller {i} got every frame");
    }
    println!("rendered: {frames} frames × {px} px → 5 controllers ({} frames each)", frames);

    // ── Persist + replay-verify ───────────────────────────────────────────
    let data = backing.into_inner();
    let out_path = "robot_show.lumyx";
    std::fs::write(out_path, &data).expect("write .lumyx");

    let manifest = ReplayManifest::from_records(&records);
    let replayed = ShowReader::new(Cursor::new(data)).expect("reader").collect_all().expect("read");
    let replay_manifest = ReplayManifest::from_records(&replayed);
    assert_eq!(manifest.aggregate_hash, replay_manifest.aggregate_hash, "replay must match render");
    println!("replay: VERIFIED — hash {:#018x} → {out_path}", manifest.aggregate_hash);

    // Sanity: the frames are not blank (Plasma lights the rig).
    let lit = records[10].pixels.iter().filter(|p| p.r > 0 || p.g > 0 || p.b > 0).count();
    assert!(lit > px / 2, "most of the rig must be lit, got {lit}/{px}");

    // ── GIF preview (2D projection of the real robot positions) ──────────
    write_gif_preview(&records, &positions, "robot_show.gif");
    println!("preview: robot_show.gif ({} frames)", frames);

    // Final hash for the determinism ledger.
    let all: Vec<PixelColor> = records.iter().flat_map(|r| r.pixels.iter().copied()).collect();
    println!("pixel_hash(all frames) = {:#018x}", compute_pixel_hash(&all));
}

/// Render the recording as an animated GIF: each pixel is a dot at its real
/// (x, y) world position, normalized into a 640×360 canvas (y flipped —
/// xLights world y grows upward, images grow downward).
fn write_gif_preview(records: &[ShowRecord], positions: &[Vec3], path: &str) {
    const W: u16 = 640;
    const H: u16 = 360;

    let (min_x, max_x) = positions.iter().map(|p| p.x).fold(
        (f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let (min_y, max_y) = positions.iter().map(|p| p.y).fold(
        (f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let scale = ((W as f32 - 20.0) / span_x).min((H as f32 - 20.0) / span_y);

    let file = std::fs::File::create(path).expect("gif file");
    let mut encoder = gif::Encoder::new(file, W, H, &[]).expect("encoder");
    encoder.set_repeat(gif::Repeat::Infinite).expect("repeat");

    // Every 2nd frame at 40fps → 20fps GIF (5 = 50ms per GIF spec unit of 10ms).
    for rec in records.iter().step_by(2) {
        let mut rgb = vec![0u8; W as usize * H as usize * 3];
        for (pos, px) in positions.iter().zip(&rec.pixels) {
            let x = (10.0 + (pos.x - min_x) * scale) as usize;
            let y = (H as f32 - 10.0 - (pos.y - min_y) * scale) as usize;
            for dy in 0..2usize {
                for dx in 0..2usize {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < W as usize && yy < H as usize {
                        let i = (yy * W as usize + xx) * 3;
                        rgb[i] = px.r;
                        rgb[i + 1] = px.g;
                        rgb[i + 2] = px.b;
                    }
                }
            }
        }
        let mut frame = gif::Frame::from_rgb(W, H, &rgb);
        frame.delay = 5; // 50ms
        encoder.write_frame(&frame).expect("frame");
    }
}
