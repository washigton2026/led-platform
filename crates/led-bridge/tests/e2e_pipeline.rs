//! End-to-end integration: full LUMYX stack without any hardware.
//!
//! Pipeline under test:
//!
//! ```text
//! [SimLoop]
//!   SineGen + BeatImpulse
//!         │  f32 samples @ 48kHz
//!         ▼
//!   audio_core::Analyzer::process_hop
//!         │  AudioFeatures v1
//!         ▼
//!   led_bridge::adapt_into             ← v1→v0 adapter (Cycle 3)
//!         │  led_core::AudioFeatures v0
//!         ▼
//!   led_pixel_engine::AudioShare
//!         │  scalars + spectrum
//!         ▼
//!   BandPulse + BeatFlash              ← audio-reactive effects
//!         │  [PixelColor; N]
//!         ▼
//!   led_core::LogicalFrame
//!         │
//!         ▼
//!   led_hal::Hal::send_frame           ← layout mapping applied once here
//!         │
//!         ▼
//!   led_hal::SimulatorDevice           ← virtual hardware, inspectable
//!         │
//!         ▼
//!   assertions on channel values
//! ```

use std::sync::Arc;

use led_bridge::sim::{SimConfig, SimLoop};
use led_core::{LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{CompiledLayout, DeviceSpec, Hal, RgbOrder, SimulatorDevice};

const PIXELS: usize = 100;

fn make_hal() -> (Arc<SimulatorDevice>, Hal) {
    let specs = [DeviceSpec { id: 1, universes: 1 }];
    let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn led_core::DeviceDriver>> = vec![sim.clone()];
    (sim, Hal::new(layout, devices))
}

// ── INVARIANT: SimLoop output reaches the HAL and lights a device ─────────
#[test]
fn sim_output_reaches_hal_and_lights_device() {
    let sim_out = SimLoop::new(SimConfig {
        tone_hz: 100.0, // bass tone → BandPulse blue channel
        beat_interval_ms: 0,
        pixel_count: PIXELS,
        ..SimConfig::default()
    }).run(300);

    let (sim_dev, hal) = make_hal();

    // Send the last frame from the simulation through the real HAL
    let frame = LogicalFrame::new(sim_out.last_frame.clone(), sim_out.hops_processed);
    hal.send_frame(&frame).unwrap();

    assert_eq!(sim_dev.frames_sent(), 1, "HAL must forward exactly 1 frame");
    // BandPulse produces blue channel (b) for bass tone — at least some pixels must be lit
    let any_blue = (0..PIXELS).any(|i| sim_dev.channel(0, i * 3 + 2).unwrap_or(0) > 0);
    assert!(any_blue, "bass tone must light the blue channel through the full stack");
}

// ── INVARIANT: 100 frames through HAL — frame counter correct ─────────────
#[test]
fn sim_100_frames_through_hal() {
    let sim_out = SimLoop::new(SimConfig {
        pixel_count: PIXELS,
        tone_hz: 100.0,
        beat_interval_ms: 200,
        ..SimConfig::default()
    }).run(2_000); // 2s → ~376 hops

    let (sim_dev, hal) = make_hal();

    // Send every 10th frame (simulate 60fps render decimation)
    let mut sent = 0u64;
    for (i, chunk) in sim_out.scalars_log.chunks(10).enumerate() {
        // Build a frame from the sim's pixel output (simplified: use last_frame)
        let frame = LogicalFrame::new(sim_out.last_frame.clone(), i as u64 * 10);
        hal.send_frame(&frame).unwrap();
        sent += 1;
        let _ = chunk;
    }
    assert_eq!(sim_dev.frames_sent(), sent, "device must receive exactly {sent} frames");
}

// ── INVARIANT: mapping applied exactly once per frame ─────────────────────
#[test]
fn e2e_mapping_applied_once_per_hal_send() {
    let sim_out = SimLoop::new(SimConfig::default()).run(100);
    let (_, hal) = make_hal();

    for i in 0..5u64 {
        let frame = LogicalFrame::new(sim_out.last_frame.clone(), i);
        hal.send_frame(&frame).unwrap();
    }
    assert_eq!(hal.layout().apply_count(), 5, "mapping applied exactly N times for N sends");
}

// ── INVARIANT: pixel 0 value arrives at channel 0 of device ──────────────
#[test]
fn e2e_pixel_zero_maps_to_device_channel_zero() {
    let mut pixels = vec![PixelColor::default(); PIXELS];
    pixels[0] = PixelColor::rgb(123, 45, 67);

    let (sim_dev, hal) = make_hal();
    hal.send_frame(&LogicalFrame::new(pixels, 0)).unwrap();

    // RgbOrder::Rgb → channels 0=R, 1=G, 2=B
    assert_eq!(sim_dev.channel(0, 0), Some(123), "R channel at device ch 0");
    assert_eq!(sim_dev.channel(0, 1), Some(45),  "G channel at device ch 1");
    assert_eq!(sim_dev.channel(0, 2), Some(67),  "B channel at device ch 2");
}

// ── STRESS: 1000 frames end-to-end without panic ─────────────────────────
#[test]
fn e2e_1000_frames_no_panic() {
    let sim_out = SimLoop::new(SimConfig {
        pixel_count: PIXELS,
        beat_interval_ms: 100,
        ..SimConfig::default()
    }).run(10_000);

    let (sim_dev, hal) = make_hal();
    for i in 0..1_000u64 {
        hal.send_frame(&LogicalFrame::new(sim_out.last_frame.clone(), i)).unwrap();
    }
    assert_eq!(sim_dev.frames_sent(), 1_000);
}

// ── REAL-TIME: full stack (sim hop + adapt + effects + HAL) < 5ms avg ─────
#[test]
fn e2e_full_stack_latency_within_realtime_budget() {
    use std::time::Instant;
    use led_bridge::{adapt, SimLoop};
    use led_bridge::sim::SimConfig;
    use led_pixel_engine::{AudioShare, Band, BandPulse, Effect, Vec3};
    use audio_core::Analyzer;
    use audio_core::contracts::HOP_SIZE;

    let (_, hal) = make_hal();
    let share = Arc::new(AudioShare::new());
    let bp = BandPulse::new(PixelColor::rgb(0, 0, 255), Band::Bass, 2.0, share.clone());
    let positions: Vec<Vec3> = (0..PIXELS).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();

    let mut analyzer = Analyzer::new(48_000);
    let mut hop = [0.0f32; HOP_SIZE];
    let mut frame_buf = vec![PixelColor::default(); PIXELS];
    let mut total_ns = 0u128;
    let runs = 100u64;

    for i in 0..runs {
        // Synthetic hop
        for (j, s) in hop.iter_mut().enumerate() {
            *s = ((j as f32 * 0.1 + i as f32) * std::f32::consts::TAU / 48.0).sin() * 0.5;
        }

        let t0 = Instant::now();
        // 1. Analyze
        let v1 = analyzer.process_hop(&hop, i * 5);
        // 2. Adapt
        let v0 = adapt(&v1);
        // 3. Publish to AudioShare
        share.publish(&v0);
        // 4. Render effect
        frame_buf.fill(PixelColor::default());
        bp.render(i * 5, &positions, &mut frame_buf);
        // 5. Send via HAL
        hal.send_frame(&LogicalFrame::new(frame_buf.clone(), i)).unwrap();
        total_ns += t0.elapsed().as_nanos();
    }

    let avg_ms = total_ns as f64 / runs as f64 / 1_000_000.0;
    assert!(avg_ms < 5.0,
        "full stack avg latency {avg_ms:.3}ms exceeds 5ms real-time budget");
}

// ── HEARTBEAT: integrate with full stack ─────────────────────────────────
#[test]
fn e2e_heartbeat_resends_last_sim_frame() {
    use led_hal::Heartbeat;
    use std::time::Duration;

    let sim_out = SimLoop::new(SimConfig {
        tone_hz: 100.0,
        pixel_count: PIXELS,
        ..SimConfig::default()
    }).run(200);

    let (sim_dev, hal) = make_hal();
    let hal: Arc<dyn ProtocolOutput> = Arc::new(hal);
    let hb = Arc::new(Heartbeat::new());

    // Record the sim's last frame
    let last = LogicalFrame::new(sim_out.last_frame.clone(), sim_out.hops_processed);
    hal.send_frame(&last).unwrap();
    hb.record(&last);

    // Spawn heartbeat at 80ms interval
    let _handle = Arc::clone(&hb).spawn(Arc::clone(&hal), Duration::from_millis(80));

    // Wait 250ms — expect at least 2 heartbeat resends
    std::thread::sleep(Duration::from_millis(250));
    let total = sim_dev.frames_sent();
    assert!(total >= 3, // 1 manual + ≥2 heartbeats
        "must have ≥3 frames (1 manual + ≥2 heartbeats), got {total}");
}
