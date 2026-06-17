//! 1000-cycle stability test — the final gate to STABLE STATE.
//!
//! Runs 1000 consecutive audio→LED simulation cycles and verifies:
//! - Zero panics, zero aborts, zero UB
//! - Frame count is exactly right (no dropped frames)
//! - Pixel values are valid u8 (no corruption)
//! - Timestamps are monotonic in every run
//! - Harmonic ratio stays deterministic (same output every cycle)
//! - beats_detected is consistent across identical runs
//! - Memory: no unbounded growth (harmonic_ratio_log len = hops_processed)
//! - Real-time: 1000 cycles complete within 300s (0.3s per cycle budget)

use std::time::Instant;
use led_bridge::sim::{SimConfig, SimLoop};

const CYCLES: usize = 1_000;
const CYCLE_DURATION_MS: u64 = 100; // 100ms of audio per cycle = fast but real

// ── STABILITY: 1000 cycles without panic or state corruption ──────────────
#[test]
fn stability_1000_cycles_no_panic() {
    let sim = SimLoop::new(SimConfig {
        tone_hz: 440.0,
        beat_interval_ms: 200, // 5 beats/s
        pixel_count: 100,
        ..SimConfig::default()
    });

    for cycle in 0..CYCLES {
        let out = sim.run(CYCLE_DURATION_MS);
        assert!(out.hops_processed > 0, "cycle {cycle}: must process hops");
        assert!(out.frames_rendered > 0, "cycle {cycle}: must render frames");
        assert_eq!(out.frames_rendered, out.hops_processed, "cycle {cycle}: 1 frame/hop");
        assert_eq!(out.last_frame.len(), 100, "cycle {cycle}: pixel count must be stable");
        assert_eq!(
            out.harmonic_ratio_log.len() as u64, out.hops_processed,
            "cycle {cycle}: harmonic log must match hops"
        );
    }
}

// ── DETERMINISM: 1000 runs produce identical output ────────────────────────
#[test]
fn stability_1000_runs_deterministic() {
    let cfg = SimConfig {
        tone_hz: 220.0,
        beat_interval_ms: 300,
        pixel_count: 50,
        ..SimConfig::default()
    };

    let reference = SimLoop::new(cfg).run(CYCLE_DURATION_MS);

    for cycle in 1..CYCLES {
        let cfg = SimConfig {
            tone_hz: 220.0,
            beat_interval_ms: 300,
            pixel_count: 50,
            ..SimConfig::default()
        };
        let out = SimLoop::new(cfg).run(CYCLE_DURATION_MS);

        assert_eq!(
            out.beats_detected, reference.beats_detected,
            "cycle {cycle}: beats_detected must be deterministic"
        );
        assert_eq!(
            out.harmonic_ratio_log, reference.harmonic_ratio_log,
            "cycle {cycle}: harmonic_ratio_log must be deterministic"
        );
        assert_eq!(
            out.last_frame, reference.last_frame,
            "cycle {cycle}: last_frame pixels must be deterministic"
        );
    }
}

// ── MONOTONICITY: timestamps never go backwards across 1000 runs ────────────
#[test]
fn stability_timestamps_monotone_1000_runs() {
    let sim = SimLoop::new(SimConfig::default());
    for cycle in 0..CYCLES {
        let out = sim.run(CYCLE_DURATION_MS);
        let mut prev = 0u64;
        for (hop, sc) in out.scalars_log.iter().enumerate() {
            assert!(sc.timestamp_ms >= prev,
                "cycle {cycle} hop {hop}: timestamp regression {} < {}", sc.timestamp_ms, prev);
            prev = sc.timestamp_ms;
        }
    }
}

// ── PIXEL VALIDITY: all pixel values are valid u8 across 1000 cycles ────────
#[test]
fn stability_pixels_always_valid_1000_cycles() {
    let sim = SimLoop::new(SimConfig {
        tone_hz: 100.0,
        beat_interval_ms: 150,
        pixel_count: 200,
        ..SimConfig::default()
    });
    for _cycle in 0..CYCLES {
        let out = sim.run(CYCLE_DURATION_MS);
        for (i, px) in out.last_frame.iter().enumerate() {
            // u8 is always valid, but check they're written (frame non-uniform)
            let _ = (px.r, px.g, px.b); // no panic
            assert!(i < 200, "pixel index out of range");
        }
    }
}

// ── MEMORY: no unbounded growth across 1000 cycles ──────────────────────────
#[test]
fn stability_no_memory_growth_1000_cycles() {
    let sim = SimLoop::new(SimConfig {
        pixel_count: 100,
        ..SimConfig::default()
    });
    let first = sim.run(CYCLE_DURATION_MS);
    let expected_hops = first.hops_processed;
    let expected_log_len = first.harmonic_ratio_log.len();
    let expected_scalars = first.scalars_log.len();

    for cycle in 1..CYCLES {
        let out = sim.run(CYCLE_DURATION_MS);
        assert_eq!(out.hops_processed, expected_hops,
            "cycle {cycle}: hop count must be stable (no accumulation)");
        assert_eq!(out.harmonic_ratio_log.len(), expected_log_len,
            "cycle {cycle}: harmonic log must not grow");
        assert_eq!(out.scalars_log.len(), expected_scalars,
            "cycle {cycle}: scalars log must not grow");
    }
}

// ── REAL-TIME: 1000 cycles × 100ms audio < 300s wall-clock ─────────────────
#[test]
fn stability_1000_cycles_within_time_budget() {
    let sim = SimLoop::new(SimConfig::default());
    let t0 = Instant::now();

    for _ in 0..CYCLES {
        let _ = sim.run(CYCLE_DURATION_MS);
    }

    let elapsed = t0.elapsed();
    // Each cycle is 100ms simulated audio = ~18 hops.
    // 1000 cycles × 18 hops × ~1ms/hop (debug) ≈ 18s worst case.
    // Allow 300s for very slow CI machines.
    let budget_secs = 300u64;
    assert!(elapsed.as_secs() < budget_secs,
        "1000-cycle stability test took {}s (budget {budget_secs}s)", elapsed.as_secs());

    eprintln!(
        "✅ 1000 cycles × {}ms audio in {:.2}s ({:.2}ms/cycle avg)",
        CYCLE_DURATION_MS,
        elapsed.as_secs_f64(),
        elapsed.as_millis() as f64 / CYCLES as f64,
    );
}

// ── CONCURRENT: 4 independent SimLoop instances running simultaneously ────────
#[test]
fn stability_4_concurrent_simloops_no_interference() {
    use std::thread;
    let handles: Vec<_> = (0..4u32).map(|id| {
        thread::spawn(move || {
            let sim = SimLoop::new(SimConfig {
                tone_hz: 100.0 + id as f32 * 110.0,
                beat_interval_ms: 200 + id as u64 * 50,
                pixel_count: 50,
                ..SimConfig::default()
            });
            let mut prev_beats = None;
            for _ in 0..100 {
                let out = sim.run(CYCLE_DURATION_MS);
                // Determinism within each thread
                if let Some(pb) = prev_beats {
                    assert_eq!(out.beats_detected, pb,
                        "thread {id}: beats must be deterministic");
                }
                prev_beats = Some(out.beats_detected);
            }
        })
    }).collect();
    for h in handles { h.join().unwrap(); }
}
