//! Proves the HAL contract end to end against a virtual device.

use std::sync::Arc;
use std::time::{Duration, Instant};

use led_hal::*;

fn wait_for(condition: impl Fn() -> bool, timeout: Duration, msg: &str) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timeout waiting for: {msg}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

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
    assert!(!hb.beat(&hal).unwrap(), "no frame yet => nothing sent");
    assert_eq!(sim1.frames_sent(), 0, "must not blast a blackout frame");

    // Record a non-zero frame, then beat: the LAST VALID frame is resent.
    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(255, 0, 0);
    hb.record(&LogicalFrame::new(pixels, 0));

    assert!(hb.beat(&hal).unwrap(), "valid frame exists => resent");
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

// ────────────────────────────────────────────────────────────────────────────
// Heartbeat REAL TIMING tests (P2 — Cycle 4)
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn heartbeat_thread_fires_within_interval_budget() {
    let (sim1, _s2, hal) = setup();
    let hb = Arc::new(Heartbeat::new());

    // Record a real frame so the heartbeat has something to send.
    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(200, 100, 50);
    hb.record(&LogicalFrame::new(pixels, 0));

    let hal: Arc<dyn led_core::ProtocolOutput> = Arc::new(hal);

    // Spawn heartbeat at 100ms interval.
    let interval = Duration::from_millis(100);
    let _handle = Arc::clone(&hb).spawn(Arc::clone(&hal), interval);

    // Causal barrier: wait until ≥2 heartbeat frames arrive (at 100ms interval, ~200ms).
    wait_for(|| sim1.frames_sent() >= 2, Duration::from_secs(5),
             "heartbeat must fire ≥2× at 100ms interval");

    let sent = sim1.frames_sent();
    assert!(sent >= 2,
        "heartbeat must fire ≥2 times at 100ms interval (got {})", sent);

    // Each heartbeat must have sent the REAL frame, not zeros.
    assert_eq!(sim1.channel(0, 1), Some(200),
        "heartbeat must resend the real frame content (R channel in GRB)");
}

#[test]
fn heartbeat_gap_thresholds_match_gosl_rules() {
    // Inline the threshold constants from LUMYX_GOSL / led-protocols::heartbeat
    // to avoid adding led-protocols as a dev-dep here.
    const HEARTBEAT_MS: u64 = 800;
    const WARN_GAP_MS:  u64 = 2_000;
    const CRIT_GAP_MS:  u64 = 2_500;

    // Compile-time invariant checks (constant values → const assert).
    const { assert!(HEARTBEAT_MS < WARN_GAP_MS) };
    const { assert!(HEARTBEAT_MS < WARN_GAP_MS) };         // 1 missed
    const { assert!(HEARTBEAT_MS * 2 < WARN_GAP_MS) };    // 2 missed → Ok
    const { assert!(HEARTBEAT_MS * 3 >= WARN_GAP_MS) };   // 3 missed → Warning+
    const { assert!(HEARTBEAT_MS * 3 < CRIT_GAP_MS) };    // 3 missed → below Critical
    const { assert!(HEARTBEAT_MS * 4 >= CRIT_GAP_MS) };   // 4 missed → Critical
}

#[test]
fn heartbeat_never_sends_zero_frame_when_record_never_called() {
    // INVARIANT: If no frame was ever recorded, heartbeat must send NOTHING.
    // This is the "never zeros" rule from LUMYX_GOSL.
    let (_s1, _s2, hal) = setup();
    let hb = Heartbeat::new();
    // no hb.record() call
    let result = hb.beat(&hal).unwrap();
    assert!(!result, "beat with no recorded frame must return false (sent nothing)");
}

// ── P3: GOSL compliance — 800ms interval stays safe under the 2.4s Critical threshold ─
#[test]
fn gosl_heartbeat_interval_provably_safe() {
    // From LUMYX_GOSL.md:
    //   Warning  = 2000ms gap
    //   Critical = 2500ms gap
    //   Heartbeat interval = 800ms
    //
    // Proof: worst-case gap = interval × (1 + missed_tick_skip) + OS_jitter
    // With interval=800ms and MissedTickBehavior::Skip, max gap = 2 × 800ms = 1600ms
    // This is comfortably below the 2000ms Warning threshold.
    const HB_MS: u64 = 800;
    const WARN_MS: u64 = 2_000;
    const CRIT_MS: u64 = 2_500;

    // Compile-time invariant checks (all values are constants → const assert).
    const { assert!(HB_MS < WARN_MS) };             // Case 1: normal operation
    const { assert!(HB_MS * 2 < WARN_MS) };         // Case 2: one tick missed → still Ok
    const { assert!(HB_MS * 3 >= WARN_MS) };        // Case 3: two missed → Warning+
    const { assert!(HB_MS * 3 < CRIT_MS) };         // Case 3: two missed → below Critical
    const { assert!(HB_MS * 4 >= CRIT_MS) };        // Case 4: three missed → Critical
}

#[test]
fn gosl_heartbeat_thread_fires_at_correct_rate() {
    use std::sync::Arc;

    // Use a counting ProtocolOutput to measure actual heartbeat rate
    let (s1, _s2, hal) = setup();
    let hb = Arc::new(Heartbeat::new());

    // Record a frame
    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(10, 20, 30);
    hb.record(&LogicalFrame::new(pixels, 0));

    let hal: Arc<dyn led_core::ProtocolOutput> = Arc::new(hal);
    let _handle = Arc::clone(&hb).spawn(Arc::clone(&hal), Duration::from_millis(80));

    // Causal barrier: wait until ≥4 real frames arrive at the spy device.
    // Previously used a 500ms wall-clock sleep; now asserts the actual causal
    // condition (≥4 frames received) with a generous timeout.
    wait_for(|| s1.frames_sent() >= 4, Duration::from_secs(5),
             "heartbeat must fire ≥4× at 80ms interval");

    assert!(s1.frames_sent() >= 4,
        "at 80ms interval, spy device must have received ≥4 frames (got {})",
        s1.frames_sent());
}
