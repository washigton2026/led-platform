//! Proves a Timeline drives the real render→send pipeline (it IS an Effect): timeline →
//! triple buffer → HAL → device, across two threads.

use std::sync::Arc;
use std::time::Duration;

use led_core::{CompiledLayout, DeviceDriver, DeviceSpec, PixelColor, ProtocolOutput, RgbOrder};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{spawn, SolidColor, Vec3};
use led_sequencer::{BlendMode, Clip, Timeline, Track};

#[test]
fn timeline_drives_render_send_pipeline() {
    const N: usize = 50;

    let layout = CompiledLayout::linear(N, &[DeviceSpec { id: 1, universes: 1 }], RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    let out: Arc<dyn ProtocolOutput> = Arc::new(Hal::new(layout, devices));

    // A timeline whose only clip holds solid red for the whole run.
    let timeline = Timeline::new(N).with_track(
        Track::new(BlendMode::Override)
            .with_clip(Clip::new(0, 10_000, Box::new(SolidColor(PixelColor::rgb(255, 0, 0))))),
    );

    let handle = spawn(Box::new(timeline), vec![Vec3::ZERO; N], out, 200);
    std::thread::sleep(Duration::from_millis(120));
    handle.stop();

    assert!(sim.frames_sent() >= 1, "no frames reached the device");
    assert_eq!(sim.channel(0, 0), Some(255), "timeline's red reached the wire");
    assert_eq!(sim.channel(0, 1), Some(0));
}
