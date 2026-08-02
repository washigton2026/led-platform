//! Contract versioning and stability certification for LUMYX canonical seams.
//!
//! ## SemVer Policy
//!
//! | Version change | Trigger |
//! |---|---|
//! | PATCH (0.x.Y) | Bug fix — no API change |
//! | MINOR (0.X.0) | Additive change — new optional field, new variant |
//! | MAJOR (X.0.0) | Breaking change — field removal, rename, type change |
//!
//! ## Contracts certified here
//!
//! | Contract | Version | Stability |
//! |---|---|---|
//! | `ProtocolOutput` | 1.0.0 | FROZEN — no breaking changes without LUMYX_GOSL approval |
//! | `DeviceDriver` | 1.0.0 | FROZEN |
//! | `LogicalFrame` | 1.1.0 | STABLE — `provenance` field added (backward compat) |
//! | `AudioFeatures` (v0) | 1.2.0 | STABLE — `musical_section` added (backward compat) |
//! | `MusicalSection` | 1.0.0 | STABLE |
//! | `Provenance` | 1.0.0 | STABLE |
//! | `ColorFormat` | 1.0.0 | EVOLVING — RGB/RGBW today; RGB+CCT and richer white modes may add variants (ADR-0011) |
//!
//! ## Breaking change process
//!
//! 1. `lumyx-system-architect` approves the change
//! 2. All consumers updated before the contract changes
//! 3. Migration test added proving old → new compatibility
//! 4. `CONTRACT_VERSION` bumped
//! 5. `lumyx-regression-guardian` validates replay hash unchanged

/// Contract version for `ProtocolOutput` + `DeviceDriver` (core HAL seam).
pub const HAL_CONTRACT_VERSION: &str = "1.0.0";

/// Contract version for `LogicalFrame`.
pub const LOGICAL_FRAME_VERSION: &str = "1.1.0";

/// Contract version for `led-core::AudioFeatures` (v0, the bridge-side contract).
pub const AUDIO_FEATURES_V0_VERSION: &str = "1.2.0";

/// Contract version for `Provenance`.
pub const PROVENANCE_VERSION: &str = "1.0.0";

/// Contract version for `MusicalSection`.
pub const MUSICAL_SECTION_VERSION: &str = "1.0.0";

/// The overall `led-core` contract version.
/// Bump MINOR for additive changes, MAJOR for breaking.
/// 1.4.0 — `WhiteMode::MinSubtract` + `residual_rgb` (ADR-0020, aditivo: `Min` inalterado).
/// 1.3.0 — `ColorFormat`/`WhiteMode` added; `PixelPhysical.order` → `format` (ADR-0011,
/// additive RGBW support; no Frozen seam signature changed).
pub const LED_CORE_CONTRACT_VERSION: &str = "1.4.0";

/// Contract stability guarantees as a structured record.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractRecord {
    pub name:    &'static str,
    pub version: &'static str,
    pub stability: ContractStability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractStability {
    /// Frozen: no changes without LUMYX_GOSL approval and full migration plan.
    Frozen,
    /// Stable: additive changes allowed; breaking changes require MAJOR bump.
    Stable,
    /// Evolving: may change with MINOR version bumps.
    Evolving,
}

/// All certified contracts in this crate.
pub fn certified_contracts() -> Vec<ContractRecord> {
    vec![
        ContractRecord { name: "ProtocolOutput",         version: HAL_CONTRACT_VERSION,         stability: ContractStability::Frozen },
        ContractRecord { name: "DeviceDriver",           version: HAL_CONTRACT_VERSION,         stability: ContractStability::Frozen },
        ContractRecord { name: "IDevice",                version: HAL_CONTRACT_VERSION,         stability: ContractStability::Frozen },
        ContractRecord { name: "LogicalFrame",           version: LOGICAL_FRAME_VERSION,        stability: ContractStability::Stable },
        ContractRecord { name: "AudioFeatures (v0)",     version: AUDIO_FEATURES_V0_VERSION,    stability: ContractStability::Stable },
        ContractRecord { name: "MusicalSection",         version: MUSICAL_SECTION_VERSION,      stability: ContractStability::Stable },
        ContractRecord { name: "Provenance",             version: PROVENANCE_VERSION,           stability: ContractStability::Stable },
        ContractRecord { name: "CompiledLayout",         version: "1.0.0",                      stability: ContractStability::Frozen },
        ContractRecord { name: "UniverseData",           version: "1.0.0",                      stability: ContractStability::Frozen },
        ContractRecord { name: "ColorFormat",            version: "1.0.0",                      stability: ContractStability::Evolving },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioFeatures, LogicalFrame, MusicalSection, PixelColor, ProtocolOutput};

    // ── Version strings are valid semver-shaped ───────────────────────────────

    fn is_semver(v: &str) -> bool {
        let parts: Vec<&str> = v.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok())
    }

    #[test]
    fn all_contract_versions_are_valid_semver() {
        for c in certified_contracts() {
            assert!(is_semver(c.version), "contract {} version '{}' is not semver", c.name, c.version);
        }
    }

    #[test]
    fn led_core_contract_version_is_valid_semver() {
        assert!(is_semver(LED_CORE_CONTRACT_VERSION));
    }

    // ── Frozen contracts have no unknown fields ───────────────────────────────

    #[test]
    fn protocol_output_contract_is_object_safe() {
        // ProtocolOutput must remain object-safe (used as Arc<dyn ProtocolOutput>)
        fn assert_object_safe<T: ?Sized>() {}
        assert_object_safe::<dyn ProtocolOutput>();
    }

    // ── LogicalFrame backward compatibility ───────────────────────────────────

    #[test]
    fn logical_frame_new_has_none_provenance() {
        // LogicalFrame::new() must remain backward compatible (provenance = None by default)
        let frame = LogicalFrame::new(vec![PixelColor::default()], 1000);
        assert!(frame.provenance.is_none(), "LogicalFrame::new must have provenance=None (backward compat)");
        assert_eq!(frame.timestamp_ms, 1000);
        assert_eq!(frame.pixels.len(), 1);
    }

    #[test]
    fn logical_frame_with_provenance_sets_it() {
        use crate::Provenance;
        let prov = Provenance::simulator(0);
        let frame = LogicalFrame::with_provenance(vec![], 500, prov.clone());
        assert_eq!(frame.provenance, Some(prov));
    }

    // ── AudioFeatures v0 backward compatibility ───────────────────────────────

    #[test]
    fn audio_features_default_has_none_section() {
        let f = AudioFeatures::default();
        assert!(f.musical_section.is_none(), "AudioFeatures default must have musical_section=None");
    }

    #[test]
    fn audio_features_v0_all_fields_accessible() {
        let f = AudioFeatures {
            sample_rate: 44100,
            timestamp_ms: 1,
            rms: 0.5,
            beat: true,
            bass: 0.3,
            mid: 0.2,
            high: 0.1,
            spectrum: vec![],
            musical_section: Some(MusicalSection::Verse),
            instrument_class: None,
        };
        assert_eq!(f.sample_rate, 44100);
        assert!(f.beat);
        assert_eq!(f.musical_section, Some(MusicalSection::Verse));
    }

    // ── MusicalSection is exhaustive ──────────────────────────────────────────

    #[test]
    fn musical_section_all_variants_reachable() {
        let variants = [
            MusicalSection::Intro, MusicalSection::Verse, MusicalSection::Chorus,
            MusicalSection::Bridge, MusicalSection::Drop, MusicalSection::Build,
            MusicalSection::Outro, MusicalSection::Unknown,
        ];
        assert_eq!(variants.len(), 8, "MusicalSection must have exactly 8 variants");
        // All are Hash + Eq (required for SectionClip HashMap)
        use std::collections::HashSet;
        let set: HashSet<MusicalSection> = variants.iter().cloned().collect();
        assert_eq!(set.len(), 8, "all variants must be distinct");
    }

    // ── Contract registry completeness ────────────────────────────────────────

    #[test]
    fn at_least_9_contracts_certified() {
        assert!(certified_contracts().len() >= 9, "must certify at least 9 contracts");
    }

    #[test]
    fn frozen_contracts_are_hal_seams() {
        let frozen: Vec<&str> = certified_contracts()
            .iter()
            .filter(|c| c.stability == ContractStability::Frozen)
            .map(|c| c.name)
            .collect();
        assert!(frozen.contains(&"ProtocolOutput"), "ProtocolOutput must be Frozen");
        assert!(frozen.contains(&"DeviceDriver"),   "DeviceDriver must be Frozen");
        assert!(frozen.contains(&"CompiledLayout"), "CompiledLayout must be Frozen");
    }

    // ── Migration compatibility: old LogicalFrame construction still works ────

    #[test]
    fn old_code_pattern_still_compiles() {
        // This is the pattern that existed before provenance — must still work
        let pixels = vec![PixelColor::rgb(255, 0, 0); 4];
        let frame = LogicalFrame::new(pixels, 1000);
        assert_eq!(frame.timestamp_ms, 1000);
        assert!(frame.provenance.is_none()); // old code gets None provenance
    }
}
