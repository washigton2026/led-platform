//! # led-hardware-profile — descritor de capacidades em design-time (ADR-0018)
//!
//! **Slice 1: schema puro.** Este crate contém **apenas dados**: nenhuma lógica, nenhuma
//! validação (Slice 2), nenhum preset (Slice 3), nenhum registry (Slice 3), nenhuma
//! integração com o HAL (Slice 5).
//!
//! ## O que este descritor é — e o que não é
//!
//! - **É** uma descrição declarativa das **capacidades** de um nó de hardware.
//! - **Não é** um catálogo de produtos. ESP32, ESP32-POE, Falcon, Advatek, Raspberry Pi,
//!   WLED e Custom são **presets** (dado), nunca variantes de enum.
//! - **Não é** estado de runtime. Estado vive em `led_core::DeviceStatus` e no
//!   `led-readmodel`. Aqui só há valores **declarados** em design-time.
//! - **Não executa** hardware: [`OutputInterface`] apenas **declara** a interface física;
//!   quem executa é um `DeviceDriver` (ADR-0018, resolução do Conflito A).
//!
//! ## Ciclo de vida
//!
//! ```text
//! HardwareProfile ──(startup)──▶ CompiledLayout + Driver Configuration ──▶ Runtime
//!        └── desaparece; NUNCA é consultado durante a renderização
//! ```
//!
//! ## Cor
//!
//! O formato de cor é [`led_core::ColorFormat`] (ADR-0011), **reusado como está**. Este crate
//! não declara nenhum tipo de cor próprio — uma segunda representação de RGBW é proibida.

#![forbid(unsafe_code)]

pub use led_core::{ColorFormat, RgbOrder, WhiteMode};

/// Versão do schema deste descritor. Um profile com `schema_version` desconhecida é
/// rejeitado na validação (Slice 2) — migração é explícita, nunca best-effort.
pub const SCHEMA_VERSION: u16 = 1;

/// Protocolo de fio que o nó fala. **Capacidade**, não produto: WLED, Falcon e Advatek são
/// presets que escolhem um destes valores; nenhum deles vira variante.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// E1.31 (sACN).
    Sacn,
    /// Art-Net (ArtDmx).
    ArtNet,
    /// DDP — caminho de capacidade preferencial para alvos WLED (ADR-0003).
    Ddp,
}

/// Interface física por onde os bytes saem do nó. O profile **declara**; a implementação de
/// cada interface é um `DeviceDriver` (ADR-0018).
///
/// Declarar uma interface **não a implementa**: um profile que declare uma interface sem
/// driver disponível falha explicitamente no startup (Slice 2/5). `Spi` e `Pwm` estão aqui
/// como valores declaráveis — seus drivers ainda não existem e têm ADR próprio.
///
/// Nomeado `OutputInterface` (e não `Connection`) porque `led_core::DeviceStatus` já carrega
/// conectividade de runtime; este tipo é estritamente design-time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputInterface {
    /// Ethernet cabeada — o único caminho suportado para show ao vivo (ADR-0005).
    Ethernet,
    /// WiFi — **proibido para show ao vivo** (ADR-0005). Declarável para config/monitoramento;
    /// o `NetworkGuard` é quem bloqueia o início do show.
    WiFi,
    /// SPI — driver ainda inexistente (fora do escopo do ADR-0018).
    Spi,
    /// PWM — driver ainda inexistente (fora do escopo do ADR-0018).
    Pwm,
}

/// Quem é o nó. Dado puro de identificação — nenhuma capacidade é inferida daqui.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Fabricante, como dado livre (ex.: "Espressif", "Falcon", "Advatek").
    pub vendor: String,
    /// Modelo, como dado livre (ex.: "ESP32-POE", "F16V3", "PixLite 16").
    pub model: String,
    /// Firmware em uso, como dado livre (ex.: "WLED", "FPP", "stock").
    pub firmware: String,
    /// Versão do firmware (ex.: "16.0.1"). Capacidades dependentes de firmware são
    /// declaradas em [`Capabilities`], nunca inferidas desta string.
    pub firmware_version: String,
    /// Número de série, quando o nó expõe um.
    pub serial: Option<String>,
}

/// O que o nó **sabe fazer**. Apenas valores declarativos ou booleanos — limites numéricos
/// de pixel vivem em [`Limits`], nunca aqui (ADR-0018).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Protocolo de fio.
    pub protocol: Protocol,
    /// Interface física de saída declarada.
    pub output_interface: OutputInterface,
    /// Formato de cor por pixel — `led_core::ColorFormat` (ADR-0011), reusado.
    pub color: ColorFormat,
    /// O nó responde a descoberta (ex.: ArtPoll).
    pub supports_discovery: bool,
    /// O nó expõe métricas próprias.
    pub supports_metrics: bool,
}

/// Limites numéricos do nó. **Único lar** dos limites de pixel (ADR-0018).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Pixels por universo (ex.: 170 para RGB em 510 canais).
    pub pixels_per_universe: u16,
    /// Total de pixels que o nó comporta.
    pub max_pixels: u32,
    /// Taxa de atualização alvo, em Hz.
    pub refresh_hz: u16,
}

/// Orçamento elétrico **declarado** em design-time. Não é proteção elétrica e não é medição:
/// tensão/corrente medidas são runtime e vivem no read-model (ADR-0018).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Power {
    /// Tensão nominal de alimentação, em volts (ex.: 5.0, 12.0).
    pub voltage_v: f32,
    /// Corrente máxima declarada, em ampères.
    pub max_current_a: f32,
}

/// Correção óptica por-output. Hoje o engine aplica gamma/brightness globalmente
/// (`led-pixel-engine`); movê-los para cá é uma slice separada (achado H5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// Expoente de gamma (ex.: 2.2).
    pub gamma: f32,
    /// Escala de brilho, 0.0..=1.0.
    pub brightness: f32,
}

/// O descritor completo de um nó, em design-time. Dado puro: sem lógica, sem estado de
/// runtime, sem dependência de HAL/driver/engine.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareProfile {
    /// Versão do schema com que este profile foi escrito. Ver [`SCHEMA_VERSION`].
    pub schema_version: u16,
    pub identity: Identity,
    pub capabilities: Capabilities,
    pub limits: Limits,
    pub power: Power,
    pub calibration: Calibration,
}

// ── Tests ──────────────────────────────────────────────────────────────────────
//
// Slice 1 é schema: os testes provam FORMA (composição, reuso do contrato de cor,
// ausência de estado de runtime), não comportamento — não há comportamento ainda.

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HardwareProfile {
        HardwareProfile {
            schema_version: SCHEMA_VERSION,
            identity: Identity {
                vendor: "Espressif".to_string(),
                model: "ESP32-POE".to_string(),
                firmware: "WLED".to_string(),
                firmware_version: "16.0.1".to_string(),
                serial: None,
            },
            capabilities: Capabilities {
                protocol: Protocol::Ddp,
                output_interface: OutputInterface::Ethernet,
                color: ColorFormat::Rgb(RgbOrder::Grb),
                supports_discovery: true,
                supports_metrics: false,
            },
            limits: Limits { pixels_per_universe: 170, max_pixels: 1_560, refresh_hz: 44 },
            power: Power { voltage_v: 5.0, max_current_a: 10.0 },
            calibration: Calibration { gamma: 2.2, brightness: 1.0 },
        }
    }

    #[test]
    fn profile_composes_from_orthogonal_capabilities() {
        let p = sample();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert_eq!(p.capabilities.protocol, Protocol::Ddp);
        assert_eq!(p.capabilities.output_interface, OutputInterface::Ethernet);
        assert_eq!(p.limits.pixels_per_universe, 170);
    }

    /// A mesma placa com outro firmware/protocolo/interface é o MESMO schema com outros
    /// valores — não uma variante nova. É esta propriedade que evita o enum de produtos.
    #[test]
    fn same_schema_expresses_a_different_node_without_new_variants() {
        let mut wifi_node = sample();
        wifi_node.identity.model = "ESP32 DevKit V1".to_string();
        wifi_node.capabilities.output_interface = OutputInterface::WiFi;
        wifi_node.capabilities.protocol = Protocol::ArtNet;
        assert_ne!(wifi_node.capabilities, sample().capabilities);
        assert_eq!(wifi_node.schema_version, sample().schema_version);
    }

    /// Cor vem de `led-core` (ADR-0011). Este teste falha a compilar se o crate declarar um
    /// tipo de cor próprio — é a prova estrutural de que não há segunda representação.
    #[test]
    fn color_is_the_led_core_contract_not_a_local_type() {
        let rgbw: led_core::ColorFormat = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        assert_eq!(rgbw.channels(), 4, "RGBW do led-core, reusado sem redefinir");
        let p = HardwareProfile { capabilities: Capabilities { color: rgbw, ..sample().capabilities }, ..sample() };
        assert_eq!(p.capabilities.color.channels(), 4);
    }

    /// Um profile RGBW muda apenas o valor de `color` — nenhum campo, tipo ou variante nova.
    #[test]
    fn rgbw_is_a_value_not_a_hardware_kind() {
        let rgb = sample();
        let mut rgbw = sample();
        rgbw.capabilities.color = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        assert_eq!(rgb.identity, rgbw.identity, "RGBW não muda a identidade do nó");
        assert_eq!(rgb.limits, rgbw.limits);
        assert_ne!(rgb.capabilities.color, rgbw.capabilities.color);
    }

    /// Interfaces sem driver são DECLARÁVEIS (o schema as expressa) — implementá-las é outra
    /// coisa, e a validação de startup é quem recusa (Slice 2/5).
    #[test]
    fn driverless_interfaces_are_declarable_by_the_schema() {
        for iface in [OutputInterface::Spi, OutputInterface::Pwm] {
            let mut p = sample();
            p.capabilities.output_interface = iface;
            assert_eq!(p.capabilities.output_interface, iface);
        }
    }

    /// Guardião estrutural: o descritor carrega apenas valores declarados. Se alguém
    /// acrescentar estado de runtime aqui, este teste deixa de refletir a estrutura e a
    /// revisão do HardwareProfileGuardian bloqueia.
    #[test]
    fn power_is_declared_budget_not_a_reading() {
        let p = sample();
        assert_eq!(p.power.voltage_v, 5.0);
        assert_eq!(p.power.max_current_a, 10.0);
    }
}
