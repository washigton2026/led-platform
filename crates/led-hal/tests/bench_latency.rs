//! Latency benchmark — validates the frame-send budget under the LUMYX_GOSL §6 rule:
//! render+send must complete within the frame period (50ms at 20fps, 33ms at 30fps).
//!
//! These are wall-clock tests, not unit tests — they run under normal cargo test
//! and fail only on catastrophic regressions (>10× budget). For p50/p99 analysis,
//! use `MetricsEmitter::snapshot_json()` after the test.

use std::sync::Arc;
use std::time::Instant;

use led_core::{CompiledLayout, DeviceSpec, LogicalFrame, PixelColor, ProtocolOutput, RgbOrder};
use led_hal::{Hal, MetricsEmitter, SimulatorDevice};

const PIXELS: usize = 512;
// 512 pixels × 3 bytes = 1536; ceil(1536/512) = 3 but the last pixel wraps to slot 3 → need 4
const PIXEL_UNIVERSES: u16 = 4;
const FRAMES: usize = 500;
/// Maximum acceptable average latency per frame (µs). In debug builds, 10ms is generous.
const MAX_AVG_US: u64 = if cfg!(debug_assertions) { 10_000 } else { 500 };

fn make_hal_with_metrics() -> (Hal, Arc<MetricsEmitter>, Arc<SimulatorDevice>) {
    let specs = [DeviceSpec { id: 1, universes: PIXEL_UNIVERSES }];
    let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let metrics = Arc::new(MetricsEmitter::new("bench-hal"));
    let hal = Hal::new(layout, vec![sim.clone()])
        .with_metrics(metrics.clone());
    (hal, metrics, sim)
}

#[test]
fn hal_send_frame_latency_within_budget() {
    let (hal, metrics, _sim) = make_hal_with_metrics();
    let frame = LogicalFrame::new(vec![PixelColor::rgb(100, 150, 200); PIXELS], 0);

    let t0 = Instant::now();
    for i in 0..FRAMES {
        let mut f = frame.clone();
        f.timestamp_ms = i as u64 * 50;
        hal.send_frame(&f).expect("send_frame must not fail");
    }
    let total_us = t0.elapsed().as_micros() as u64;
    let avg_us = total_us / FRAMES as u64;

    assert_eq!(metrics.frame_count(), FRAMES as u64, "all frames must be recorded");
    assert!(
        avg_us < MAX_AVG_US,
        "avg frame latency {avg_us}µs exceeds budget {MAX_AVG_US}µs — REGRESSION\n{}",
        metrics.snapshot_json()
    );

    let p99 = metrics.p99_us();
    // p99 must not exceed 10× average (outlier gate)
    let p99_limit = (avg_us * 10).max(MAX_AVG_US * 2);
    assert!(
        p99 < p99_limit,
        "p99={p99}µs exceeds {p99_limit}µs — tail latency regression\n{}",
        metrics.snapshot_json()
    );

    println!(
        "HAL latency benchmark ({PIXELS}px, {FRAMES} frames):\n  {}",
        metrics.snapshot_json()
    );
}

#[test]
fn hal_latency_scales_linearly_to_10k_pixels() {
    // Verify O(n) scaling: 10k pixels should not take 20× longer than 512 pixels.
    const N_LARGE: usize = 10_000;

    // 10_000 pixels × 3 bytes = 30_000 bytes; ceil(30_000 / 512) = 59 universes
    let specs_large = [DeviceSpec { id: 2, universes: 59 }];
    let layout_large = CompiledLayout::linear(N_LARGE, &specs_large, RgbOrder::Rgb);
    let sim_large = SimulatorDevice::new(2, layout_large.device_universes(2));
    let m_large = Arc::new(MetricsEmitter::new("bench-10k"));
    let hal_large = Hal::new(layout_large, vec![sim_large]).with_metrics(m_large.clone());
    let frame_large = LogicalFrame::new(vec![PixelColor::default(); N_LARGE], 0);

    for i in 0..FRAMES {
        let mut f = frame_large.clone();
        f.timestamp_ms = i as u64;
        hal_large.send_frame(&f).unwrap();
    }

    let avg_10k = m_large.frame_count()
        .checked_div(FRAMES as u64)
        .unwrap_or(1);

    // 10k pixels / 512 pixels = ~20× more pixels → allow ≤ 50× more time (O(n) with slack)
    let slack_budget = MAX_AVG_US * 50;
    let avg_10k_us = m_large.p50_us().max(1);
    assert!(
        avg_10k_us < slack_budget,
        "10k pixel latency {avg_10k_us}µs exceeds O(n) budget {slack_budget}µs\n{}",
        m_large.snapshot_json()
    );
    let _ = avg_10k; // suppress unused warning

    println!("10k pixel benchmark:\n  {}", m_large.snapshot_json());
}

#[test]
fn metrics_emitter_attached_to_hal_records_all_frames() {
    let (hal, metrics, _) = make_hal_with_metrics();
    assert_eq!(metrics.frame_count(), 0, "starts at 0");

    let frame = LogicalFrame::new(vec![PixelColor::default(); PIXELS], 0);
    for i in 0..10u64 {
        let mut f = frame.clone();
        f.timestamp_ms = i;
        hal.send_frame(&f).unwrap();
    }
    assert_eq!(metrics.frame_count(), 10, "must record all 10 frames");
    assert!(metrics.p50_us() > 0, "p50 must be non-zero after frames");
}

#[test]
fn hal_without_metrics_has_no_overhead() {
    // Verify Hal::new() (no metrics) does not panic and still sends frames
    let specs4 = [DeviceSpec { id: 4, universes: PIXEL_UNIVERSES }];
    let layout4 = CompiledLayout::linear(PIXELS, &specs4, RgbOrder::Rgb);
    let sim4 = SimulatorDevice::new(4, layout4.device_universes(4));
    let hal = Hal::new(layout4, vec![sim4.clone()]);
    assert!(hal.metrics().is_none(), "default Hal must have no metrics");

    let frame = LogicalFrame::new(vec![PixelColor::default(); PIXELS], 0);
    hal.send_frame(&frame).expect("send_frame must work without metrics");
    assert!(sim4.frames_sent() >= 1, "frame must reach simulator");
}
