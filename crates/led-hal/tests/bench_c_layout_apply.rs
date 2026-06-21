use std::sync::Arc;
use std::time::Instant;
use led_hal::*;
use led_core::{DeviceDriver, LogicalFrame, PixelColor, ProtocolOutput};

#[test]
fn bench_layout_apply_scale() {
    const RUNS: usize = 200;
    let scales = [10_000usize, 50_000, 100_000];

    println!("\n=== Bench C: Layout Apply (L→P mapping) ===");
    println!("{:<12} {:<12} {:<10} {:<10} {:<10} budget", "pixels", "universes", "avg_us", "p50_us", "p95_us");

    for &n in &scales {
        let universes = ((n + 169) / 170) as u16;
        let specs = vec![DeviceSpec { id: 1, universes }];
        let layout = CompiledLayout::linear(n, &specs, RgbOrder::Rgb);
        let sim = SimulatorDevice::new(1, layout.device_universes(1));
        let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
        let hal: Arc<dyn ProtocolOutput> = Arc::new(Hal::new(layout, devices));
        let frame = LogicalFrame::new(vec![PixelColor::rgb(1, 2, 3); n], 0);

        let mut times_ns: Vec<u128> = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let t0 = Instant::now();
            hal.send_frame(&frame).unwrap();
            times_ns.push(t0.elapsed().as_nanos());
        }
        times_ns.sort_unstable();
        let avg_us = times_ns.iter().sum::<u128>() / RUNS as u128 / 1_000;
        let p50_us = times_ns[RUNS / 2] / 1_000;
        let p95_us = times_ns[RUNS * 95 / 100] / 1_000;
        let ok = if avg_us <= 1_000 { "OK <=1ms" } else { "OVER budget" };
        println!("{:<12} {:<12} {:<10} {:<10} {:<10} {}", n, universes, avg_us, p50_us, p95_us, ok);
    }
}
