//! M1 — measurement-only benchmark for the HAL send path under concurrency.
//!
//! Constitutional-review finding M1 (`[VALIDAR]`): `Hal::send_frame` takes the `scratch`
//! `Mutex` every frame, and in production the render thread AND the heartbeat thread both call
//! `send_frame` on the same `Hal` → they can contend on that lock. This bench QUANTIFIES the
//! contention: it measures the render path's per-op latency (p50/p99) **alone** vs **while a
//! second thread hammers the same `Hal`** (the heartbeat scenario). The delta is the evidence.
//!
//! This file changes NO production logic — it only exercises the existing public API. It does
//! not assert a hard performance threshold (the value is the printed measurement); it only
//! sanity-checks that the run completed. Run with `--nocapture` to read the numbers:
//!   cargo test -p led-hal --test bench_contention -- --nocapture
//!
//! Honesty of environment: measured on the machine running the suite (dev macOS here, not the
//! production cabled-Linux appliance) with a `SimulatorDevice` (no network I/O) — so the
//! numbers isolate the in-process lock path, not wire latency. Treat them as a relative
//! (alone vs contended) signal, not an absolute production figure.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use led_hal::*;

/// Nearest-rank percentile over a sorted slice of nanosecond latencies.
fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize).min(sorted.len() - 1);
    sorted[idx]
}

/// Run `n` `send_frame`s on `hal`, returning (p50, p99) per-op latency in ns.
fn measure(hal: &Arc<Hal>, frame: &LogicalFrame, n: usize) -> (u64, u64) {
    let mut lat = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        hal.send_frame(frame).unwrap();
        lat.push(t.elapsed().as_nanos() as u64);
    }
    lat.sort_unstable();
    (pct(&lat, 50.0), pct(&lat, 99.0))
}

#[test]
#[ignore = "measurement bench (non-deterministic timing, ~11s) — run manually: \
            cargo test -p led-hal --test bench_contention -- --ignored --nocapture"]
fn bench_scratch_mutex_contention() {
    let px = 300usize;
    let sim = SimulatorDevice::new(0, &[1u16, 2u16]);
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim];
    let layout = CompiledLayout::linear(px, &[DeviceSpec { id: 0, universes: 2 }], RgbOrder::Grb);
    let hal = Arc::new(Hal::new(layout, devices));
    let frame = LogicalFrame::new(vec![PixelColor::rgb(10, 20, 30); px], 0);

    let iters = 100_000usize;

    // Warm-up (flush lazy init before measuring).
    for _ in 0..2000 {
        hal.send_frame(&frame).unwrap();
    }

    // A — render path ALONE.
    let (a50, a99) = measure(&hal, &frame, iters);

    // B — render path WHILE a heartbeat-like contender hammers the same Hal.
    let stop = Arc::new(AtomicBool::new(false));
    let c_hal = hal.clone();
    let c_frame = frame.clone();
    let c_stop = stop.clone();
    let contender = thread::spawn(move || {
        while !c_stop.load(Ordering::Relaxed) {
            let _ = c_hal.send_frame(&c_frame);
        }
    });
    let (b50, b99) = measure(&hal, &frame, iters);
    stop.store(true, Ordering::Relaxed);
    contender.join().unwrap();

    let f50 = b50 as f64 / a50.max(1) as f64;
    let f99 = b99 as f64 / a99.max(1) as f64;
    eprintln!("── M1 · HAL send_frame contention ({iters} iters, {px}px, SimulatorDevice) ──");
    eprintln!("  render ALONE       : p50={a50:>7} ns   p99={a99:>7} ns");
    eprintln!("  render + contender : p50={b50:>7} ns   p99={b99:>7} ns");
    eprintln!("  contention factor  : p50 ×{f50:.2}      p99 ×{f99:.2}");
    eprintln!("  (factor ≈1.0 = negligible; ≫1.0 = the lock adds real jitter to the render path)");

    // Sanity only — the deliverable is the printed measurement, not a threshold.
    assert!(a99 > 0 && b99 > 0, "measurement produced non-zero latencies");
}
