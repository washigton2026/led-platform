//! E2E do fluxo HardwareProfile (ADR-0018), ponta a ponta:
//!
//! ```text
//! preset (dado) ─▶ HardwareProfile ─▶ Validator ─▶ CompiledLayout ─▶ Hal ─▶ SimulatorDevice
//!                                  └─▶ DriverConfig ────────────────┘
//! ```
//!
//! Por que este teste vive aqui, e não no `led-hardware-profile`: o `SimulatorDevice` está no
//! `led-hal`, e o crate do profile é *leaf* por contrato (ADR-0018 / Guardian check 4). O
//! `integration-tests` é o lar dos testes cross-crate — assim o E2E existe **sem** o profile
//! ganhar uma dependência de HAL.
//!
//! O que ele prova de fato: uma capacidade **declarada** num preset (formato de cor, pixels por
//! universo) chega **como bytes** ao dispositivo, atravessando validação, compilação e o HAL.

use std::sync::Arc;

use led_core::{DeviceDriver, LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{Hal, SimulatorDevice};
use led_hardware_profile::{
    compile_layout, driver_config, validate, Available, ColorFormat, HardwareRegistry,
    OutputInterface, Protocol, RgbOrder, WhiteMode,
};

const DEVICE: u16 = 0;
const FIRST_UNIVERSE: u16 = 1;

fn addr() -> std::net::SocketAddr {
    "192.168.2.156:4048".parse().unwrap()
}

/// Fluxo completo com o preset Ethernet+DDP (o alvo de migração do rig real).
#[test]
fn preset_to_wire_bytes_end_to_end() {
    // ── 1 · preset (dado) → profile ────────────────────────────────────────────
    let registry = HardwareRegistry::with_builtin();
    let profile = registry.profile("esp32-poe-wled-ddp").expect("preset embutido");
    assert_eq!(profile.capabilities.color, ColorFormat::Rgb(RgbOrder::Grb));

    // ── 2 · validação contra o que o ambiente oferece (injetado como DADO) ─────
    let available = Available {
        interfaces: &[OutputInterface::Ethernet],
        protocols: &[Protocol::Ddp],
    };
    let report = validate(&profile, &available);
    assert!(
        !report.has_errors(),
        "preset válido não deve ter erros: {:?}",
        report.errors().collect::<Vec<_>>()
    );

    // ── 3 · compilação → CompiledLayout + DriverConfig ─────────────────────────
    let pixel_count = 340u32; // 2 universos a 170 px/universo
    let layout = compile_layout(&profile, pixel_count, DEVICE, FIRST_UNIVERSE)
        .expect("layout compila a partir do profile");
    assert_eq!(layout.universe_count(), 2);

    let cfg = driver_config(&profile, DEVICE, addr(), FIRST_UNIVERSE);
    assert_eq!(cfg.protocol, Protocol::Ddp);
    assert_eq!(cfg.first_universe, FIRST_UNIVERSE);

    // ── 4 · HAL + SimulatorDevice construídos a partir do que foi compilado ────
    // O SimulatorDevice recebe exatamente os universos que o layout atribuiu ao device.
    let sim = SimulatorDevice::new(cfg.device_id, layout.device_universes(cfg.device_id));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    let hal = Hal::new(layout, devices);

    // ── 5 · um frame lógico atravessa tudo e vira bytes no dispositivo ─────────
    let mut pixels = vec![PixelColor::default(); pixel_count as usize];
    pixels[0] = PixelColor::rgb(10, 20, 30); // GRB declarado no preset → [20, 10, 30]
    pixels[170] = PixelColor::rgb(1, 2, 3); // primeiro pixel do 2º universo
    hal.send_frame(&LogicalFrame::new(pixels, 0)).expect("send");

    assert_eq!(sim.channel(FIRST_UNIVERSE, 0), Some(20), "G do pixel 0 (ordem GRB do preset)");
    assert_eq!(sim.channel(FIRST_UNIVERSE, 1), Some(10), "R do pixel 0");
    assert_eq!(sim.channel(FIRST_UNIVERSE, 2), Some(30), "B do pixel 0");
    assert_eq!(sim.channel(FIRST_UNIVERSE + 1, 0), Some(2), "pixel 170 abre o universo seguinte");
    assert_eq!(sim.frames_sent(), 1);
}

/// O mesmo fluxo com o preset RGBW: 4 canais por pixel e o branco derivado no mapper
/// (ADR-0011) chegam ao dispositivo — RGBW é um **valor** do preset, não um caminho de código.
#[test]
fn rgbw_preset_reaches_the_device_as_four_channels() {
    let registry = HardwareRegistry::with_builtin();
    let profile = registry.profile("generic-sk6812-rgbw-sacn").expect("preset RGBW");
    assert_eq!(profile.capabilities.color, ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min));

    let available = Available {
        interfaces: &[OutputInterface::Ethernet],
        protocols: &[Protocol::Sacn],
    };
    assert!(!validate(&profile, &available).has_errors());

    // 128 px/universo × 4 canais = 512 exatos.
    let layout = compile_layout(&profile, 128, DEVICE, FIRST_UNIVERSE).expect("compila");
    assert_eq!(layout.universe_count(), 1);

    let sim = SimulatorDevice::new(DEVICE, layout.device_universes(DEVICE));
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]);

    let mut pixels = vec![PixelColor::default(); 128];
    pixels[1] = PixelColor::rgb(10, 20, 30); // GRB + W = min(10,20,30) = 10
    hal.send_frame(&LogicalFrame::new(pixels, 0)).expect("send");

    // Pixel 1 começa no canal 4 porque o formato declarado tem 4 canais/pixel.
    assert_eq!(sim.channel(FIRST_UNIVERSE, 4), Some(20), "G");
    assert_eq!(sim.channel(FIRST_UNIVERSE, 5), Some(10), "R");
    assert_eq!(sim.channel(FIRST_UNIVERSE, 6), Some(30), "B");
    assert_eq!(sim.channel(FIRST_UNIVERSE, 7), Some(10), "W derivado no mapper (WhiteMode::Min)");
}

/// Um `pixels_per_universe` **declarado abaixo do máximo** é honrado até os bytes — é a razão
/// de o campo existir (Slice 5, opção (i)).
#[test]
fn a_declared_packing_below_the_maximum_survives_to_the_device() {
    let registry = HardwareRegistry::with_builtin();
    let mut profile = registry.profile("esp32-poe-wled-ddp").expect("preset");
    profile.limits.pixels_per_universe = 150; // caberiam 170

    let layout = compile_layout(&profile, 300, DEVICE, FIRST_UNIVERSE).expect("compila");
    assert_eq!(layout.universe_count(), 2, "300/150 = 2 universos, não 300/170");

    let sim = SimulatorDevice::new(DEVICE, layout.device_universes(DEVICE));
    let hal = Hal::new(layout, vec![sim.clone() as Arc<dyn DeviceDriver>]);

    let mut pixels = vec![PixelColor::default(); 300];
    pixels[150] = PixelColor::rgb(7, 7, 7);
    hal.send_frame(&LogicalFrame::new(pixels, 0)).expect("send");

    assert_eq!(
        sim.channel(FIRST_UNIVERSE + 1, 0),
        Some(7),
        "o pixel 150 abre o 2º universo porque o preset declarou 150, não 170"
    );
}

/// Controle negativo: sem driver para a interface declarada, o fluxo **para na validação** e
/// nada chega ao dispositivo. O preset do ESP32 DevKit declara WiFi (a placa não tem Ethernet).
#[test]
fn a_profile_whose_interface_has_no_driver_never_reaches_the_device() {
    let registry = HardwareRegistry::with_builtin();
    let profile = registry.profile("esp32-devkit-wled-artnet").expect("preset WiFi");
    assert_eq!(profile.capabilities.output_interface, OutputInterface::WiFi);

    // Ambiente sem driver WiFi.
    let available = Available {
        interfaces: &[OutputInterface::Ethernet],
        protocols: &[Protocol::ArtNet],
    };
    let report = validate(&profile, &available);
    assert!(report.has_errors(), "sem driver para a interface, o profile deve ser recusado");

    // O gate é do chamador: com erros, não se compila nem se envia nada.
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![];
    assert!(devices.is_empty(), "nenhum dispositivo é construído a partir de um profile inválido");
}

/// Com driver WiFi disponível o profile passa, mas o **aviso** do ADR-0005 permanece — o
/// validador declara, o `NetworkGuard` é quem bloqueia o show ao vivo.
#[test]
fn wifi_is_a_warning_not_a_compile_failure() {
    let registry = HardwareRegistry::with_builtin();
    let profile = registry.profile("esp32-devkit-wled-artnet").expect("preset WiFi");

    let available = Available {
        interfaces: &[OutputInterface::WiFi],
        protocols: &[Protocol::ArtNet],
    };
    let report = validate(&profile, &available);
    assert!(!report.has_errors(), "WiFi não é erro de validação");
    assert_eq!(report.warnings().count(), 1, "mas é um aviso (ADR-0005)");

    // E ainda assim compila: a decisão de iniciar o show é do NetworkGuard, não deste caminho.
    let layout = compile_layout(&profile, 170, DEVICE, FIRST_UNIVERSE).expect("compila");
    assert_eq!(layout.universe_count(), 1);
}
