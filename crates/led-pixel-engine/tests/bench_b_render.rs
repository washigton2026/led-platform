/// Bench B: led-pixel-engine render O(pixels) per frame.
/// Measures SolidColor and BandPulse render at 10k, 50k, 100k pixels.

use std::sync::Arc;
use std::time::Instant;
use led_pixel_engine::{AudioShare, Band, BandPulse, Effect, SolidColor, Vec3};
use led_core::{AudioFeatures, PixelColor};

fn dummy_features() -> AudioFeatures {
    AudioFeatures {
        sample_rate: 48_000,
        timestamp_ms: 1,
        rms: 0.5,
        beat: false,
        bass: 0.8,
        mid: 0.3,
        high: 0.1,
        spectrum: vec![0.0; 16],
    }
}

#[test]
fn bench_render_scale() {
    const RUNS: usize = 500;
    let scales = [10_000usize, 50_000, 100_000];

    println!("\n=== Bench B: led-pixel-engine render ===");
    println!("{:<12} {:<14} {:<10} {:<10} {:<10} budget",
        "pixels", "effect", "avg_us", "p50_us", "p95_us");

    for &n in &scales {
        let positions: Vec<Vec3> = (0..n).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        let mut out = vec![PixelColor::default(); n];

        // ── SolidColor (simplest, O(N) fill) ──────────────────────────────
        let solid = SolidColor(PixelColor::rgb(255, 0, 0));
        let mut times_ns: Vec<u128> = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let t0 = Instant::now();
            solid.render(0, &positions, &mut out);
            times_ns.push(t0.elapsed().as_nanos());
        }
        times_ns.sort_unstable();
        let avg_us = times_ns.iter().sum::<u128>() / RUNS as u128 / 1_000;
        let p50_us = times_ns[RUNS / 2] / 1_000;
        let p95_us = times_ns[RUNS * 95 / 100] / 1_000;
        let ok = if avg_us <= 5_000 { "OK <=5ms" } else { "OVER budget" };
        println!("{:<12} {:<14} {:<10} {:<10} {:<10} {}", n, "SolidColor", avg_us, p50_us, p95_us, ok);

        // ── BandPulse (audio-reactive, reads AudioShare) ──────────────────
        let share = Arc::new(AudioShare::new());
        share.publish(&dummy_features());
        let bp = BandPulse::new(PixelColor::rgb(0, 0, 255), Band::Bass, 2.0, share.clone());
        let mut times_bp: Vec<u128> = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let t0 = Instant::now();
            bp.render(0, &positions, &mut out);
            times_bp.push(t0.elapsed().as_nanos());
        }
        times_bp.sort_unstable();
        let avg_bp = times_bp.iter().sum::<u128>() / RUNS as u128 / 1_000;
        let p50_bp = times_bp[RUNS / 2] / 1_000;
        let p95_bp = times_bp[RUNS * 95 / 100] / 1_000;
        let ok_bp = if avg_bp <= 5_000 { "OK <=5ms" } else { "OVER budget" };
        println!("{:<12} {:<14} {:<10} {:<10} {:<10} {}", n, "BandPulse", avg_bp, p50_bp, p95_bp, ok_bp);
    }

    println!("\n  Budget: render ≤5ms per frame (part of total ≤5ms pipeline)");
}
