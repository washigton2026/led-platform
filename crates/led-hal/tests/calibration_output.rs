//! Calibração por-output no HAL (ADR-0019), provada nos **bytes que chegam ao device**.

use std::sync::Arc;

use led_hal::*;

const DEV: u16 = 0;
/// `CompiledLayout::linear` numera os universos globalmente a partir de 0 — o primeiro
/// device fica com o universo 0. O simulador é construído a partir do próprio layout
/// (`device_universes`) para que o espelho case com o que o HAL envia.
const UNI: u16 = 0;
const PIXELS: usize = 4;

fn rig() -> (CompiledLayout, Arc<SimulatorDevice>) {
    let layout = CompiledLayout::linear(PIXELS, &[DeviceSpec { id: DEV, universes: 1 }], RgbOrder::Rgb);
    let sim = SimulatorDevice::new(DEV, layout.device_universes(DEV));
    (layout, sim)
}

fn frame(n: usize, c: PixelColor) -> LogicalFrame {
    LogicalFrame::new(vec![c; n], 0)
}

#[test]
fn without_calibration_the_bytes_are_unchanged() {
    let (layout, sim) = rig();
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]);
    hal.send_frame(&frame(PIXELS, PixelColor::rgb(100, 150, 200))).unwrap();
    assert_eq!(sim.channel(UNI, 0), Some(100));
    assert_eq!(sim.channel(UNI, 1), Some(150));
    assert_eq!(sim.channel(UNI, 2), Some(200));
}

#[test]
fn an_identity_calibration_does_not_change_the_bytes() {
    // Provar que a mera PRESENÇA da calibração não altera a saída.
    let (layout, sim) = rig();
    let mut cal = Calibration::new();
    cal.set(DEV, 1.0, 1.0); // gamma 1, brightness 1 = identidade
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]).with_calibration(cal);
    hal.send_frame(&frame(PIXELS, PixelColor::rgb(100, 150, 200))).unwrap();
    assert_eq!(sim.channel(UNI, 0), Some(100));
    assert_eq!(sim.channel(UNI, 1), Some(150));
    assert_eq!(sim.channel(UNI, 2), Some(200));
}

#[test]
fn brightness_reaches_the_device_bytes() {
    let (layout, sim) = rig();
    let mut cal = Calibration::new();
    cal.set(DEV, 1.0, 0.5);
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]).with_calibration(cal);
    hal.send_frame(&frame(PIXELS, PixelColor::rgb(200, 100, 0))).unwrap();
    assert_eq!(sim.channel(UNI, 0), Some(100), "200 * 0.5");
    assert_eq!(sim.channel(UNI, 1), Some(50), "100 * 0.5");
    assert_eq!(sim.channel(UNI, 2), Some(0));
}

#[test]
fn gamma_reaches_the_device_bytes() {
    let (layout, sim) = rig();
    let mut cal = Calibration::new();
    cal.set(DEV, 2.2, 1.0);
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]).with_calibration(cal);
    hal.send_frame(&frame(PIXELS, PixelColor::rgb(128, 255, 0))).unwrap();
    let mid = sim.channel(UNI, 0).unwrap();
    assert!(mid < 128 && mid > 0, "gamma 2.2 escurece o meio-tom, sem apagar: {mid}");
    assert_eq!(sim.channel(UNI, 1), Some(255), "branco continua branco");
    assert_eq!(sim.channel(UNI, 2), Some(0), "preto continua preto");
}

/// A razão do buffer separado: um frame **mais curto** que o layout deixa alvos não cobertos.
/// Corrigir o scratch no lugar os re-corrigiria a cada frame, escurecendo-os cumulativamente.
#[test]
fn calibration_does_not_compound_across_frames_on_a_short_frame() {
    let (layout, sim) = rig();
    let mut cal = Calibration::new();
    cal.set(DEV, 2.2, 1.0); // gamma agressivo: o efeito cumulativo apareceria rápido
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]).with_calibration(cal);

    // Frame cobrindo TODOS os pixels — estabelece o valor de referência.
    hal.send_frame(&frame(PIXELS, PixelColor::rgb(200, 200, 200))).unwrap();
    let first = sim.channel(UNI, 0).unwrap();

    // Agora 50 frames CURTOS (1 pixel): os alvos 1..3 ficam sem cobertura.
    for _ in 0..50 {
        hal.send_frame(&frame(1, PixelColor::rgb(200, 200, 200))).unwrap();
    }

    // O pixel 0 (sempre coberto) tem o mesmo valor — idempotente por frame.
    assert_eq!(sim.channel(UNI, 0), Some(first), "correção não pode acumular no pixel coberto");
    // E os não cobertos mantêm o valor da última cobertura, não uma rampa escurecendo.
    assert_eq!(
        sim.channel(UNI, 3),
        Some(first),
        "alvo não coberto mantém o último valor válido — sem escurecimento cumulativo"
    );
}

#[test]
fn repeated_frames_are_stable() {
    let (layout, sim) = rig();
    let mut cal = Calibration::new();
    cal.set(DEV, 2.2, 0.6);
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]).with_calibration(cal);
    let f = frame(PIXELS, PixelColor::rgb(180, 90, 30));
    hal.send_frame(&f).unwrap();
    let snapshot: Vec<Option<u8>> = (0..9).map(|c| sim.channel(UNI, c)).collect();
    for _ in 0..200 {
        hal.send_frame(&f).unwrap();
    }
    let after: Vec<Option<u8>> = (0..9).map(|c| sim.channel(UNI, c)).collect();
    assert_eq!(snapshot, after, "200 frames idênticos → bytes idênticos");
}

/// Custo da calibração, **medido** (ADR-0019 exige medição, não estimativa).
/// `cargo test -p led-hal --test calibration_output -- --ignored --nocapture`
#[test]
#[ignore = "measurement bench (wall-clock) — run manually with --ignored --nocapture"]
fn bench_calibration_cost() {
    use std::time::Instant;

    const N: usize = 6_200; // escala do rig real
    let specs = [DeviceSpec { id: DEV, universes: 37 }];
    let layout = CompiledLayout::linear(N, &specs, RgbOrder::Grb);
    let sim = SimulatorDevice::new(DEV, layout.device_universes(DEV));
    let plain = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]);

    let layout2 = CompiledLayout::linear(N, &specs, RgbOrder::Grb);
    let sim2 = SimulatorDevice::new(DEV, layout2.device_universes(DEV));
    let mut cal = Calibration::new();
    cal.set(DEV, 2.2, 0.8);
    let calibrated =
        Hal::new(layout2, vec![sim2 as Arc<dyn DeviceDriver>]).with_calibration(cal);

    let f = frame(N, PixelColor::rgb(120, 60, 30));
    let run = |hal: &Hal| {
        for _ in 0..200 {
            hal.send_frame(&f).unwrap();
        }
        let t = Instant::now();
        for _ in 0..2_000 {
            hal.send_frame(&f).unwrap();
        }
        t.elapsed().as_nanos() as f64 / 2_000.0
    };

    let a = run(&plain);
    let b = run(&calibrated);
    eprintln!("── ADR-0019 · custo da calibração ({N} px, 37 universos) ──");
    eprintln!("  sem calibração : {a:>10.0} ns/frame");
    eprintln!("  com calibração : {b:>10.0} ns/frame");
    eprintln!("  delta          : {:>10.0} ns/frame  (×{:.2})", b - a, b / a.max(1.0));
    eprintln!("  orçamento cabeado: 5 ms/frame = 5_000_000 ns");
    assert!(a > 0.0 && b > 0.0);
}

/// Calibração é **por device**: um device registrado é corrigido, o outro não.
#[test]
fn calibration_is_per_device() {
    // 340 px = 170 por universo: o device 0 fica com o universo 0 e o device 1 com o 1.
    // (Com poucos pixels, `linear` colocaria todos no primeiro universo e o device 1 sequer
    // apareceria no layout.)
    let specs = [DeviceSpec { id: 0, universes: 1 }, DeviceSpec { id: 1, universes: 1 }];
    let layout = CompiledLayout::linear(340, &specs, RgbOrder::Rgb);
    let sim_a = SimulatorDevice::new(0, layout.device_universes(0));
    let sim_b = SimulatorDevice::new(1, layout.device_universes(1));

    let mut cal = Calibration::new();
    cal.set(0, 1.0, 0.5); // só o device 0

    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim_a.clone(), sim_b.clone()];
    let hal = Hal::new(layout, devices).with_calibration(cal);
    hal.send_frame(&LogicalFrame::new(vec![PixelColor::rgb(200, 200, 200); 340], 0)).unwrap();

    assert_eq!(sim_a.channel(0, 0), Some(100), "device 0 calibrado (metade)");
    assert_eq!(sim_b.channel(1, 0), Some(200), "device 1 sem calibração — inalterado");
}
