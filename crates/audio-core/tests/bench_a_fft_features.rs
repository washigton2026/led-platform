/// Bench A: audio-core FFT + feature extraction throughput.
/// Measures hop latency (the real hot-path) and analyze_all throughput
/// at 1s, 5s, 10s of audio. Pixel-count independent.

use audio_core::{Analyzer, MockCaptureSource};
use audio_core::contracts::HOP_SIZE;
use std::f32::consts::TAU;
use std::time::Instant;

fn sine_samples(hz: f32, sr: u32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (TAU * hz * i as f32 / sr as f32).sin() * 0.5).collect()
}

#[test]
fn bench_audio_core_throughput() {
    let sr = 48_000u32;

    println!("\n=== Bench A: audio-core FFT + features ===");
    println!("  (pixel-count independent — measures per-hop pipeline)");

    // ── Single hop isolation: the true hot-path latency ──────────────────────
    let mut analyzer = Analyzer::new(sr);
    let hop = [0.1f32; HOP_SIZE];
    // Warmup
    for i in 0..10u64 { analyzer.process_hop(&hop, i * 5); }

    let single_runs = 1_000u64;
    let mut single_times_ns: Vec<u128> = Vec::with_capacity(single_runs as usize);
    for i in 0..single_runs {
        let t0 = Instant::now();
        let _ = analyzer.process_hop(&hop, i * 5);
        single_times_ns.push(t0.elapsed().as_nanos());
    }
    single_times_ns.sort_unstable();
    let single_avg_us = single_times_ns.iter().sum::<u128>() / single_runs as u128 / 1_000;
    let single_p50_us = single_times_ns[single_runs as usize / 2] / 1_000;
    let single_p99_us = single_times_ns[single_runs as usize * 99 / 100] / 1_000;
    let budget_5ms = if single_avg_us <= 5_000 { "OK" } else { "OVER" };

    println!("\n  Single hop (process_hop, 1000 runs):");
    println!("    avg={single_avg_us}us  p50={single_p50_us}us  p99={single_p99_us}us  ≤5ms: {budget_5ms}");

    // ── analyze_all throughput at 1s, 5s, 10s ────────────────────────────────
    println!("\n  analyze_all throughput:");
    println!("  {:<10} {:<10} {:<12} {:<14} {:<10}",
        "duration", "hops", "total_ms", "avg_ms/hop", "hops/sec");

    for &secs in &[1u32, 5, 10] {
        let n = sr as usize * secs as usize;
        let samples = sine_samples(440.0, sr, n);
        let mock = MockCaptureSource::new(sr, samples);

        let t0 = Instant::now();
        let results = mock.analyze_all();
        let elapsed_ms = t0.elapsed().as_millis();

        let hops = results.len();
        let avg_ms_per_hop = if hops > 0 { elapsed_ms as f64 / hops as f64 } else { 0.0 };
        let hops_per_sec = if elapsed_ms > 0 { hops as f64 * 1000.0 / elapsed_ms as f64 } else { 0.0 };

        println!("  {:<10} {:<10} {:<12} {:<14.3} {:<10.0}",
            format!("{}s", secs), hops, elapsed_ms, avg_ms_per_hop, hops_per_sec);

        // Sanity check: all hops must have correct sample_rate
        for f in &results {
            assert_eq!(f.sample_rate, sr, "sample_rate must propagate");
        }
    }

    println!("\n  Budget note: ≤5ms per hop for real-time constraint (48kHz, 256-sample hop = 5.3ms real-time)");
}
