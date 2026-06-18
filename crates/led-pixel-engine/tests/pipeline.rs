//! Proves the full render→send pipeline across two real threads: an effect renders frames,
//! the triple buffer hands them off, the HAL maps + fans out to a device. End to end.

use std::sync::Arc;
use std::time::{Duration, Instant};

use led_core::{CompiledLayout, DeviceDriver, DeviceSpec, ProtocolOutput, RgbOrder};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{spawn, SolidColor, Vec3};
use led_core::PixelColor;

#[test]
fn render_send_pipeline_drives_a_device() {
    const N: usize = 50;

    // HAL + simulator behind a ProtocolOutput.
    let layout = CompiledLayout::linear(N, &[DeviceSpec { id: 1, universes: 1 }], RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    let out: Arc<dyn ProtocolOutput> = Arc::new(Hal::new(layout, devices));

    // A solid-red effect through the render→send pipeline.
    let positions = vec![Vec3::ZERO; N];
    let handle = spawn(Box::new(SolidColor(PixelColor::rgb(255, 0, 0))), positions, out, 200);

    // Causal barrier: wait for ≥1 real frame instead of sleeping 120ms.
    let deadline = Instant::now() + Duration::from_secs(5);
    while sim.frames_sent() < 1 {
        assert!(Instant::now() < deadline, "timeout: no frame reached the device within 5s");
        std::thread::sleep(Duration::from_millis(1));
    }
    handle.stop(); // stops + joins both threads, with a final drain

    // Frames flowed, and the device shows the rendered color (RGB order: pixel 0 → ch 0..3).
    assert!(sim.frames_sent() >= 1, "no frames reached the device");
    assert_eq!(sim.channel(0, 0), Some(255));
    assert_eq!(sim.channel(0, 1), Some(0));
    assert_eq!(sim.channel(0, 2), Some(0));
}
