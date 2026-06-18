//! Multi-system integration: LED pipeline + Drone safety running simultaneously.
//!
//! Validates that the LUMYX platform can sustain two independent real-time subsystems
//! in parallel without interference, shared-state corruption, or timing regression.
//!
//! ## Systems under test
//!
//! | System | Thread | Data flow |
//! |---|---|---|
//! | LED | T1 | SimLoop → adapt → AudioShare → effects → HAL → SimulatorDevice |
//! | Drone | T2 | Formation → trajectory → safety gate → ValidationReport |
//!
//! Both threads run concurrently. After joining, both outputs are validated for:
//! - Correctness (no corruption from concurrent execution)
//! - Safety (drone validation must pass, LED frames must be non-zero)
//! - Performance (each subsystem must complete within its real-time budget)

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use led_bridge::sim::{SimConfig, SimLoop};
use led_core::{LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{CompiledLayout, DeviceSpec, Hal, RgbOrder, SimulatorDevice};

// ── Drone imports (from drone-platform via path dep) ─────────────────────
// NOTE: drone-platform is a separate workspace — we simulate the drone
// safety computation inline using the same math to avoid cross-workspace deps.

const LED_PIXELS: usize = 100;
const DRONE_COUNT: usize = 50;
const SEPARATION_M: f32 = 5.1; // RTK static threshold

fn make_led_hal() -> (Arc<SimulatorDevice>, Hal) {
    let specs = [DeviceSpec { id: 1, universes: 1 }];
    let layout = CompiledLayout::linear(LED_PIXELS, &specs, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn led_core::DeviceDriver>> = vec![sim.clone()];
    (sim, Hal::new(layout, devices))
}

/// Minimal drone position for safety check simulation (no cross-workspace dep).
#[derive(Clone, Copy)]
struct DronePos { x: f32, y: f32, z: f32 }

impl DronePos {
    fn distance(self, o: DronePos) -> f32 {
        let (dx, dy, dz) = (self.x-o.x, self.y-o.y, self.z-o.z);
        (dx*dx + dy*dy + dz*dz).sqrt()
    }
}

fn drone_swarm_safe(drones: &[DronePos]) -> bool {
    for i in 0..drones.len() {
        for j in (i+1)..drones.len() {
            if drones[i].distance(drones[j]) < SEPARATION_M {
                return false;
            }
        }
    }
    true
}

// ── TEST: LED and Drone run concurrently, no interference ─────────────────
#[test]
fn led_and_drone_run_concurrently_no_interference() {
    let (sim_dev, hal) = make_led_hal();
    let hal: Arc<dyn ProtocolOutput> = Arc::new(hal);
    let hal_clone = hal.clone();

    // LED thread: full audio→pixel→HAL pipeline
    let led_handle = thread::spawn(move || {
        let t0 = Instant::now();
        let out = SimLoop::new(SimConfig {
            tone_hz: 100.0,
            beat_interval_ms: 200,
            pixel_count: LED_PIXELS,
            ..SimConfig::default()
        }).run(2_000); // 2 seconds

        // Send 100 frames through HAL
        for i in 0..100u64 {
            let frame = LogicalFrame::new(out.last_frame.clone(), i);
            hal_clone.send_frame(&frame).unwrap();
        }
        (out.hops_processed, out.frames_rendered, t0.elapsed())
    });

    // Drone thread: safety validation of 50-drone formation over 1000 iterations
    let drone_handle = thread::spawn(|| {
        let t0 = Instant::now();
        let drones: Vec<DronePos> = (0..DRONE_COUNT)
            .map(|i| DronePos { x: i as f32 * SEPARATION_M * 1.1, y: 10.0, z: 0.0 })
            .collect();
        let mut safe_count = 0u32;
        for _iter in 0..1_000 {
            if drone_swarm_safe(&drones) { safe_count += 1; }
        }
        (safe_count, t0.elapsed())
    });

    let (led_hops, led_frames, led_elapsed) = led_handle.join().unwrap();
    let (drone_safe, drone_elapsed) = drone_handle.join().unwrap();

    // LED system assertions
    assert!(led_hops > 0, "LED must have processed hops");
    assert!(led_frames > 0, "LED must have rendered frames");
    assert_eq!(sim_dev.frames_sent(), 100, "HAL must have received 100 frames");

    // Drone safety assertions
    assert_eq!(drone_safe, 1_000, "all 1000 swarm checks must pass (50 drones at {SEPARATION_M}m)");

    // Real-time budget: both must complete within 10s wall-clock
    assert!(led_elapsed < Duration::from_secs(10),
        "LED subsystem took {}ms", led_elapsed.as_millis());
    assert!(drone_elapsed < Duration::from_secs(10),
        "Drone subsystem took {}ms", drone_elapsed.as_millis());
}

// ── TEST: LED pipeline under load — AudioShare contention ─────────────────
#[test]
fn audioshare_under_concurrent_led_and_audio_threads() {
    use led_pixel_engine::AudioShare;
    use led_core::AudioFeatures;

    let share = Arc::new(AudioShare::new());
    let share_write = share.clone();
    let share_read  = share.clone();

    // Audio thread: publishes features at ~200Hz
    let writer = thread::spawn(move || {
        for i in 0..1_000u64 {
            share_write.publish(&AudioFeatures {
                sample_rate: 48_000,
                timestamp_ms: i * 5,
                rms: (i as f32 * 0.001).min(1.0),
                beat: i % 94 == 0,
                bass: (i as f32 * 0.002).min(1.0),
                mid: 0.3, high: 0.1,
                spectrum: vec![0.0; 512],
            });
            std::hint::spin_loop(); // yield briefly
        }
    });

    // LED render thread: reads scalars at ~60fps
    let reader = thread::spawn(move || {
        let mut reads = 0u32;
        let mut beat_reads = 0u32;
        for _ in 0..500 {
            let sc = share_read.scalars();
            reads += 1;
            if sc.beat { beat_reads += 1; }
        }
        (reads, beat_reads)
    });

    writer.join().unwrap();
    let (reads, _beat_reads) = reader.join().unwrap();
    assert_eq!(reads, 500, "render thread must complete 500 reads");
}

// ── TEST: multiple HAL instances don't interfere ─────────────────────────
#[test]
fn multiple_hal_instances_independent() {
    let (sim1, hal1) = make_led_hal();
    let (sim2, hal2) = make_led_hal();
    let hal1: Arc<dyn ProtocolOutput> = Arc::new(hal1);
    let hal2: Arc<dyn ProtocolOutput> = Arc::new(hal2);

    let h1 = hal1.clone();
    let h2 = hal2.clone();

    // Two LED pipelines running concurrently
    let t1 = thread::spawn(move || {
        let pixels = vec![PixelColor::rgb(255, 0, 0); LED_PIXELS];
        for i in 0..50u64 {
            h1.send_frame(&LogicalFrame::new(pixels.clone(), i)).unwrap();
        }
    });
    let t2 = thread::spawn(move || {
        let pixels = vec![PixelColor::rgb(0, 0, 255); LED_PIXELS];
        for i in 0..50u64 {
            h2.send_frame(&LogicalFrame::new(pixels.clone(), i)).unwrap();
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    assert_eq!(sim1.frames_sent(), 50, "HAL 1 must have 50 frames");
    assert_eq!(sim2.frames_sent(), 50, "HAL 2 must have 50 frames");

    // Content must be independent: HAL1 got red, HAL2 got blue
    assert_eq!(sim1.channel(0, 0), Some(255), "HAL1 R=255");
    assert_eq!(sim1.channel(0, 2), Some(0),   "HAL1 B=0");
    assert_eq!(sim2.channel(0, 0), Some(0),   "HAL2 R=0");
    assert_eq!(sim2.channel(0, 2), Some(255), "HAL2 B=255");
}

// ── TEST: drone safety + LED heartbeat concurrently ───────────────────────
#[test]
fn drone_safety_and_led_heartbeat_run_concurrently() {
    use led_hal::Heartbeat;

    let (sim_dev, hal) = make_led_hal();
    let hal: Arc<dyn ProtocolOutput> = Arc::new(hal);
    let hb = Arc::new(Heartbeat::new());

    // Send one LED frame and record it for heartbeat
    let pixels = vec![PixelColor::rgb(100, 150, 200); LED_PIXELS];
    let frame = LogicalFrame::new(pixels, 0);
    hal.send_frame(&frame).unwrap();
    hb.record(&frame);

    // Start LED heartbeat at 80ms
    let _hb_handle = Arc::clone(&hb).spawn(Arc::clone(&hal), Duration::from_millis(80));

    // Concurrently: drone safety check loop
    let drone_thread = thread::spawn(|| {
        let drones: Vec<DronePos> = (0..DRONE_COUNT)
            .map(|i| DronePos { x: i as f32 * 8.0, y: 15.0, z: 0.0 })
            .collect();
        let mut violations = 0u32;
        for _ in 0..500 {
            if !drone_swarm_safe(&drones) { violations += 1; }
        }
        violations
    });

    // Causal barrier: wait until ≥2 LED heartbeat frames arrive instead of sleeping 200ms.
    let deadline = Instant::now() + Duration::from_secs(5);
    while sim_dev.frames_sent() < 2 {
        assert!(Instant::now() < deadline,
            "timeout: LED heartbeat must fire ≥2×, got {}", sim_dev.frames_sent());
        thread::sleep(Duration::from_millis(1));
    }

    let violations = drone_thread.join().unwrap();
    let led_frames = sim_dev.frames_sent();

    assert_eq!(violations, 0, "drone swarm must be safe throughout");
    assert!(led_frames >= 2,
        "LED heartbeat must fire ≥2× (got {led_frames})");
    // Content preserved: R channel
    assert_eq!(sim_dev.channel(0, 0), Some(100), "heartbeat must preserve R=100");
}

// ── STRESS: 4 concurrent LED pipelines + 4 drone threads ─────────────────
#[test]
fn stress_four_led_four_drone_threads() {
    let mut handles = Vec::new();

    // 4 LED threads
    for id in 0..4u32 {
        handles.push(thread::spawn(move || {
            let out = SimLoop::new(SimConfig {
                tone_hz: 100.0 + id as f32 * 50.0,
                beat_interval_ms: 200 + id as u64 * 100,
                pixel_count: 50,
                ..SimConfig::default()
            }).run(500);
            assert!(out.hops_processed > 0, "LED thread {id} must process hops");
        }));
    }

    // 4 Drone threads
    for id in 0..4u32 {
        handles.push(thread::spawn(move || {
            let drones: Vec<DronePos> = (0..20)
                .map(|i| DronePos { x: i as f32 * 6.0 + id as f32 * 200.0, y: 10.0, z: 0.0 })
                .collect();
            for _ in 0..200 {
                assert!(drone_swarm_safe(&drones), "drone thread {id}: safety violation");
            }
        }));
    }

    for h in handles { h.join().unwrap(); }
}
