//! Capacity benchmark: how many pixels can the LUMYX pipeline drive at 40 fps?
//!
//! Renders Plasma over N pixels and pushes every frame through the full
//! production path (effect → LogicalFrame → Hal mapping → DeviceDriver) to
//! simulated controllers, timing the whole per-frame cost.
//!
//! Run in RELEASE — debug numbers are meaningless:
//! ```text
//! cargo run --release -p led-demo --example capacity_bench
//! ```
//!
//! Context: the user's current rig is 6,200 px on 5 WLED ESP32s (WiFi ArtNet,
//! practical ceiling ~2,500 px/controller). This bench shows the PLATFORM's
//! headroom; the wire budget then decides controllers-per-rig, not software.

use std::time::Instant;

use led_core::{CompiledLayout, DeviceSpec, LogicalFrame, PixelColor, ProtocolOutput, RgbOrder};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{ComputeEffect, Effect, Plasma, Vec3};

const FRAMES: u64 = 100;
const BUDGET_MS: f64 = 25.0; // 40 fps

fn bench(pixel_count: usize) -> (f64, f64) {
    // ceil(px*3 / 510) universes, controllers of 28 universes each — mirrors
    // the real rig's per-robot allocation. Every device gets the full 28;
    // unused capacity is harmless.
    let universes_needed = (pixel_count * 3).div_ceil(510) as u16;
    let per_device = 28u16;
    let device_count = universes_needed.div_ceil(per_device).max(1);
    let specs: Vec<DeviceSpec> = (0..device_count)
        .map(|id| DeviceSpec { id, universes: per_device })
        .collect();

    let layout = CompiledLayout::linear(pixel_count, &specs, RgbOrder::Rgb);
    let devices: Vec<_> = (0..device_count)
        .map(|id| SimulatorDevice::new(id, layout.device_universes(id)))
        .collect();
    let hal = Hal::new(
        CompiledLayout::linear(pixel_count, &specs, RgbOrder::Rgb),
        devices.iter().map(|d| d.clone() as _).collect(),
    );

    let effect = ComputeEffect::new(Plasma { scale: 0.02, speed: 1.0 });
    let positions: Vec<Vec3> = (0..pixel_count)
        .map(|i| Vec3::new((i % 1000) as f32, (i / 1000) as f32, 0.0))
        .collect();
    let mut buf = vec![PixelColor::default(); pixel_count];

    let mut total_ms = 0.0f64;
    let mut max_ms = 0.0f64;
    for f in 0..FRAMES {
        let t0 = Instant::now();
        effect.render(f * 25, &positions, &mut buf);
        hal.send_frame(&LogicalFrame::new(buf.clone(), f * 25)).unwrap();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        total_ms += ms;
        max_ms = max_ms.max(ms);
    }
    (total_ms / FRAMES as f64, max_ms)
}

fn main() {
    println!("LUMYX capacity bench — {FRAMES} frames each, full pipeline (render+map+send)");
    println!("{:>10} {:>12} {:>10} {:>10}  verdict (budget {BUDGET_MS}ms @40fps)",
        "pixels", "controllers", "avg ms", "max ms");
    for px in [6_200usize, 24_800, 62_000, 124_000, 248_000] {
        let controllers = ((px * 3).div_ceil(510) as u16).div_ceil(28).max(1);
        let (avg, max) = bench(px);
        let verdict = if avg <= BUDGET_MS { "OK — fits 40fps" } else { "exceeds budget" };
        println!("{px:>10} {controllers:>12} {avg:>10.2} {max:>10.2}  {verdict}");
    }
}
