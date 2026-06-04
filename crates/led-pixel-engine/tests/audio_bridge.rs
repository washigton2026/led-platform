//! Proves the audio→light bridge end to end: real `led-audio` Analyzer → `AudioShare` →
//! reactive effect → (pipeline → HAL → device).

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use led_audio::Analyzer;
use led_core::{
    AudioFeatures, CompiledLayout, DeviceDriver, DeviceSpec, PixelColor, ProtocolOutput, RgbOrder,
};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{spawn, AudioShare, Band, BandPulse, Effect, Vec3};

fn tone(n: usize, freq: f32, sr: u32) -> Vec<f32> {
    (0..n).map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin()).collect()
}

fn render_first(fx: &dyn Effect) -> PixelColor {
    let mut out = [PixelColor::default(); 4];
    fx.render(0, &[Vec3::ZERO; 4], &mut out);
    out[0]
}

#[test]
fn real_analyzer_drives_a_reactive_effect() {
    let (sr, n) = (16_000u32, 2048usize);
    let mut an = Analyzer::new(sr, n);

    let loud = an.analyze(&tone(n, 100.0, sr), 0); // bass-heavy
    let quiet = an.analyze(&vec![0.0; n], 50); // silence

    assert!(loud.bass > quiet.bass, "analyzer sees bass energy in the tone");

    let share = Arc::new(AudioShare::new());
    let fx = BandPulse::new(PixelColor::rgb(255, 0, 0), Band::Bass, 1.0, share.clone());

    share.publish(&loud);
    let bright = render_first(&fx).r;
    share.publish(&quiet);
    let dark = render_first(&fx).r;

    assert!(bright > dark, "bass energy lights it up (bright {bright} > dark {dark})");
    assert_eq!(dark, 0, "silence ⇒ dark");
}

#[test]
fn reactive_effect_runs_through_the_pipeline_to_a_device() {
    const N: usize = 50;
    let layout = CompiledLayout::linear(N, &[DeviceSpec { id: 1, universes: 1 }], RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    let out: Arc<dyn ProtocolOutput> = Arc::new(Hal::new(layout, devices));

    let share = Arc::new(AudioShare::new());
    // Pre-publish strong bass so rendered frames are bright.
    share.publish(&AudioFeatures {
        sample_rate: 16_000,
        timestamp_ms: 0,
        rms: 1.0,
        beat: false,
        bass: 1.0,
        mid: 0.0,
        high: 0.0,
        spectrum: vec![0.0; 16],
    });

    let fx = BandPulse::new(PixelColor::rgb(255, 0, 0), Band::Bass, 1.0, share.clone());
    let handle = spawn(Box::new(fx), vec![Vec3::ZERO; N], out, 200);
    std::thread::sleep(Duration::from_millis(120));
    handle.stop();

    assert!(sim.frames_sent() >= 1, "frames reached the device");
    assert_eq!(sim.channel(0, 0), Some(255), "bass-driven red reached the wire");
}
