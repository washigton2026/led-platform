//! Slice 3 — presets como **dado puro** (ADR-0018).
//!
//! Este módulo é uma **tabela**, não código: contém a definição de [`PresetRow`] e a constante
//! [`PRESETS`]. **Zero `fn`, zero `impl`, zero ramificação** — a conversão de uma linha em
//! [`crate::HardwareProfile`] vive no `registry`, porque é responsabilidade do registro, não do
//! preset.
//!
//! **Adicionar hardware novo = adicionar uma linha aqui.** Nenhum braço de `match`, nenhum
//! `if`, nenhum tipo novo. ESP32, ESP32-POE, Falcon, Advatek, Raspberry Pi e WLED não são
//! variantes de enum — são linhas desta tabela.
//!
//! Como todos os campos são literais e variantes de enum, `PRESETS` é uma `const` de verdade:
//! dado resolvido em tempo de compilação, no qual é **impossível** esconder lógica.
//!
//! ## Sobre os valores
//!
//! Os números abaixo são **pontos de partida plausíveis por família de controlador**, não
//! medições — capacidade real varia por revisão de placa, firmware e fiação. Cada instalação
//! deve ajustar `max_pixels`, `refresh_hz` e sobretudo `Power` contra a folha de dados e a
//! fonte usada. O validador (Slice 2) recusa combinações impossíveis, mas não sabe qual é a
//! sua fonte.

use crate::{ColorFormat, OutputInterface, Protocol, RgbOrder};

/// Uma linha da tabela de presets: dado puro, achatado (sem aninhamento) para que a tabela
/// seja legível e `const`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresetRow {
    /// Chave estável usada para localizar o preset no registro.
    pub name: &'static str,
    // ── Identity ──
    pub vendor: &'static str,
    pub model: &'static str,
    pub firmware: &'static str,
    pub firmware_version: &'static str,
    // ── Capabilities ──
    pub protocol: Protocol,
    pub output_interface: OutputInterface,
    pub color: ColorFormat,
    pub supports_discovery: bool,
    pub supports_metrics: bool,
    // ── Limits ──
    pub pixels_per_universe: u16,
    pub max_pixels: u32,
    pub refresh_hz: u16,
    // ── Power (declarado, não medido) ──
    pub voltage_v: f32,
    pub max_current_a: f32,
    // ── Calibration ──
    pub gamma: f32,
    pub brightness: f32,
}

/// Presets embutidos. **Dado**, não código.
pub const PRESETS: &[PresetRow] = &[
    // O nó de bancada real do rig: ESP32 DevKit V1 não tem Ethernet — declara WiFi, e o
    // validador emite o aviso previsto pelo ADR-0005. O aviso é a regra funcionando.
    PresetRow {
        name: "esp32-devkit-wled-artnet",
        vendor: "Espressif",
        model: "ESP32 DevKit V1",
        firmware: "WLED",
        firmware_version: "16.0.1",
        protocol: Protocol::ArtNet,
        output_interface: OutputInterface::WiFi,
        color: ColorFormat::Rgb(RgbOrder::Grb),
        supports_discovery: true,
        supports_metrics: true,
        pixels_per_universe: 170,
        max_pixels: 1_500,
        refresh_hz: 40,
        voltage_v: 5.0,
        max_current_a: 10.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Alvo de migração para show ao vivo: Ethernet cabeada + DDP (ADR-0003 e ADR-0005).
    PresetRow {
        name: "esp32-poe-wled-ddp",
        vendor: "Olimex",
        model: "ESP32-POE",
        firmware: "WLED",
        firmware_version: "16.0.1",
        protocol: Protocol::Ddp,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgb(RgbOrder::Grb),
        supports_discovery: true,
        supports_metrics: true,
        pixels_per_universe: 170,
        max_pixels: 1_500,
        refresh_hz: 44,
        voltage_v: 5.0,
        max_current_a: 10.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Controlador profissional falando sACN — nenhum código específico de Falcon existe
    // nem é necessário: o protocolo já resolve.
    PresetRow {
        name: "falcon-f16v3-sacn",
        vendor: "Falcon",
        model: "F16V3",
        firmware: "stock",
        firmware_version: "unknown",
        protocol: Protocol::Sacn,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgb(RgbOrder::Grb),
        supports_discovery: true,
        supports_metrics: false,
        pixels_per_universe: 170,
        max_pixels: 16_384,
        refresh_hz: 44,
        voltage_v: 12.0,
        max_current_a: 60.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Idem Advatek: preset, zero código específico.
    PresetRow {
        name: "advatek-pixlite16-sacn",
        vendor: "Advatek",
        model: "PixLite 16 Mk2",
        firmware: "stock",
        firmware_version: "unknown",
        protocol: Protocol::Sacn,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgb(RgbOrder::Grb),
        supports_discovery: true,
        supports_metrics: false,
        pixels_per_universe: 170,
        max_pixels: 16_320,
        refresh_hz: 44,
        voltage_v: 12.0,
        max_current_a: 60.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Raspberry Pi rodando FPP — outra família de placa, mesma tabela.
    PresetRow {
        name: "raspberry-fpp-sacn",
        vendor: "Raspberry Pi Foundation",
        model: "Raspberry Pi 4",
        firmware: "FPP",
        firmware_version: "unknown",
        protocol: Protocol::Sacn,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgb(RgbOrder::Grb),
        supports_discovery: true,
        supports_metrics: true,
        pixels_per_universe: 170,
        max_pixels: 32_768,
        refresh_hz: 44,
        voltage_v: 5.0,
        max_current_a: 20.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Fita RGBW sobre sACN: RGBW é um VALOR de `color`, não um tipo de hardware. Note o
    // `pixels_per_universe` menor — 128 × 4 canais = 512, o limite que o validador cobra.
    PresetRow {
        name: "generic-sk6812-rgbw-sacn",
        vendor: "generic",
        model: "SK6812 RGBW strip",
        firmware: "n/a",
        firmware_version: "n/a",
        protocol: Protocol::Sacn,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgbw(RgbOrder::Grb, crate::WhiteMode::Min),
        supports_discovery: false,
        supports_metrics: false,
        pixels_per_universe: 128,
        max_pixels: 4_096,
        refresh_hz: 44,
        voltage_v: 5.0,
        max_current_a: 20.0,
        gamma: 2.2,
        brightness: 1.0,
    },
    // Ponto de partida neutro para hardware não catalogado. "Custom" não é um caminho
    // especial no código — é só mais uma linha.
    PresetRow {
        name: "custom",
        vendor: "custom",
        model: "custom",
        firmware: "custom",
        firmware_version: "0",
        protocol: Protocol::Sacn,
        output_interface: OutputInterface::Ethernet,
        color: ColorFormat::Rgb(RgbOrder::Rgb),
        supports_discovery: false,
        supports_metrics: false,
        pixels_per_universe: 170,
        max_pixels: 1_024,
        refresh_hz: 40,
        voltage_v: 5.0,
        max_current_a: 5.0,
        gamma: 2.2,
        brightness: 1.0,
    },
];
