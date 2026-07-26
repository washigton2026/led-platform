//! Generate a `striptest.lumyx` for hardware bring-up (live-validation ETAPA 7):
//! solid R → G → B (confirms DDP paints every pixel in each channel) + a white
//! comet that sweeps the strip (a gap as it passes = a dead pixel), then all-off.
//!
//! Deliberately dim (value 64) and mostly sparse so the total current stays low;
//! WLED's automatic brightness limiter (ABL) is the backstop.
//!
//! ```text
//! make_striptest [pixel_count=720] [out=striptest.lumyx]
//! # then, on the bench:
//! led-player striptest.lumyx --ddp <controller-ip>
//! ```

use led_core::PixelColor;
use led_show_recorder::{finalise_seekable, ShowRecord, ShowWriter};
use std::fs::File;

fn frame(w: &mut ShowWriter<File>, t: u64, pixels: Vec<PixelColor>) -> std::io::Result<()> {
    w.write_frame(&ShowRecord { timestamp_ms: t, pixels, audio: None })
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(720);
    let out = args.next().unwrap_or_else(|| "striptest.lumyx".to_string());

    const V: u8 = 64; // dim test value — low current; ABL is the backstop
    const W: usize = 20; // comet width (pixels)
    const S: usize = 8; // comet step per frame (pixels)
    const DT: u64 = 50; // ms per comet step

    let file = File::create(&out)?;
    let mut w = ShowWriter::new(file, n as u32)?;

    // Solid R / G / B — each held ~1.2 s. Confirms the whole strip lights in
    // every channel over DDP (and that GRB color order is right end-to-end).
    frame(&mut w, 0, vec![PixelColor::rgb(V, 0, 0); n])?;
    frame(&mut w, 1200, vec![PixelColor::rgb(0, V, 0); n])?;
    frame(&mut w, 2400, vec![PixelColor::rgb(0, 0, V); n])?;

    // White comet: a 20-px block stepping across all N pixels. Low current
    // (only ~20 lit at a time) and it exposes any dead/skipped pixel.
    let base = 3600u64;
    let steps = n.div_ceil(S);
    for k in 0..steps {
        let mut px = vec![PixelColor::rgb(0, 0, 0); n];
        let start = k * S;
        let end = (start + W).min(n);
        for slot in &mut px[start..end] {
            *slot = PixelColor::rgb(V, V, V);
        }
        frame(&mut w, base + k as u64 * DT, px)?;
    }

    // All off (clean end state).
    frame(&mut w, base + steps as u64 * DT, vec![PixelColor::rgb(0, 0, 0); n])?;

    finalise_seekable(&mut w)?;
    println!("wrote {out}: {n} px, {} frames", w.frame_count());
    Ok(())
}
