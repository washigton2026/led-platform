//! Slice 3 — [`HardwareRegistry`]: registra e localiza presets (ADR-0018, item 3).
//!
//! O `HardwareProfile` permanece **descrição declarativa**; quem sabe *quais* presets existem
//! e como achá-los pelo nome é este registro. A conversão de uma [`PresetRow`] em
//! [`HardwareProfile`] também mora aqui — é responsabilidade do registro, não do preset, e é
//! por isso que `presets.rs` fica sem nenhum `fn`/`impl`.
//!
//! Design-time apenas: nada aqui toca runtime, HAL, driver ou hot-path.

use crate::{
    Calibration, Capabilities, HardwareProfile, Identity, Limits, Power, PresetRow,
    SCHEMA_VERSION, PRESETS,
};

/// Converte uma linha da tabela num profile. **Só atribuição** — nenhuma ramificação, nenhuma
/// regra: validar é papel do `validate` (Slice 2), não desta conversão.
fn row_to_profile(row: &PresetRow) -> HardwareProfile {
    HardwareProfile {
        schema_version: SCHEMA_VERSION,
        identity: Identity {
            vendor: row.vendor.to_string(),
            model: row.model.to_string(),
            firmware: row.firmware.to_string(),
            firmware_version: row.firmware_version.to_string(),
            serial: None,
        },
        capabilities: Capabilities {
            protocol: row.protocol,
            output_interface: row.output_interface,
            color: row.color,
            supports_discovery: row.supports_discovery,
            supports_metrics: row.supports_metrics,
        },
        limits: Limits {
            pixels_per_universe: row.pixels_per_universe,
            max_pixels: row.max_pixels,
            refresh_hz: row.refresh_hz,
        },
        transport: crate::Transport { mtu_bytes: row.mtu_bytes, heartbeat_ms: row.heartbeat_ms },
        power: Power { voltage_v: row.voltage_v, max_current_a: row.max_current_a },
        calibration: Calibration { gamma: row.gamma, brightness: row.brightness },
    }
}

/// Registro de presets: os embutidos ([`PRESETS`]) mais os que a instalação adicionar.
///
/// Localizar é por `name`. Um registro **não valida** — encadeie `validate` (Slice 2) sobre o
/// profile obtido; a Slice 4 prova que todo preset embutido passa.
#[derive(Clone, Debug, Default)]
pub struct HardwareRegistry {
    rows: Vec<PresetRow>,
}

impl HardwareRegistry {
    /// Registro vazio — útil quando a instalação quer apenas os seus próprios presets.
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Registro com todos os presets embutidos.
    pub fn with_builtin() -> Self {
        Self { rows: PRESETS.to_vec() }
    }

    /// Acrescenta um preset. Um nome já existente é **substituído** (a instalação sobrepõe o
    /// embutido), o que mantém a busca por nome inequívoca.
    pub fn register(&mut self, row: PresetRow) {
        match self.rows.iter_mut().find(|r| r.name == row.name) {
            Some(slot) => *slot = row,
            None => self.rows.push(row),
        }
    }

    /// A linha de preset com este nome.
    pub fn get(&self, name: &str) -> Option<&PresetRow> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// O profile correspondente a este preset.
    pub fn profile(&self, name: &str) -> Option<HardwareProfile> {
        self.get(name).map(row_to_profile)
    }

    /// Nomes registrados, na ordem de registro.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|r| r.name)
    }

    /// Quantos presets estão registrados.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// `true` se nenhum preset está registrado.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{validate, Available, Severity};
    use crate::{ColorFormat, OutputInterface, Protocol};

    const IFACES: &[OutputInterface] = &[OutputInterface::Ethernet, OutputInterface::WiFi];
    const PROTOS: &[Protocol] = &[Protocol::Sacn, Protocol::ArtNet, Protocol::Ddp];

    fn available() -> Available<'static> {
        Available { interfaces: IFACES, protocols: PROTOS }
    }

    // ── Slice 3: registro e localização ───────────────────────────────────────

    #[test]
    fn builtin_registry_exposes_every_table_row() {
        let reg = HardwareRegistry::with_builtin();
        assert_eq!(reg.len(), PRESETS.len());
        assert!(!reg.is_empty());
        for row in PRESETS {
            assert!(reg.get(row.name).is_some(), "preset {} deve ser localizável", row.name);
        }
    }

    #[test]
    fn preset_names_are_unique() {
        let mut seen: Vec<&str> = PRESETS.iter().map(|r| r.name).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "nomes de preset devem ser únicos (busca inequívoca)");
    }

    #[test]
    fn profile_reflects_the_row_it_came_from() {
        let reg = HardwareRegistry::with_builtin();
        let row = reg.get("esp32-poe-wled-ddp").expect("preset embutido");
        let p = reg.profile("esp32-poe-wled-ddp").expect("profile");
        assert_eq!(p.identity.model, row.model);
        assert_eq!(p.capabilities.protocol, row.protocol);
        assert_eq!(p.capabilities.output_interface, row.output_interface);
        assert_eq!(p.limits.max_pixels, row.max_pixels);
        assert_eq!(p.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn unknown_name_is_none_not_a_fallback() {
        let reg = HardwareRegistry::with_builtin();
        assert!(reg.get("no-such-controller").is_none());
        assert!(reg.profile("no-such-controller").is_none(), "sem fallback silencioso");
    }

    #[test]
    fn registering_a_new_name_adds_and_an_existing_name_replaces() {
        let mut reg = HardwareRegistry::new();
        let mut row = PRESETS[0];
        row.name = "mine";
        reg.register(row);
        assert_eq!(reg.len(), 1);

        row.max_pixels = 999;
        reg.register(row); // mesmo nome → substitui, não duplica
        assert_eq!(reg.len(), 1, "nome existente substitui");
        assert_eq!(reg.get("mine").unwrap().max_pixels, 999);
    }

    /// Um controlador novo entra como **dado**: nenhuma variante, nenhum ramo de código.
    #[test]
    fn new_hardware_is_a_row_not_a_code_path() {
        let mut reg = HardwareRegistry::with_builtin();
        let before = reg.len();
        let mut novel = PRESETS[1];
        novel.name = "brand-new-controller-2027";
        novel.vendor = "SomeoneNew";
        reg.register(novel);
        assert_eq!(reg.len(), before + 1);
        let p = reg.profile("brand-new-controller-2027").expect("profile do hardware novo");
        assert_eq!(p.identity.vendor, "SomeoneNew");
    }

    // ── Slice 4: todo preset embutido é validado ──────────────────────────────

    /// Guardian check 6: nenhum preset entra sem passar pelo validador.
    #[test]
    fn every_builtin_preset_validates_without_errors() {
        let reg = HardwareRegistry::with_builtin();
        for row in PRESETS {
            let p = reg.profile(row.name).expect("profile");
            let v = validate(&p, &available());
            assert!(
                !v.has_errors(),
                "preset '{}' tem erro(s) de validação: {:?}",
                row.name,
                v.errors().collect::<Vec<_>>()
            );
        }
    }

    /// Os avisos esperados são exatamente os previstos pelos ADRs — nem a mais, nem a menos.
    #[test]
    fn preset_warnings_are_the_ones_the_adrs_predict() {
        let reg = HardwareRegistry::with_builtin();
        for row in PRESETS {
            let p = reg.profile(row.name).expect("profile");
            let v = validate(&p, &available());
            for w in v.warnings() {
                use crate::validate::Finding;
                match w {
                    // ADR-0005: o preset ESP32 DevKit é WiFi por construção (não tem Ethernet).
                    Finding::WifiNotPermittedLive => {
                        assert_eq!(row.output_interface, OutputInterface::WiFi)
                    }
                    // ADR-0011 + limitação de data type do DDP.
                    Finding::RgbwOverDdpDataType => {
                        assert!(matches!(row.color, ColorFormat::Rgbw(_, _)));
                        assert_eq!(row.protocol, Protocol::Ddp);
                    }
                    other => panic!("aviso inesperado em '{}': {other:?}", row.name),
                }
            }
        }
    }

    /// O preset RGBW respeita o limite que o validador cobra: 128 × 4 canais = 512.
    #[test]
    fn the_rgbw_preset_fits_a_universe_exactly() {
        let reg = HardwareRegistry::with_builtin();
        let row = reg.get("generic-sk6812-rgbw-sacn").expect("preset RGBW");
        assert_eq!(row.color.channels(), 4);
        assert_eq!(row.pixels_per_universe as usize * row.color.channels(), 512);
        assert!(!validate(&reg.profile(row.name).unwrap(), &available()).has_errors());
    }

    /// Sem driver para a interface do preset, a validação **falha** — o preset é dado, a
    /// disponibilidade é injetada, e a combinação é recusada explicitamente.
    #[test]
    fn a_preset_is_rejected_when_its_interface_has_no_driver() {
        let only_ethernet: &[OutputInterface] = &[OutputInterface::Ethernet];
        let av = Available { interfaces: only_ethernet, protocols: PROTOS };
        let reg = HardwareRegistry::with_builtin();
        let wifi_profile = reg.profile("esp32-devkit-wled-artnet").expect("profile");
        let v = validate(&wifi_profile, &av);
        assert!(v.has_errors(), "sem driver WiFi disponível, o preset deve ser recusado");
        assert!(v.findings.iter().any(|f| f.severity() == Severity::Error));
    }
}
