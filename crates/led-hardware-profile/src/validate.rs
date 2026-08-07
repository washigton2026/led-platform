//! Slice 2 — validação de um [`HardwareProfile`] em **design-time** (ADR-0018).
//!
//! ## Por que o validador recebe as capacidades disponíveis como DADO
//!
//! Detectar "driver inexistente" exige saber **quais drivers existem** — mas este crate é
//! *leaf* e **não pode depender** de `led-hal`/`led-protocols` (ADR-0018 / HardwareProfileGuardian
//! check 4). A saída é **injeção de dados**: quem conhece os drivers (o HAL, no startup) passa
//! um [`Available`]. O profile declara, o driver executa, e o validador só compara dados.
//!
//! ## Fronteira com o enforcement
//!
//! O validador **não usurpa** guardas de runtime. `OutputInterface::WiFi` é reportado como
//! **aviso** — quem bloqueia o início do show é o `NetworkGuard` (ADR-0005). Igualmente, este
//! módulo não toca runtime, HAL, driver ou hot-path: roda em design-time/startup e produz
//! apenas uma lista de achados.

use led_core::UNIVERSE_SIZE;

use crate::{ColorFormat, HardwareProfile, OutputInterface, Protocol, SCHEMA_VERSION};

/// Capacidades que o ambiente realmente oferece, passadas como **dado** pelo chamador que
/// conhece os drivers. Nenhuma dependência de crate de driver é criada por isto.
#[derive(Clone, Copy, Debug)]
pub struct Available<'a> {
    /// Interfaces físicas com driver disponível.
    pub interfaces: &'a [OutputInterface],
    /// Protocolos de fio com driver disponível.
    pub protocols: &'a [Protocol],
}

/// Gravidade de um achado. `Error` impede o uso do profile; `Warning` é informativo e o
/// enforcement real acontece noutro lugar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// O que a validação encontrou. Cada variante nomeia a regra e carrega o valor ofensor —
/// nunca uma string genérica.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finding {
    /// `schema_version` desconhecida — migração é explícita, nunca best-effort (ADR-0018).
    UnknownSchemaVersion { found: u16, expected: u16 },
    /// A interface declarada não tem driver disponível (hoje: `Spi`/`Pwm`).
    InterfaceHasNoDriver { interface: OutputInterface },
    /// O protocolo declarado não tem driver disponível.
    ProtocolHasNoDriver { protocol: Protocol },
    /// Os pixels declarados não cabem num universo com esse número de canais/pixel.
    /// Só se aplica a protocolos baseados em universo (sACN/Art-Net); DDP é endereçado por byte.
    PixelsExceedUniverse { pixels_per_universe: u16, channels_per_pixel: u16, universe_size: u16 },
    /// Um limite obrigatório está zerado.
    ZeroLimit { field: &'static str },
    /// Orçamento elétrico inválido.
    InvalidPower { field: &'static str },
    /// Calibração fora de faixa.
    InvalidCalibration { field: &'static str },
    /// RGBW sobre DDP: o stride de 4 canais funciona, mas o cabeçalho DDP declara o data type
    /// RGB de 8 bits (`led-protocols::ddp`), então um receptor estrito pode interpretar mal.
    /// Limitação conhecida — aviso, não erro.
    RgbwOverDdpDataType,
    /// WiFi declarado: proibido para show ao vivo (ADR-0005). O `NetworkGuard` é quem bloqueia.
    WifiNotPermittedLive,
}

impl Finding {
    /// Gravidade desta regra.
    pub fn severity(self) -> Severity {
        match self {
            Finding::RgbwOverDdpDataType | Finding::WifiNotPermittedLive => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

/// Resultado da validação: todos os achados, na ordem em que as regras rodaram.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Validation {
    pub findings: Vec<Finding>,
}

impl Validation {
    /// `true` se existe pelo menos um achado de gravidade `Error`.
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity() == Severity::Error)
    }

    /// Somente os achados de erro.
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.severity() == Severity::Error)
    }

    /// Somente os avisos.
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.severity() == Severity::Warning)
    }
}

/// `true` apenas para um número estritamente positivo. `NaN` **não** é positivo — é essa
/// propriedade que faz as regras de `power`/`calibration` rejeitarem `NaN` sem caso especial.
fn is_positive(x: f32) -> bool {
    x > 0.0
}

/// Protocolos endereçados por universo. DDP é endereçado por **byte** (sem universos), então
/// limites por universo não o restringem.
fn is_universe_based(p: Protocol) -> bool {
    match p {
        Protocol::Sacn | Protocol::ArtNet => true,
        Protocol::Ddp => false,
    }
}

/// Valida um profile contra as capacidades realmente disponíveis.
///
/// Roda em design-time/startup. Não toca runtime, HAL, driver nem hot-path.
pub fn validate(profile: &HardwareProfile, available: &Available) -> Validation {
    let mut findings = Vec::new();

    // 1 · schema_version conhecida
    if profile.schema_version != SCHEMA_VERSION {
        findings.push(Finding::UnknownSchemaVersion {
            found: profile.schema_version,
            expected: SCHEMA_VERSION,
        });
    }

    let caps = &profile.capabilities;

    // 2 · a interface declarada tem driver?
    if !available.interfaces.contains(&caps.output_interface) {
        findings.push(Finding::InterfaceHasNoDriver { interface: caps.output_interface });
    }

    // 3 · o protocolo declarado tem driver?
    if !available.protocols.contains(&caps.protocol) {
        findings.push(Finding::ProtocolHasNoDriver { protocol: caps.protocol });
    }

    // 4 · os pixels cabem no universo? (só para protocolos baseados em universo)
    let channels = caps.color.channels() as u16;
    if is_universe_based(caps.protocol) {
        let needed = profile.limits.pixels_per_universe as usize * channels as usize;
        if needed > UNIVERSE_SIZE {
            findings.push(Finding::PixelsExceedUniverse {
                pixels_per_universe: profile.limits.pixels_per_universe,
                channels_per_pixel: channels,
                universe_size: UNIVERSE_SIZE as u16,
            });
        }
    }

    // 5 · limites obrigatórios não podem ser zero
    if profile.limits.pixels_per_universe == 0 {
        findings.push(Finding::ZeroLimit { field: "pixels_per_universe" });
    }
    if profile.limits.max_pixels == 0 {
        findings.push(Finding::ZeroLimit { field: "max_pixels" });
    }
    if profile.limits.refresh_hz == 0 {
        findings.push(Finding::ZeroLimit { field: "refresh_hz" });
    }

    // 6 · orçamento elétrico declarado precisa ser positivo
    if !is_positive(profile.power.voltage_v) {
        findings.push(Finding::InvalidPower { field: "voltage_v" });
    }
    if !is_positive(profile.power.max_current_a) {
        findings.push(Finding::InvalidPower { field: "max_current_a" });
    }

    // 7 · calibração dentro da faixa
    if !is_positive(profile.calibration.gamma) {
        findings.push(Finding::InvalidCalibration { field: "gamma" });
    }
    if !(0.0..=1.0).contains(&profile.calibration.brightness) {
        findings.push(Finding::InvalidCalibration { field: "brightness" });
    }

    // 8 · RGBW sobre DDP — o data type do cabeçalho DDP é RGB8 (aviso, não erro)
    if matches!(caps.color, ColorFormat::Rgbw(_, _)) && caps.protocol == Protocol::Ddp {
        findings.push(Finding::RgbwOverDdpDataType);
    }

    // 9 · WiFi declarado — proibido ao vivo; o NetworkGuard bloqueia (aviso)
    if caps.output_interface == OutputInterface::WiFi {
        findings.push(Finding::WifiNotPermittedLive);
    }

    Validation { findings }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Calibration, Capabilities, Identity, Limits, Power, RgbOrder, WhiteMode};

    const ALL_IFACES: &[OutputInterface] = &[OutputInterface::Ethernet, OutputInterface::WiFi];
    const ALL_PROTOS: &[Protocol] = &[Protocol::Sacn, Protocol::ArtNet, Protocol::Ddp];

    fn available() -> Available<'static> {
        Available { interfaces: ALL_IFACES, protocols: ALL_PROTOS }
    }

    fn valid() -> HardwareProfile {
        HardwareProfile {
            schema_version: SCHEMA_VERSION,
            identity: Identity {
                vendor: "Espressif".into(),
                model: "ESP32-POE".into(),
                firmware: "WLED".into(),
                firmware_version: "16.0.1".into(),
                serial: None,
            },
            capabilities: Capabilities {
                protocol: Protocol::ArtNet,
                output_interface: OutputInterface::Ethernet,
                color: ColorFormat::Rgb(RgbOrder::Grb),
                supports_discovery: true,
                supports_metrics: false,
            },
            limits: Limits { pixels_per_universe: 170, max_pixels: 1_560, refresh_hz: 44 },
            transport: crate::Transport { mtu_bytes: 1_500, heartbeat_ms: 800 },
            power: Power { voltage_v: 5.0, max_current_a: 10.0 },
            calibration: Calibration { gamma: 2.2, brightness: 1.0 },
        }
    }

    #[test]
    fn a_valid_profile_produces_no_findings() {
        let v = validate(&valid(), &available());
        assert_eq!(v.findings, vec![], "profile válido não deve gerar achados");
        assert!(!v.has_errors());
    }

    #[test]
    fn unknown_schema_version_is_an_error() {
        let mut p = valid();
        p.schema_version = SCHEMA_VERSION + 7;
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::UnknownSchemaVersion {
            found: SCHEMA_VERSION + 7,
            expected: SCHEMA_VERSION,
        }));
        assert!(v.has_errors(), "migração é explícita, nunca best-effort");
    }

    #[test]
    fn interface_without_driver_is_an_error() {
        // Spi/Pwm são declaráveis pelo schema mas não têm driver — o validador é quem recusa.
        for iface in [OutputInterface::Spi, OutputInterface::Pwm] {
            let mut p = valid();
            p.capabilities.output_interface = iface;
            let v = validate(&p, &available());
            assert!(v.findings.contains(&Finding::InterfaceHasNoDriver { interface: iface }));
            assert!(v.has_errors());
        }
    }

    #[test]
    fn protocol_without_driver_is_an_error() {
        let only_ddp: &[Protocol] = &[Protocol::Ddp];
        let av = Available { interfaces: ALL_IFACES, protocols: only_ddp };
        let v = validate(&valid(), &av); // profile fala ArtNet
        assert!(v.findings.contains(&Finding::ProtocolHasNoDriver { protocol: Protocol::ArtNet }));
    }

    #[test]
    fn rgb_pixels_must_fit_the_universe() {
        let mut p = valid();
        p.limits.pixels_per_universe = 170; // 170*3 = 510 <= 512
        assert!(validate(&p, &available()).findings.is_empty());

        p.limits.pixels_per_universe = 171; // 171*3 = 513 > 512
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::PixelsExceedUniverse {
            pixels_per_universe: 171,
            channels_per_pixel: 3,
            universe_size: UNIVERSE_SIZE as u16,
        }));
    }

    #[test]
    fn rgbw_halves_the_pixels_that_fit_a_universe() {
        let mut p = valid();
        p.capabilities.color = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        p.limits.pixels_per_universe = 128; // 128*4 = 512 <= 512
        assert!(validate(&p, &available()).findings.is_empty(), "128 px RGBW cabem exatamente");

        p.limits.pixels_per_universe = 129; // 129*4 = 516 > 512
        assert!(validate(&p, &available()).has_errors(), "129 px RGBW não cabem");
    }

    #[test]
    fn ddp_is_byte_addressed_so_universe_limits_do_not_apply() {
        let mut p = valid();
        p.capabilities.protocol = Protocol::Ddp;
        p.limits.pixels_per_universe = 487; // irrelevante para DDP
        let v = validate(&p, &available());
        assert!(
            !v.findings.iter().any(|f| matches!(f, Finding::PixelsExceedUniverse { .. })),
            "DDP não é baseado em universo"
        );
    }

    #[test]
    fn rgbw_over_ddp_is_a_warning_not_an_error() {
        let mut p = valid();
        p.capabilities.protocol = Protocol::Ddp;
        p.capabilities.color = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::RgbwOverDdpDataType));
        assert!(!v.has_errors(), "limitação conhecida do data type DDP é aviso, não erro");
        assert_eq!(v.warnings().count(), 1);
    }

    #[test]
    fn wifi_is_a_warning_the_network_guard_enforces() {
        let mut p = valid();
        p.capabilities.output_interface = OutputInterface::WiFi;
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::WifiNotPermittedLive));
        assert!(!v.has_errors(), "o validador não usurpa o NetworkGuard (ADR-0005)");
    }

    #[test]
    fn zero_limits_are_errors() {
        for (field, mutate) in [
            ("pixels_per_universe", (|p: &mut HardwareProfile| p.limits.pixels_per_universe = 0) as fn(&mut HardwareProfile)),
            ("max_pixels", |p: &mut HardwareProfile| p.limits.max_pixels = 0),
            ("refresh_hz", |p: &mut HardwareProfile| p.limits.refresh_hz = 0),
        ] {
            let mut p = valid();
            mutate(&mut p);
            let v = validate(&p, &available());
            assert!(v.findings.contains(&Finding::ZeroLimit { field }), "{field} zerado deve falhar");
        }
    }

    #[test]
    fn invalid_power_and_calibration_are_errors() {
        let mut p = valid();
        p.power.voltage_v = 0.0;
        p.power.max_current_a = -1.0;
        p.calibration.gamma = 0.0;
        p.calibration.brightness = 1.5;
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::InvalidPower { field: "voltage_v" }));
        assert!(v.findings.contains(&Finding::InvalidPower { field: "max_current_a" }));
        assert!(v.findings.contains(&Finding::InvalidCalibration { field: "gamma" }));
        assert!(v.findings.contains(&Finding::InvalidCalibration { field: "brightness" }));
        assert_eq!(v.errors().count(), 4);
    }

    #[test]
    fn nan_calibration_is_rejected() {
        // Comparações com NaN são falsas — as regras usam `!(x > 0.0)` justamente para pegá-lo.
        let mut p = valid();
        p.calibration.gamma = f32::NAN;
        p.calibration.brightness = f32::NAN;
        let v = validate(&p, &available());
        assert!(v.findings.contains(&Finding::InvalidCalibration { field: "gamma" }));
        assert!(v.findings.contains(&Finding::InvalidCalibration { field: "brightness" }));
    }

    #[test]
    fn findings_accumulate_instead_of_short_circuiting() {
        // Um profile ruim deve reportar TUDO de uma vez, não a primeira falha.
        let mut p = valid();
        p.schema_version = 99;
        p.capabilities.output_interface = OutputInterface::Spi;
        p.limits.max_pixels = 0;
        let v = validate(&p, &available());
        assert!(v.findings.len() >= 3, "achados acumulam: {:?}", v.findings);
    }
}
