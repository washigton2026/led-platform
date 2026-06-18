//! Proves the IDevice management plane and the independent heartbeat thread.

use std::sync::Arc;
use std::time::{Duration, Instant};

use led_hal::*;

/// Spin-wait until `condition()` is true or `timeout` elapses.
/// 1ms poll keeps CPU load low while removing the fixed sleep bias.
fn wait_for(condition: impl Fn() -> bool, timeout: Duration, msg: &str) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timeout waiting for: {msg}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn one_device() -> (Arc<SimulatorDevice>, Hal) {
    let layout = CompiledLayout::linear(100, &[DeviceSpec { id: 1, universes: 1 }], RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    (sim, Hal::new(layout, devices))
}

#[test]
fn idevice_lifecycle_and_firmware_safety() {
    let (sim, hal) = one_device();

    assert!(sim.status().connected, "connected by default");

    // configure stores config
    let cfg = DeviceConfig { name: Some("tree".into()), priority: Some(120) };
    sim.configure(&cfg).unwrap();
    assert_eq!(sim.config(), cfg);

    // firmware refused while the device is live
    assert!(sim.update_firmware(b"fw").is_err(), "must refuse firmware on a live device");

    // disconnect, then firmware allowed; empty image rejected
    sim.disconnect();
    assert!(!sim.status().connected);
    sim.update_firmware(b"fw").unwrap();
    assert_eq!(sim.firmware_updates(), 1);
    assert!(sim.update_firmware(b"").is_err(), "empty image rejected");

    // a frame bumps the counter; reboot resets it
    sim.connect().unwrap();
    hal.send_frame(&LogicalFrame::new(vec![PixelColor::default(); 100], 0)).unwrap();
    assert!(sim.frames_sent() >= 1);
    sim.reboot().unwrap();
    assert_eq!(sim.frames_sent(), 0, "reboot resets the frame counter");
}

#[test]
fn heartbeat_thread_keeps_sending_the_last_valid_frame() {
    let (sim, hal) = one_device();
    let out: Arc<dyn ProtocolOutput> = Arc::new(hal);

    let hb = Arc::new(Heartbeat::new());
    let mut pixels = vec![PixelColor::default(); 100];
    pixels[0] = PixelColor::rgb(255, 0, 0);
    hb.record(&LogicalFrame::new(pixels, 0));

    let handle = hb.clone().spawn(out, Duration::from_millis(20));

    // Causal barrier: wait until ≥3 heartbeat frames arrive instead of sleeping.
    wait_for(|| sim.frames_sent() >= 3, Duration::from_secs(5),
             "heartbeat must fire ≥3× at 20ms interval");
    handle.stop();

    let n = sim.frames_sent();
    assert!(n >= 3, "heartbeat should have beat several times, got {n}");
    assert_eq!(sim.channel(0, 0), Some(255), "resent the real frame, never zeros");
}
