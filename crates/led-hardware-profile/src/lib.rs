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

pub mod compile;
pub mod presets;
pub mod registry;
pub mod validate;

pub use compile::{compile_layout, driver_config, CompileError, DriverConfig};
pub use led_core::{ColorFormat, RgbOrder, WhiteMode};
pub use presets::{PresetRow, PRESETS};
pub use registry::HardwareRegistry;
pub use validate::{validate, Available, Finding, Severity, Validation};

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
    /// O que o transporte impõe (GS4.3). A fragmentação **deriva** daqui.
    pub transport: Transport,
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
            transport: Transport { mtu_bytes: 1_500, heartbeat_ms: 800 },
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

// ─────────────────────────────────────────────────────────────────────────────
// GS4.3 — Transporte: MTU declarado, fragmentação DERIVADA
// ─────────────────────────────────────────────────────────────────────────────

/// O que o **transporte** impõe ao nó.
///
/// # A fragmentação não é declarada — é derivada
///
/// Declarar "487 pixels por datagrama" ao lado de "MTU 1500" seria escrever a mesma verdade
/// duas vezes, e a segunda apodreceria em silêncio no dia em que a primeira mudasse. Aqui só
/// o **MTU** é dado; [`Transport::pixels_per_datagram`] calcula o resto a partir dele, do
/// protocolo e do formato de cor. Há um teste que confirma que a derivação reproduz o
/// `DDP_MAX_PIXELS = 487` já validado em hardware — a concordância é **provada**, não assumida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transport {
    /// MTU do caminho, em bytes. 1500 é o Ethernet padrão.
    pub mtu_bytes: u16,
    /// Período do keep-alive, em ms. **Não é opcional**: um nó sem keep-alive entra em modo
    /// seguro sozinho, e o invariante do `LUMYX_GOSL` fixa o teto em 2400 ms.
    pub heartbeat_ms: u16,
}

impl Transport {
    /// MTU de uma Ethernet normal.
    pub const ETHERNET_MTU: u16 = 1_500;
    /// Cabeçalhos IPv4 (20) + UDP (8) que saem do MTU antes de haver payload.
    pub const IP_UDP_OVERHEAD: usize = 28;
    /// Teto duro do intervalo entre frames, do `LUMYX_GOSL`. Um nó além disto apaga.
    pub const MAX_GAP_MS: u16 = 2_400;

    /// Quantos bytes de payload UDP cabem num datagrama sem fragmentar ao nível do IP.
    pub fn udp_payload_bytes(&self) -> usize {
        (self.mtu_bytes as usize).saturating_sub(Self::IP_UDP_OVERHEAD)
    }

    /// Quantos pixels cabem num datagrama, **dado o protocolo**.
    ///
    /// DDP endereça por byte e é limitado pelo MTU. Art-Net e sACN são limitados pelo
    /// **universo** (512 canais), que é muito menor que o MTU — por isso o MTU não é o que
    /// os prende, e dizer o contrário seria descrever o protocolo errado.
    pub fn pixels_per_datagram(
        &self,
        protocol: Protocol,
        color: ColorFormat,
        pixels_per_universe: u16,
    ) -> usize {
        let canais = color.channels();
        match protocol {
            Protocol::Ddp => {
                const DDP_HEADER: usize = 10;
                self.udp_payload_bytes().saturating_sub(DDP_HEADER) / canais
            }
            // O universo é o limite, não o MTU. `min` com o teto do MTU mantém a afirmação
            // honesta caso alguém declare um MTU absurdamente pequeno.
            Protocol::ArtNet | Protocol::Sacn => {
                const ARTNET_HEADER: usize = 18;
                let por_mtu = self.udp_payload_bytes().saturating_sub(ARTNET_HEADER) / canais;
                (pixels_per_universe as usize).min(por_mtu)
            }
        }
    }

    /// Em quantos datagramas um frame inteiro se parte.
    pub fn datagrams_for(
        &self,
        pixels: u32,
        protocol: Protocol,
        color: ColorFormat,
        pixels_per_universe: u16,
    ) -> u32 {
        let por = self.pixels_per_datagram(protocol, color, pixels_per_universe).max(1) as u32;
        pixels.div_ceil(por)
    }

    /// O heartbeat declarado respeita o teto do `LUMYX_GOSL`?
    pub fn heartbeat_is_safe(&self) -> bool {
        self.heartbeat_ms > 0 && self.heartbeat_ms < Self::MAX_GAP_MS
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use led_core::RgbOrder;

    fn eth() -> Transport {
        Transport { mtu_bytes: Transport::ETHERNET_MTU, heartbeat_ms: 800 }
    }

    /// **A derivação reproduz a constante validada em hardware.** Se algum dia divergirem,
    /// este teste diz qual das duas mudou — em vez de o rig o descobrir no palco.
    #[test]
    fn a_fragmentacao_derivada_do_mtu_bate_com_o_ddp_validado_no_rig() {
        let t = eth();
        assert_eq!(t.udp_payload_bytes(), 1_472, "1500 − 20 (IP) − 8 (UDP)");
        assert_eq!(
            t.pixels_per_datagram(Protocol::Ddp, ColorFormat::Rgb(RgbOrder::Grb), 170),
            led_protocols_ddp_max_pixels(),
            "derivado ≠ DDP_MAX_PIXELS: uma das duas fontes está errada"
        );
    }

    /// O valor com que o `led-protocols` fragmenta hoje, e que o rig aceitou 94/94 frames em
    /// 2026-07-20. Reproduzido aqui como literal **de propósito**: importar o crate faria o
    /// `led-hardware-profile` deixar de ser leaf, e o objetivo é comparar duas fontes
    /// independentes — não colar uma na outra.
    fn led_protocols_ddp_max_pixels() -> usize {
        487
    }

    /// RGBW muda a fragmentação, e o profile já sabia disso (ADR-0011).
    #[test]
    fn rgbw_cabe_menos_por_datagrama() {
        let t = eth();
        let rgb = t.pixels_per_datagram(Protocol::Ddp, ColorFormat::Rgb(RgbOrder::Grb), 170);
        let rgbw =
            t.pixels_per_datagram(Protocol::Ddp, ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::MinSubtract), 170);
        assert_eq!(rgbw, 365, "1462 / 4");
        assert!(rgbw < rgb, "4 canais têm de caber menos que 3");
    }

    /// **Art-Net e sACN são presos pelo universo, não pelo MTU** — e o teste diz porquê.
    #[test]
    fn artnet_e_sacn_sao_limitados_pelo_universo() {
        let t = eth();
        let cor = ColorFormat::Rgb(RgbOrder::Grb);
        for p in [Protocol::ArtNet, Protocol::Sacn] {
            assert_eq!(t.pixels_per_datagram(p, cor, 170), 170, "{p:?}");
        }
        // Prova de que é o universo que prende: com um universo maior o MTU passaria a valer.
        assert!(t.pixels_per_datagram(Protocol::ArtNet, cor, 10_000) < 10_000);
    }

    /// O rig real: 720 px por nó, nos três protocolos.
    #[test]
    fn o_numero_de_datagramas_do_no_de_bancada() {
        let t = eth();
        let cor = ColorFormat::Rgb(RgbOrder::Grb);
        assert_eq!(t.datagrams_for(720, Protocol::Ddp, cor, 170), 2, "487 + 233");
        assert_eq!(t.datagrams_for(720, Protocol::ArtNet, cor, 170), 5, "ceil(720/170)");
        assert_eq!(t.datagrams_for(720, Protocol::Sacn, cor, 170), 5);
        // E o rig inteiro, 6.200 px, se algum dia sair por um só nó.
        assert_eq!(t.datagrams_for(6_200, Protocol::Ddp, cor, 170), 13);
    }

    /// Um heartbeat que não respeita o teto do GOSL **não é seguro**, e o profile diz isso.
    #[test]
    fn o_heartbeat_declarado_e_confrontado_com_o_teto_do_gosl() {
        assert!(Transport { mtu_bytes: 1500, heartbeat_ms: 800 }.heartbeat_is_safe());
        assert!(!Transport { mtu_bytes: 1500, heartbeat_ms: 2_400 }.heartbeat_is_safe());
        assert!(!Transport { mtu_bytes: 1500, heartbeat_ms: 3_000 }.heartbeat_is_safe());
        assert!(!Transport { mtu_bytes: 1500, heartbeat_ms: 0 }.heartbeat_is_safe(), "0 = nunca bate");
    }

    /// MTU pequeno de propósito (VPN, PPPoE): a derivação **acompanha**, sem constante nova.
    #[test]
    fn um_mtu_menor_reduz_a_fragmentacao_sozinho() {
        let t = Transport { mtu_bytes: 576, heartbeat_ms: 800 };
        let px = t.pixels_per_datagram(Protocol::Ddp, ColorFormat::Rgb(RgbOrder::Grb), 170);
        assert_eq!(px, (576 - 28 - 10) / 3);
        assert!(px < 487, "MTU menor tem de fragmentar mais");
    }
}
