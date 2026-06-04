//! Proves the HAL contract end to end against a virtual device.

use std::sync::Arc;

use led_hal::*;

const PIXELS: usize = 171; // 170 pixels fill universe 0 (510 ch); pixel 170 spills to universe 1

/// Two devices, one universe each. Device 1 owns universe 0, device 2 owns universe 1.
fn setup() -> (Arc<SimulatorDevice>, Arc<SimulatorDevice>, Hal) {
    let specs = [DeviceSpec { id: 1, universes: 1 }, DeviceSpec { id: 2, universes: 1 }];
    let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Grb);
    let sim1 = SimulatorDevice::new(1, layout.device_universes(1));
    let sim2 = SimulatorDevice::new(2, layout.device_universes(2));
    let devices: Vec<std::sync::Arc<dyn DeviceDriver>> = vec![sim1.clone(), sim2.clone()];
    let hal = Hal::new(layout, devices);
    (sim1, sim2, hal)
}

#[test]
fn mapping_applied_exactly_once_per_frame() {
    let (_s1, _s2, hal) = setup();
    let frame = LogicalFrame::new(vec![PixelColor::rgb(10, 20, 30); PIXELS], 0);

    hal.send_frame(&frame).unwrap();
    assert_eq!(hal.layout().apply_count(), 1, "first frame: mapped once");

    hal.send_frame(&frame).unwrap();
    assert_eq!(hal.layout().apply_count(), 2, "second frame: mapped once more, never twice");
}

#[test]
fn fanout_each_device_gets_only_its_own_universes() {
    let (sim1, sim2, hal) = setup();

    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(255, 0, 0); // -> device 1, universe 0, channels 0..3
    pixels[170] = PixelColor::rgb(0, 0, 255); // -> device 2, universe 1, channels 0..3
    let frame = LogicalFrame::new(pixels, 0);

    hal.send_frame(&frame).unwrap();

    // GRB order: red -> [g,r,b] = [0,255,0]; blue -> [0,0,255].
    assert_eq!(sim1.channel(0, 0), Some(0));
    assert_eq!(sim1.channel(0, 1), Some(255));
    assert_eq!(sim1.channel(0, 2), Some(0));

    assert_eq!(sim2.channel(1, 0), Some(0));
    assert_eq!(sim2.channel(1, 1), Some(0));
    assert_eq!(sim2.channel(1, 2), Some(255));

    // Device 1 never sees device 2's universe.
    assert_eq!(sim1.channel(1, 0), None, "device 1 must not hold universe 1");
    assert_eq!(sim2.channel(0, 0), None, "device 2 must not hold universe 0");

    assert_eq!(sim1.frames_sent(), 1);
    assert_eq!(sim2.frames_sent(), 1);
}

#[test]
fn heartbeat_resends_last_valid_and_never_zeros() {
    let (sim1, _s2, hal) = setup();
    let hb = Heartbeat::new();

    // No valid frame yet: a beat must send NOTHING — never a fabricated zero frame.
    assert_eq!(hb.beat(&hal).unwrap(), false, "no frame yet => nothing sent");
    assert_eq!(sim1.frames_sent(), 0, "must not blast a blackout frame");

    // Record a non-zero frame, then beat: the LAST VALID frame is resent.
    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(255, 0, 0);
    hb.record(&LogicalFrame::new(pixels, 0));

    assert_eq!(hb.beat(&hal).unwrap(), true, "valid frame exists => resent");
    assert_eq!(sim1.channel(0, 1), Some(255), "resent the real frame, not zeros");
    assert_eq!(sim1.frames_sent(), 1);
}

#[test]
fn core_reaches_hardware_only_through_protocol_output() {
    let (_s1, _s2, hal) = setup();

    // The Core is constructed from a `dyn ProtocolOutput`. It has no access to any device.
    let core = Core::new(Arc::new(hal));
    let frame = LogicalFrame::new(vec![PixelColor::rgb(1, 2, 3); PIXELS], 0);

    core.render_and_send(&frame).unwrap();
    assert_eq!(core.universe_count(), 2);
}
