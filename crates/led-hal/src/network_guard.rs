//! `NetworkGuard` — WiFi-forbidden enforcement at show-start.
//!
//! ## Hardware Rule
//! > "WiFi is forbidden for live shows. Cable only."
//!
//! This module enforces that rule at the transport layer. Before a live show begins
//! (i.e. before the first frame is sent to real hardware), the guard checks that no
//! Wi-Fi interface is active on the host. If one is found, `check()` returns an error
//! describing which interface is up. The caller decides whether to abort or log a
//! critical warning and proceed.
//!
//! ## Design
//! - `NetworkGuard` is a trait: swap real enforcement for `PermissiveGuard` in tests
//!   and in environments where the check is not applicable (e.g. headless CI, Simulator).
//! - `WifiBlockGuard` is the production implementation. It uses platform-specific probes
//!   to detect active Wi-Fi interfaces.
//! - Zero dependencies beyond `std`. No async, no allocations on the hot path (check
//!   is only called at show-start, not per-frame).

use std::fmt;

// ── Error type ────────────────────────────────────────────────────────────────

/// Returned when a network policy violation is detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkPolicyError {
    /// One or more Wi-Fi interfaces were found active.
    ///
    /// The `interfaces` field lists every active wireless interface by name
    /// (e.g. `["en0"]` on macOS, `["wlan0"]` on Linux).
    WifiActive { interfaces: Vec<String> },

    /// The network state could not be determined (probe command failed or
    /// was unavailable on this platform). The show is allowed to proceed but
    /// this should be surfaced as a WARNING to the operator.
    ProbeUnavailable { reason: String },
}

impl fmt::Display for NetworkPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkPolicyError::WifiActive { interfaces } => write!(
                f,
                "[LUMYX CRITICAL] WiFi active on interface(s): {}. \
                 Disable WiFi before starting a live show (Hardware Rule).",
                interfaces.join(", ")
            ),
            NetworkPolicyError::ProbeUnavailable { reason } => write!(
                f,
                "[LUMYX WARNING] WiFi check unavailable ({reason}). \
                 Cannot enforce WiFi-forbidden rule — verify manually."
            ),
        }
    }
}

impl std::error::Error for NetworkPolicyError {}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Checks that network conditions satisfy the LUMYX hardware policy before show-start.
///
/// Call `check()` once, before the first frame is sent to real hardware. In test
/// environments, use `PermissiveGuard` to bypass the check. In simulators, use
/// `PermissiveGuard`. In production, use `WifiBlockGuard`.
pub trait NetworkGuard: Send + Sync {
    /// Returns `Ok(())` if the network policy is satisfied, or an error describing
    /// the violation.
    ///
    /// This is called once at show-start, NOT per-frame. It may spawn a process to
    /// inspect network state.
    fn check(&self) -> Result<(), NetworkPolicyError>;

    /// Human-readable name of this guard (for logging).
    fn name(&self) -> &'static str;
}

// ── PermissiveGuard (always passes — for tests + simulator) ──────────────────

/// A `NetworkGuard` that always allows the show to proceed.
///
/// Use in:
/// - Unit and integration tests
/// - CI environments (no real hardware)
/// - `SimulatorDevice`-only shows
/// - Platforms where network inspection is not supported
pub struct PermissiveGuard;

impl NetworkGuard for PermissiveGuard {
    fn check(&self) -> Result<(), NetworkPolicyError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "PermissiveGuard (no enforcement)"
    }
}

// ── WifiBlockGuard (production enforcement) ───────────────────────────────────

/// A `NetworkGuard` that fails if any Wi-Fi interface is active.
///
/// ## Platform support
/// | Platform | Probe method |
/// |---|---|
/// | macOS | `networksetup -listallhardwareports` + `ifconfig <iface>` status |
/// | Linux | `/sys/class/net/wl*/operstate` |
/// | Other | `ProbeUnavailable` warning (allows show to proceed) |
pub struct WifiBlockGuard;

impl NetworkGuard for WifiBlockGuard {
    fn check(&self) -> Result<(), NetworkPolicyError> {
        probe_wifi()
    }

    fn name(&self) -> &'static str {
        "WifiBlockGuard (WiFi-forbidden enforcement)"
    }
}

// ── Platform probes ───────────────────────────────────────────────────────────

fn probe_wifi() -> Result<(), NetworkPolicyError> {
    #[cfg(target_os = "macos")]
    return probe_macos();

    #[cfg(target_os = "linux")]
    return probe_linux();

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err(NetworkPolicyError::ProbeUnavailable {
        reason: format!(
            "unsupported platform '{}' — WiFi check not implemented",
            std::env::consts::OS
        ),
    });
}

/// macOS: enumerate hardware ports via `networksetup`, find Wi-Fi devices,
/// then use `ifconfig` to check if the interface is UP and RUNNING.
#[cfg(target_os = "macos")]
fn probe_macos() -> Result<(), NetworkPolicyError> {
    use std::process::Command;

    // Step 1: list all hardware ports to find Wi-Fi interface names
    let ports_out = Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .map_err(|e| NetworkPolicyError::ProbeUnavailable {
            reason: format!("networksetup failed: {e}"),
        })?;

    if !ports_out.status.success() {
        return Err(NetworkPolicyError::ProbeUnavailable {
            reason: "networksetup -listallhardwareports returned non-zero".into(),
        });
    }

    let ports_str = String::from_utf8_lossy(&ports_out.stdout);
    let wifi_ifaces = parse_macos_wifi_interfaces(&ports_str);

    if wifi_ifaces.is_empty() {
        // No Wi-Fi hardware found at all — policy satisfied
        return Ok(());
    }

    // Step 2: for each Wi-Fi interface, check if it's UP and RUNNING via ifconfig
    let mut active: Vec<String> = Vec::new();
    for iface in &wifi_ifaces {
        if is_interface_active_macos(iface) {
            active.push(iface.clone());
        }
    }

    if active.is_empty() {
        Ok(())
    } else {
        Err(NetworkPolicyError::WifiActive { interfaces: active })
    }
}

/// Parse `networksetup -listallhardwareports` output to extract Wi-Fi interface names.
/// A Wi-Fi port block looks like:
/// ```text
/// Hardware Port: Wi-Fi
/// Device: en0
/// Ethernet Address: xx:xx:xx:xx:xx:xx
/// ```
#[cfg(target_os = "macos")]
fn parse_macos_wifi_interfaces(output: &str) -> Vec<String> {
    let mut ifaces = Vec::new();
    let mut in_wifi_block = false;
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Hardware Port:") {
            // Check if this port is a Wi-Fi port
            let port_name = line.trim_start_matches("Hardware Port:").trim();
            in_wifi_block = port_name.eq_ignore_ascii_case("wi-fi")
                || port_name.eq_ignore_ascii_case("airport");
        } else if in_wifi_block && line.starts_with("Device:") {
            let device = line.trim_start_matches("Device:").trim().to_string();
            if !device.is_empty() {
                ifaces.push(device);
            }
            in_wifi_block = false;
        }
    }
    ifaces
}

/// Check if a macOS network interface is active (UP + RUNNING) via `ifconfig`.
#[cfg(target_os = "macos")]
fn is_interface_active_macos(iface: &str) -> bool {
    use std::process::Command;
    let out = Command::new("/sbin/ifconfig")
        .arg(iface)
        .output()
        .ok();
    let Some(out) = out else { return false };
    if !out.status.success() { return false }
    let text = String::from_utf8_lossy(&out.stdout);
    // ifconfig shows "status: active" when the interface is connected
    text.contains("status: active")
}

/// Linux: check `/sys/class/net/` for wireless interfaces whose `operstate` is `up`.
/// Wireless interfaces typically have a `wireless/` or `phy80211/` subdirectory.
#[cfg(target_os = "linux")]
fn probe_linux() -> Result<(), NetworkPolicyError> {
    use std::fs;
    use std::path::Path;

    let net_path = Path::new("/sys/class/net");
    if !net_path.exists() {
        return Err(NetworkPolicyError::ProbeUnavailable {
            reason: "/sys/class/net not found".into(),
        });
    }

    let mut active: Vec<String> = Vec::new();

    let entries = fs::read_dir(net_path).map_err(|e| NetworkPolicyError::ProbeUnavailable {
        reason: format!("read_dir /sys/class/net: {e}"),
    })?;

    for entry in entries.flatten() {
        let iface_path = entry.path();
        let iface_name = entry.file_name().to_string_lossy().to_string();

        // Wireless interfaces have a `wireless/` or `phy80211/` subdirectory
        let is_wireless = iface_path.join("wireless").exists()
            || iface_path.join("phy80211").exists();

        if !is_wireless {
            continue;
        }

        // Check operstate
        let operstate_path = iface_path.join("operstate");
        if let Ok(state) = fs::read_to_string(&operstate_path) {
            if state.trim() == "up" {
                active.push(iface_name);
            }
        }
    }

    if active.is_empty() {
        Ok(())
    } else {
        Err(NetworkPolicyError::WifiActive { interfaces: active })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PermissiveGuard ───────────────────────────────────────────────────────

    #[test]
    fn permissive_guard_always_passes() {
        let g = PermissiveGuard;
        assert!(g.check().is_ok(), "PermissiveGuard must always pass");
        assert_eq!(g.name(), "PermissiveGuard (no enforcement)");
    }

    // ── NetworkPolicyError display ────────────────────────────────────────────

    #[test]
    fn error_wifi_active_displays_interface_names() {
        let err = NetworkPolicyError::WifiActive {
            interfaces: vec!["en0".into(), "en1".into()],
        };
        let msg = err.to_string();
        assert!(msg.contains("en0"), "must name the interface: {msg}");
        assert!(msg.contains("en1"), "must name both interfaces: {msg}");
        assert!(msg.contains("CRITICAL"), "must be flagged as critical: {msg}");
        assert!(msg.contains("WiFi"), "must mention WiFi: {msg}");
    }

    #[test]
    fn error_probe_unavailable_displays_reason() {
        let err = NetworkPolicyError::ProbeUnavailable {
            reason: "test reason".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("WARNING"), "must be a warning: {msg}");
        assert!(msg.contains("test reason"), "must include reason: {msg}");
    }

    #[test]
    fn error_wifi_active_is_not_probe_unavailable() {
        let wifi = NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] };
        let probe = NetworkPolicyError::ProbeUnavailable { reason: "x".into() };
        assert_ne!(wifi, probe);
    }

    // ── WifiBlockGuard (macOS only) ───────────────────────────────────────────

    /// On macOS CI (no Wi-Fi active or no hardware), WifiBlockGuard must either pass
    /// or return a typed error — it must NEVER panic.
    #[test]
    #[cfg(target_os = "macos")]
    fn wifi_block_guard_does_not_panic_on_macos() {
        let g = WifiBlockGuard;
        let result = g.check();
        // We don't assert Ok/Err because the CI host may or may not have Wi-Fi.
        // We assert it returns a typed result without panicking.
        match &result {
            Ok(()) => { /* Wi-Fi not active — policy satisfied */ }
            Err(NetworkPolicyError::WifiActive { interfaces }) => {
                assert!(!interfaces.is_empty(), "WifiActive must name at least one interface");
            }
            Err(NetworkPolicyError::ProbeUnavailable { reason }) => {
                assert!(!reason.is_empty(), "ProbeUnavailable must include a reason");
            }
        }
    }

    /// Parsing: a typical macOS networksetup output with a Wi-Fi block
    #[test]
    #[cfg(target_os = "macos")]
    fn parse_macos_wifi_interfaces_finds_wifi_block() {
        let output = "\
Hardware Port: Thunderbolt 1
Device: en1
Ethernet Address: aa:bb:cc:dd:ee:ff

Hardware Port: Wi-Fi
Device: en0
Ethernet Address: 11:22:33:44:55:66

Hardware Port: Bluetooth PAN
Device: en3
Ethernet Address: 77:88:99:aa:bb:cc
";
        let ifaces = parse_macos_wifi_interfaces(output);
        assert_eq!(ifaces, vec!["en0"], "must extract only the Wi-Fi device");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn parse_macos_wifi_interfaces_handles_airport_name() {
        let output = "\
Hardware Port: AirPort
Device: en0
Ethernet Address: 11:22:33:44:55:66
";
        let ifaces = parse_macos_wifi_interfaces(output);
        assert_eq!(ifaces, vec!["en0"], "must also recognise 'AirPort' port name");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn parse_macos_wifi_interfaces_empty_when_no_wifi_hardware() {
        let output = "\
Hardware Port: Thunderbolt 1
Device: en1
Ethernet Address: aa:bb:cc:dd:ee:ff
";
        let ifaces = parse_macos_wifi_interfaces(output);
        assert!(ifaces.is_empty(), "no Wi-Fi hardware → empty list");
    }

    // ── Linux probe (unit test without real /sys) ─────────────────────────────

    /// On non-macOS, non-Linux platforms (or Linux without /sys), the guard must
    /// return ProbeUnavailable, not panic.
    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn wifi_block_guard_probe_unavailable_on_unsupported_platform() {
        let g = WifiBlockGuard;
        let result = g.check();
        assert!(
            matches!(result, Err(NetworkPolicyError::ProbeUnavailable { .. })),
            "unsupported platform must return ProbeUnavailable, got {result:?}"
        );
    }

    // ── NetworkGuard as trait object ──────────────────────────────────────────

    #[test]
    fn network_guard_is_object_safe() {
        // If this compiles, the trait is object-safe.
        let guard: Box<dyn NetworkGuard> = Box::new(PermissiveGuard);
        assert!(guard.check().is_ok());
    }

    #[test]
    fn network_guard_name_is_accessible_on_trait_object() {
        let guard: Box<dyn NetworkGuard> = Box::new(PermissiveGuard);
        assert!(!guard.name().is_empty());
    }
}
