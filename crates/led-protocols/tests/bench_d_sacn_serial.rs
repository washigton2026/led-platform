/// Bench D: led-protocols sACN packet serialization at scale.
/// Measures build_data_packet() throughput for N universes (equiv to pixel counts).
/// 10k px = 59 universes, 50k px = 295 universes, 100k px = 589 universes.

use std::time::Instant;
use led_protocols::packet::{self, PACKET_LEN};

#[test]
fn bench_sacn_serialization() {
    const RUNS: usize = 100;
    let cid = [0u8; 16];
    let dmx_data = [128u8; 512]; // full universe, mid-value

    let scales: &[(usize, usize)] = &[
        (10_000,  59),
        (50_000,  295),
        (100_000, 589),
    ];

    println!("\n=== Bench D: sACN Serialization (build_data_packet) ===");
    println!("{:<12} {:<12} {:<14} {:<14} budget",
        "pixels", "universes", "avg_total_us", "per_univ_us");

    for &(pixels, universes) in scales {
        let mut bufs: Vec<[u8; PACKET_LEN]> = vec![[0u8; PACKET_LEN]; universes];
        let mut times_ns: Vec<u128> = Vec::with_capacity(RUNS);

        for run in 0..RUNS {
            let t0 = Instant::now();
            for (u, buf) in bufs.iter_mut().enumerate() {
                packet::build_data_packet(
                    buf,
                    &cid,
                    "LUMYX-bench",
                    100u8,                         // priority
                    (run * universes + u) as u8,   // sequence
                    (u + 1) as u16,                // universe 1-based
                    &dmx_data,
                );
            }
            times_ns.push(t0.elapsed().as_nanos());
        }

        times_ns.sort_unstable();
        let avg_total_us = times_ns.iter().sum::<u128>() / RUNS as u128 / 1_000;
        let per_univ_us  = avg_total_us / universes as u128;
        let ok = if avg_total_us <= 1_000 { "OK <=1ms" } else { "OVER budget" };
        println!("{:<12} {:<12} {:<14} {:<14} {}",
            pixels, universes, avg_total_us, per_univ_us, ok);
    }

    println!("\n  Budget: serialization ≤1ms for full frame (all universes)");
    println!("  Note: measures CPU serialization only — no network I/O");
}
