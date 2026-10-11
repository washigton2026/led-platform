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

    /// There is **no probe for this platform** (neither macOS nor Linux). The show is
    /// allowed to proceed but this should be surfaced as a WARNING to the operator —
    /// this is the case ADR-0005 («Consequências») and ADR-0018 decided as non-fatal.
    ///
    /// It is **not** used when a probe exists and fails: that is [`Self::ProbeFailed`].
    ProbeUnavailable { reason: String },

    /// The platform **is supported**, but its probe failed to run (TD-029). Nothing can be
    /// concluded — neither "WiFi is on" nor "WiFi is off" — and, unlike an unsupported
    /// platform, this case was never decided as non-fatal: the caller must treat it as
    /// a refusal unless the operator explicitly affirms otherwise.
    ProbeFailed { probe: &'static str, error: String },
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
            NetworkPolicyError::ProbeFailed { probe, error } => write!(
                f,
                "[LUMYX CRITICAL] WiFi probe `{probe}` failed ({error}). \
                 The WiFi-forbidden rule could not be verified on a supported platform."
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
///
/// On macOS and Linux a probe that **fails to run** (`networksetup`, `/sys/class/net`) returns
/// `ProbeFailed` (TD-029). NOT yet the per-interface reads: an `ifconfig` that fails, an unreadable
/// `operstate` or an I/O error inside the `read_dir` still count the interface as INACTIVE —
/// fail-open, open in TD-029 (residuals (b) and (d)).
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

// ── Estado de uma interface WiFi (TD-029, R8.5b — decisões do operador (a) e (b)) ─────────────
//
// Fail-closed: só estados RECONHECIDOS como «desligado» deixam passar. Tudo o que não se consegue
// ler, ou não se reconhece, é INDETERMINADO — a classe «sonda falhada» (`ProbeFailed`): bloqueia por
// omissão, e o operador pode afirmar com `--assume-no-wifi` (D2). Uma interface comprovadamente
// ATIVA é `WifiActive`, que o override nunca desbloqueia (ADR-0005).

/// O que a sonda conseguiu saber de UMA interface WiFi.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EstadoWifi {
    /// Comprovadamente ativa.
    Ativa,
    /// Comprovadamente desligada.
    Desligada,
    /// Não se sabe — tratada como possivelmente ligada.
    Indeterminada,
}

/// Junta os estados das interfaces num veredito: ativa vence indeterminada, que vence desligada.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn veredito_wifi(
    sonda: &'static str,
    estados: Vec<(String, EstadoWifi, String)>,
) -> Result<(), NetworkPolicyError> {
    let ativas: Vec<String> =
        estados.iter().filter(|(_, e, _)| *e == EstadoWifi::Ativa).map(|(n, _, _)| n.clone()).collect();
    if !ativas.is_empty() {
        return Err(NetworkPolicyError::WifiActive { interfaces: ativas });
    }
    let indeterminadas: Vec<String> = estados
        .iter()
        .filter(|(_, e, _)| *e == EstadoWifi::Indeterminada)
        .map(|(n, _, porque)| format!("{n}={porque}"))
        .collect();
    if !indeterminadas.is_empty() {
        return Err(NetworkPolicyError::ProbeFailed {
            probe: sonda,
            error: format!("estado WiFi indeterminado: {}", indeterminadas.join(", ")),
        });
    }
    Ok(())
}

/// macOS: a sonda real — `probe_macos_com` com o `networksetup` e o `ifconfig` do sistema.
#[cfg(target_os = "macos")]
fn probe_macos() -> Result<(), NetworkPolicyError> {
    probe_macos_com(networksetup_real, ifconfig_real)
}

/// macOS: `networksetup -listallhardwareports` (a lista de portas, ou `ProbeFailed`).
#[cfg(target_os = "macos")]
fn networksetup_real() -> Result<String, NetworkPolicyError> {
    let ports_out = std::process::Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .map_err(|e| NetworkPolicyError::ProbeFailed {
            probe: "networksetup -listallhardwareports",
            error: e.to_string(),
        })?;
    if !ports_out.status.success() {
        return Err(NetworkPolicyError::ProbeFailed {
            probe: "networksetup -listallhardwareports",
            error: format!("exit status {}", ports_out.status),
        });
    }
    Ok(String::from_utf8_lossy(&ports_out.stdout).to_string())
}

/// macOS: `ifconfig <iface>` — `None` se não correu; `Some((exit ok, stdout))` caso contrário.
#[cfg(target_os = "macos")]
fn ifconfig_real(iface: &str) -> Option<(bool, String)> {
    let o = std::process::Command::new("/sbin/ifconfig").arg(iface).output().ok()?;
    Some((o.status.success(), String::from_utf8_lossy(&o.stdout).to_string()))
}

/// macOS: a regra inteira, com as duas sondas por PARÂMETRO (o padrão do `run_com`; TD-029 R8.5c).
/// Para cada porta Wi-Fi, o estado do `ifconfig`: falhar, sair com erro ou não trazer `status:` é
/// INDETERMINADO — nunca «inactive», e nunca filtrado (H13/H14b da ronda 14).
#[cfg(any(test, target_os = "macos"))]
fn probe_macos_com(
    listar_portas: impl FnOnce() -> Result<String, NetworkPolicyError>,
    ifconfig: impl Fn(&str) -> Option<(bool, String)>,
) -> Result<(), NetworkPolicyError> {
    let portas = listar_portas()?;
    let estados = parse_macos_wifi_interfaces(&portas)
        .into_iter()
        .map(|iface| {
            let saida = ifconfig(&iface);
            let (estado, porque) = estado_ifconfig(saida.as_ref().map(|(ok, t)| (*ok, t.as_str())));
            (iface, estado, porque)
        })
        .collect();
    veredito_wifi("ifconfig", estados)
}

/// Parse `networksetup -listallhardwareports` output to extract Wi-Fi interface names.
/// A Wi-Fi port block looks like:
/// ```text
/// Hardware Port: Wi-Fi
/// Device: en0
/// Ethernet Address: xx:xx:xx:xx:xx:xx
/// ```
#[cfg(any(test, target_os = "macos"))]
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

/// macOS: o estado de uma interface a partir do resultado do `ifconfig <iface>`.
/// `None` = o `ifconfig` não correu; `Some((ok, stdout))` caso contrário.
#[cfg(any(test, target_os = "macos"))]
fn estado_ifconfig(saida: Option<(bool, &str)>) -> (EstadoWifi, String) {
    let Some((ok, texto)) = saida else {
        return (EstadoWifi::Indeterminada, "ifconfig nao correu".into());
    };
    if !ok {
        return (EstadoWifi::Indeterminada, "ifconfig com exit != 0".into());
    }
    for linha in texto.lines() {
        match linha.trim() {
            "status: active" => return (EstadoWifi::Ativa, "status: active".into()),
            "status: inactive" => return (EstadoWifi::Desligada, "status: inactive".into()),
            _ => {}
        }
    }
    (EstadoWifi::Indeterminada, "sem linha status: reconhecida".into())
}

/// Linux: os estados de `operstate` RECONHECIDOS como desligado. Só `up` é ativo.
#[cfg(any(test, target_os = "linux"))]
const OPERSTATE_DESLIGADO: [&str; 3] = ["down", "lowerlayerdown", "notpresent"];

/// Linux: o estado de uma interface WiFi a partir do `operstate` lido (`None` = ilegível).
#[cfg(any(test, target_os = "linux"))]
fn estado_operstate(lido: Option<&str>) -> (EstadoWifi, String) {
    match lido.map(str::trim) {
        None => (EstadoWifi::Indeterminada, "operstate ilegivel".into()),
        Some("up") => (EstadoWifi::Ativa, "up".into()),
        Some(s) if OPERSTATE_DESLIGADO.contains(&s) => (EstadoWifi::Desligada, s.into()),
        Some(s) => (EstadoWifi::Indeterminada, s.into()),
    }
}

/// Linux: a interface é WiFi? Basta UM sinal: `wireless/`, `phy80211/`, `DEVTYPE=wlan` no
/// `uevent`, ou nome começado por `wl` (wlan*, wlp*, wlx*). Um `uevent` ilegível numa interface sem
/// nenhum outro sinal é DÚVIDA — devolvida como `None`, e quem chama trata-a como indeterminada.
#[cfg(any(test, target_os = "linux"))]
fn e_wifi_linux(nome: &str, tem_wireless: bool, tem_phy80211: bool, uevent: Option<&str>) -> Option<bool> {
    if tem_wireless || tem_phy80211 || nome.starts_with("wl") {
        return Some(true);
    }
    uevent.map(|u| u.lines().any(|l| l.trim() == "DEVTYPE=wlan"))
}

/// Linux: o que se leu de UMA entrada de `/sys/class/net`.
#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum EntradaSysfs {
    /// Não é diretório (ex.: `bonding_masters`, um ficheiro do módulo bonding): não é interface.
    NaoDiretorio,
    /// Não se conseguiu saber o que é (metadata ilegível, ex.: symlink pendurado) — fica INDETERMINADA.
    Ilegivel { nome: String, porque: String },
    /// Um diretório de interface, com os sinais já lidos.
    Interface { nome: String, wireless: bool, phy80211: bool, uevent: Option<String>, operstate: Option<String> },
}

/// Linux: lê uma entrada. Só DIRETÓRIOS (seguindo o symlink — em /sys/class/net cada interface é um
/// symlink para o diretório do dispositivo) contam como interface (R8.5c, `bonding_masters`).
#[cfg(any(test, target_os = "linux"))]
fn ler_entrada(p: &std::path::Path, nome: String) -> EntradaSysfs {
    use std::fs;
    match fs::metadata(p) {
        Err(_) => EntradaSysfs::Ilegivel { nome, porque: "metadata ilegivel".into() },
        Ok(m) if !m.is_dir() => EntradaSysfs::NaoDiretorio,
        Ok(_) => EntradaSysfs::Interface {
            wireless: p.join("wireless").exists(),
            phy80211: p.join("phy80211").exists(),
            uevent: fs::read_to_string(p.join("uevent")).ok(),
            operstate: fs::read_to_string(p.join("operstate")).ok(),
            nome,
        },
    }
}

/// Linux: a regra sobre as entradas já lidas — as leituras chegam por PARÂMETRO, por isso uma entrada
/// de `read_dir` ilegível é testável sem injeção no binário (H11 da ronda 14).
#[cfg(any(test, target_os = "linux"))]
fn classificar_entradas(
    sonda: &'static str,
    entradas: impl Iterator<Item = std::io::Result<EntradaSysfs>>,
) -> Result<(), NetworkPolicyError> {
    let mut estados = Vec::new();
    for entrada in entradas {
        match entrada {
            // Uma entrada ilegível não pode desaparecer em silêncio: pode ser a interface WiFi.
            Err(_) => estados.push(("?".to_string(), EstadoWifi::Indeterminada, "entrada de read_dir ilegivel".to_string())),
            Ok(EntradaSysfs::NaoDiretorio) => continue,
            Ok(EntradaSysfs::Ilegivel { nome, porque }) => estados.push((nome, EstadoWifi::Indeterminada, porque)),
            Ok(EntradaSysfs::Interface { nome, wireless, phy80211, uevent, operstate }) => {
                match e_wifi_linux(&nome, wireless, phy80211, uevent.as_deref()) {
                    Some(false) => continue,
                    None => estados.push((nome, EstadoWifi::Indeterminada, "uevent ilegivel".to_string())),
                    Some(true) => {
                        let (estado, porque) = estado_operstate(operstate.as_deref());
                        estados.push((nome, estado, porque));
                    }
                }
            }
        }
    }
    veredito_wifi(sonda, estados)
}

/// Linux: a regra inteira sobre um diretório com o formato de `/sys/class/net`. O `probe_linux` passa a
/// raiz real; os testes passam um diretório temporário.
#[cfg(any(test, target_os = "linux"))]
fn probe_sysfs(raiz: &std::path::Path) -> Result<(), NetworkPolicyError> {
    const SONDA: &str = "sysfs /sys/class/net";

    if !raiz.exists() {
        return Err(NetworkPolicyError::ProbeFailed { probe: SONDA, error: format!("{} not found", raiz.display()) });
    }
    let entradas = std::fs::read_dir(raiz).map_err(|e| NetworkPolicyError::ProbeFailed {
        probe: SONDA,
        error: format!("read_dir: {e}"),
    })?;
    classificar_entradas(
        SONDA,
        entradas.map(|r| r.map(|e| ler_entrada(&e.path(), e.file_name().to_string_lossy().to_string()))),
    )
}

/// Linux: a raiz real das interfaces de rede.
#[cfg(any(test, target_os = "linux"))]
const RAIZ_SYSFS: &str = "/sys/class/net";

/// Linux: a sonda real — `probe_sysfs` sobre `RAIZ_SYSFS`, sem nada à volta (L2/L3/L6 da ronda 14).
#[cfg(target_os = "linux")]
fn probe_linux() -> Result<(), NetworkPolicyError> {
    probe_sysfs(std::path::Path::new(RAIZ_SYSFS))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TD-029 R8.5b: estados por tabela (fail-closed) ───────────────────────

    #[test]
    fn operstate_por_tabela_so_up_e_ativo_e_so_tres_sao_desligados() {
        use EstadoWifi::*;
        let tabela: [(Option<&str>, EstadoWifi); 13] = [
            (Some("up"), Ativa),
            (Some("up\n"), Ativa),
            (Some("down"), Desligada),
            (Some("lowerlayerdown"), Desligada),
            (Some("notpresent"), Desligada),
            (Some("dormant"), Indeterminada),
            (Some("unknown"), Indeterminada),
            (Some("testing"), Indeterminada),
            (Some("valor-inventado-pelo-kernel-2031"), Indeterminada),
            (Some(""), Indeterminada),
            (Some("UP"), Indeterminada),
            (Some("down-ish"), Indeterminada),
            (None, Indeterminada),
        ];
        for (lido, esperado) in tabela {
            assert_eq!(estado_operstate(lido).0, esperado, "operstate {lido:?}");
        }
    }

    #[test]
    fn deteccao_de_wifi_quatro_sinais_isolados_e_combinados() {
        // (nome, wireless/, phy80211/, uevent, esperado)
        type Linha<'a> = (&'a str, bool, bool, Option<&'a str>, Option<bool>);
        let tabela: [Linha; 14] = [
            ("eth0", true, false, Some("INTERFACE=eth0\n"), Some(true)),
            ("eth0", false, true, Some("INTERFACE=eth0\n"), Some(true)),
            ("eth0", false, false, Some("DEVTYPE=wlan\nINTERFACE=eth0\n"), Some(true)),
            ("wlan0", false, false, Some("INTERFACE=wlan0\n"), Some(true)),
            ("wlp2s0", false, false, Some(""), Some(true)),
            ("wlx00c0ca123456", false, false, None, Some(true)),
            ("wlan0", true, true, Some("DEVTYPE=wlan\n"), Some(true)),
            ("enp3s0", true, false, Some("DEVTYPE=wlan\n"), Some(true)),
            ("eth0", false, false, Some("INTERFACE=eth0\n"), Some(false)),
            ("en0", false, false, Some("INTERFACE=en0\n"), Some(false)),
            ("enp3s0", false, false, Some("DEVTYPE=ethernet\n"), Some(false)),
            ("eth1", false, false, Some("DEVTYPE=wlanx\n"), Some(false)),
            ("lo", false, false, Some("INTERFACE=lo\n"), Some(false)),
            ("eth0", false, false, None, None),
        ];
        for (nome, w, p, u, esperado) in tabela {
            assert_eq!(e_wifi_linux(nome, w, p, u), esperado, "{nome} wireless={w} phy={p} uevent={u:?}");
        }
    }

    /// (nome, wireless/, phy80211/, uevent, operstate) de uma interface no sysfs de teste.
    type Iface<'a> = (&'a str, bool, bool, Option<&'a str>, Option<&'a str>);

    /// Um diretório com o formato de /sys/class/net, construído a partir de uma lista de interfaces.
    struct Sysfs(std::path::PathBuf);
    impl Sysfs {
        fn novo(ifaces: &[Iface]) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let raiz = std::env::temp_dir()
                .join(format!("lumyx-sysfs-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = std::fs::remove_dir_all(&raiz);
            for (nome, wireless, phy, uevent, operstate) in ifaces {
                let d = raiz.join(nome);
                std::fs::create_dir_all(&d).unwrap();
                if *wireless { std::fs::create_dir_all(d.join("wireless")).unwrap(); }
                if *phy { std::fs::create_dir_all(d.join("phy80211")).unwrap(); }
                if let Some(u) = uevent { std::fs::write(d.join("uevent"), u).unwrap(); }
                if let Some(o) = operstate { std::fs::write(d.join("operstate"), o).unwrap(); }
            }
            std::fs::create_dir_all(&raiz).unwrap();
            Sysfs(raiz)
        }
    }
    impl Drop for Sysfs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn classe(r: &Result<(), NetworkPolicyError>) -> &'static str {
        match r {
            Ok(()) => "ok",
            Err(NetworkPolicyError::WifiActive { .. }) => "wifi_ativo",
            Err(NetworkPolicyError::ProbeFailed { .. }) => "sonda_falhada",
            Err(NetworkPolicyError::ProbeUnavailable { .. }) => "nao_suportada",
        }
    }

    #[test]
    fn probe_sysfs_por_operstate_numa_interface_wifi_real_em_disco() {
        let eth = ("eth0", false, false, Some("INTERFACE=eth0\n"), Some("up\n"));
        for (operstate, esperado) in [
            (Some("up\n"), "wifi_ativo"),
            (Some("down\n"), "ok"),
            (Some("lowerlayerdown\n"), "ok"),
            (Some("notpresent\n"), "ok"),
            (Some("dormant\n"), "sonda_falhada"),
            (Some("unknown\n"), "sonda_falhada"),
            (Some("testing\n"), "sonda_falhada"),
            (Some("inventado\n"), "sonda_falhada"),
            (None, "sonda_falhada"),
        ] {
            let fs = Sysfs::novo(&[eth, ("wlan0", true, true, Some("DEVTYPE=wlan\n"), operstate)]);
            let r = probe_sysfs(&fs.0);
            assert_eq!(classe(&r), esperado, "wlan0 operstate={operstate:?} → {r:?}");
            if esperado == "sonda_falhada" {
                let Err(NetworkPolicyError::ProbeFailed { error, .. }) = &r else { unreachable!() };
                assert!(error.contains("wlan0"), "a interface indeterminada é nomeada: {error}");
            }
        }
    }

    #[test]
    fn probe_sysfs_cada_sinal_isolado_torna_a_interface_wifi() {
        // Interface com nome de cabo e operstate dormant: só é bloqueada se for vista como WiFi.
        for (nome, w, p, u, esperado) in [
            ("eth0", true, false, Some("INTERFACE=eth0\n"), "sonda_falhada"),
            ("eth0", false, true, Some("INTERFACE=eth0\n"), "sonda_falhada"),
            ("eth0", false, false, Some("DEVTYPE=wlan\n"), "sonda_falhada"),
            ("wlx00c0ca123456", false, false, Some("INTERFACE=x\n"), "sonda_falhada"),
            ("eth0", false, false, Some("INTERFACE=eth0\n"), "ok"),
            ("eth0", false, false, None, "sonda_falhada"), // dúvida → indeterminada
        ] {
            let fs = Sysfs::novo(&[(nome, w, p, u, Some("dormant\n"))]);
            assert_eq!(classe(&probe_sysfs(&fs.0)), esperado, "{nome} w={w} p={p} u={u:?}");
        }
    }

    #[test]
    fn probe_sysfs_ativa_vence_indeterminada_e_cabo_ativo_nao_conta() {
        let fs = Sysfs::novo(&[
            ("wlan0", true, false, Some(""), Some("dormant\n")),
            ("wlp2s0", false, true, Some(""), Some("up\n")),
            ("eth0", false, false, Some("INTERFACE=eth0\n"), Some("up\n")),
        ]);
        match probe_sysfs(&fs.0) {
            Err(NetworkPolicyError::WifiActive { interfaces }) => assert_eq!(interfaces, vec!["wlp2s0".to_string()]),
            outro => panic!("WiFi comprovadamente ativo tem de ser WifiActive (o override nunca o desbloqueia): {outro:?}"),
        }
        let fs = Sysfs::novo(&[("eth0", false, false, Some("INTERFACE=eth0\n"), Some("up\n"))]);
        assert_eq!(classe(&probe_sysfs(&fs.0)), "ok", "só cabo, ativo: não há WiFi");
        let ausente = std::env::temp_dir().join("lumyx-sysfs-que-nao-existe");
        assert_eq!(classe(&probe_sysfs(&ausente)), "sonda_falhada", "raiz ausente é sonda falhada (D3)");
    }

    #[test]
    fn ifconfig_por_tabela_erro_nunca_e_desligado() {
        use EstadoWifi::*;
        let ativo = "en0: flags=8863<UP>\n\tstatus: active\n";
        let inativo = "en0: flags=8822<BROADCAST>\n\tstatus: inactive\n";
        let tabela: [(Option<(bool, &str)>, EstadoWifi); 7] = [
            (Some((true, ativo)), Ativa),
            (Some((true, inativo)), Desligada),
            (Some((true, "en0: flags=8863<UP>\n")), Indeterminada),
            (Some((true, "\tstatus: activeX\n")), Indeterminada),
            (Some((false, ativo)), Indeterminada),
            (Some((false, inativo)), Indeterminada),
            (None, Indeterminada),
        ];
        for (saida, esperado) in tabela {
            assert_eq!(estado_ifconfig(saida).0, esperado, "{saida:?}");
        }
    }

    #[test]
    fn veredito_ativa_vence_indeterminada_que_vence_desligada() {
        use EstadoWifi::*;
        let e = |v: &[(&str, EstadoWifi)]| {
            classe(&veredito_wifi("t", v.iter().map(|(n, s)| (n.to_string(), *s, "x".to_string())).collect()))
        };
        assert_eq!(e(&[]), "ok");
        assert_eq!(e(&[("a", Desligada)]), "ok");
        assert_eq!(e(&[("a", Desligada), ("b", Indeterminada)]), "sonda_falhada");
        assert_eq!(e(&[("a", Indeterminada), ("b", Ativa)]), "wifi_ativo");
    }

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

    /// TD-029: a failed probe on a supported platform is CRITICAL, names the probe and
    /// the error, and is distinct from "no probe for this platform".
    #[test]
    fn error_probe_failed_is_critical_and_names_probe_and_error() {
        let err = NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "read_dir: permission denied".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("CRITICAL"), "must be critical, not a warning: {msg}");
        assert!(msg.contains("sysfs /sys/class/net"), "must name the probe: {msg}");
        assert!(msg.contains("permission denied"), "must carry the error: {msg}");
        assert_ne!(err, NetworkPolicyError::ProbeUnavailable { reason: "x".into() });
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
                panic!("macOS is a SUPPORTED platform: it must never report ProbeUnavailable ({reason})");
            }
            Err(NetworkPolicyError::ProbeFailed { probe, error }) => {
                assert!(!probe.is_empty() && !error.is_empty(), "ProbeFailed must name probe and error");
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

    // ── TD-029 R8.5c: um teste por sobrevivente da ronda 14 (o nome diz o mutante que mata) ──────
    //
    // Cada um afirma o VEREDITO (variante + Display exato). A frase do journal para estes mesmos valores é
    // afirmada no led-daemon-bin (`r85c_journal_por_sobrevivente`), pela entrada da tabela `FRASES`.

    /// O fonte de PRODUÇÃO deste ficheiro (até ao `mod tests`), sem comentários `//` nem espaços.
    fn producao_normalizada() -> String {
        let f = include_str!("network_guard.rs");
        let producao = &f[..f.find("#[cfg(test)]\nmod tests").expect("mod tests")];
        normaliza(producao)
    }
    fn normaliza(t: &str) -> String {
        t.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<String>().chars().filter(|c| !c.is_whitespace()).collect()
    }
    /// O item que começa em `inicio` até à chaveta que fecha o seu corpo.
    fn item(fonte: &str, inicio: &str) -> String {
        let i = fonte.find(inicio).unwrap_or_else(|| panic!("{inicio} não encontrado"));
        let mut prof = 0i32;
        for (j, c) in fonte[i..].char_indices() {
            match c {
                '{' => prof += 1,
                '}' => {
                    prof -= 1;
                    if prof == 0 {
                        return fonte[i..i + j + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("{inicio}: corpo sem fecho")
    }

    const PORTAS: &str = "Hardware Port: Wi-Fi\nDevice: en0\nEthernet Address: aa\n\nHardware Port: Ethernet\nDevice: en1\n";

    #[test]
    fn r85c_h13_ifconfig_falhado_nao_e_filtrado() {
        let consultadas = std::cell::RefCell::new(Vec::new());
        let r = probe_macos_com(
            || Ok(PORTAS.to_string()),
            |i| {
                consultadas.borrow_mut().push(i.to_string());
                None
            },
        );
        let esperado = NetworkPolicyError::ProbeFailed {
            probe: "ifconfig",
            error: "estado WiFi indeterminado: en0=ifconfig nao correu".into(),
        };
        assert_eq!(r, Err(esperado.clone()), "ifconfig que não corre é INDETERMINADO, nunca filtrado");
        assert_eq!(
            esperado.to_string(),
            "[LUMYX CRITICAL] WiFi probe `ifconfig` failed (estado WiFi indeterminado: en0=ifconfig nao correu). \
             The WiFi-forbidden rule could not be verified on a supported platform."
        );
        assert_eq!(*consultadas.borrow(), vec!["en0".to_string()], "só a porta Wi-Fi é consultada");
        // controlos: ativa → WifiActive; inactive → Ok; networksetup falhado → o seu ProbeFailed
        assert_eq!(probe_macos_com(|| Ok(PORTAS.into()), |_| Some((true, "\tstatus: active\n".into()))),
            Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] }));
        assert_eq!(probe_macos_com(|| Ok(PORTAS.into()), |_| Some((true, "\tstatus: inactive\n".into()))), Ok(()));
        let ns = NetworkPolicyError::ProbeFailed { probe: "networksetup -listallhardwareports", error: "exit status 1".into() };
        assert_eq!(probe_macos_com(|| Err(ns.clone()), |_| panic!("não há portas para consultar")), Err(ns));
    }

    #[test]
    fn r85c_h14b_indeterminada_nao_vira_desligada() {
        for (saida, erro) in [
            (Some((false, "\tstatus: inactive\n")), "estado WiFi indeterminado: en0=ifconfig com exit != 0"),
            (Some((true, "\tether aa\n")), "estado WiFi indeterminado: en0=sem linha status: reconhecida"),
        ] {
            let r = probe_macos_com(|| Ok(PORTAS.into()), |_| saida.map(|(ok, t)| (ok, t.to_string())));
            let esperado = NetworkPolicyError::ProbeFailed { probe: "ifconfig", error: erro.into() };
            assert_eq!(r, Err(esperado.clone()), "{saida:?}: indeterminado nunca é desligado");
            assert_eq!(
                esperado.to_string(),
                format!("[LUMYX CRITICAL] WiFi probe `ifconfig` failed ({erro}). The WiFi-forbidden rule could not be verified on a supported platform.")
            );
        }
    }

    fn cabo(nome: &str) -> EntradaSysfs {
        EntradaSysfs::Interface { nome: nome.into(), wireless: false, phy80211: false,
            uevent: Some(format!("INTERFACE={nome}\n")), operstate: Some("up\n".into()) }
    }

    #[test]
    fn r85c_h11_entrada_ilegivel_do_read_dir_e_indeterminada() {
        let entradas = vec![Ok(cabo("eth0")), Err(std::io::Error::other("EIO")), Ok(cabo("eth1"))];
        let r = classificar_entradas("sysfs /sys/class/net", entradas.into_iter());
        let esperado = NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "estado WiFi indeterminado: ?=entrada de read_dir ilegivel".into(),
        };
        assert_eq!(r, Err(esperado.clone()), "uma entrada ilegível não desaparece em silêncio");
        assert_eq!(esperado.to_string(), "[LUMYX CRITICAL] WiFi probe `sysfs /sys/class/net` failed (estado WiFi indeterminado: ?=entrada de read_dir ilegivel). The WiFi-forbidden rule could not be verified on a supported platform.");
        // controlo: sem a entrada ilegível, só cabo → Ok
        assert_eq!(classificar_entradas("sysfs /sys/class/net", vec![Ok(cabo("eth0"))].into_iter()), Ok(()));
    }

    const PROBE_LINUX: &str = r#"
        #[cfg(target_os = "linux")]
        fn probe_linux() -> Result<(), NetworkPolicyError> {
            probe_sysfs(std::path::Path::new(RAIZ_SYSFS))
        }"#;

    #[test]
    fn r85c_l2_probe_linux_nao_tem_fallback() {
        // (a) estrutural: o corpo de produção é EXATAMENTE a chamada — sem fallback para o algoritmo antigo
        let f = producao_normalizada();
        assert_eq!(item(&f, "#[cfg(target_os=\"linux\")]fnprobe_linux()"), normaliza(PROBE_LINUX));
        // (b) a regra: um rádio phy80211 em `dormant` é INDETERMINADO (o algoritmo antigo dizia desligado)
        let sys = Sysfs::novo(&[("wlan0", false, true, Some("INTERFACE=wlan0\n"), Some("dormant\n"))]);
        let esperado = NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "estado WiFi indeterminado: wlan0=dormant".into(),
        };
        assert_eq!(probe_sysfs(&sys.0), Err(esperado.clone()));
        assert_eq!(esperado.to_string(), "[LUMYX CRITICAL] WiFi probe `sysfs /sys/class/net` failed (estado WiFi indeterminado: wlan0=dormant). The WiFi-forbidden rule could not be verified on a supported platform.");
    }

    #[test]
    fn r85c_l3_probe_failed_nao_vira_ok() {
        let corpo = item(&producao_normalizada(), "#[cfg(target_os=\"linux\")]fnprobe_linux()");
        assert!(!corpo.contains("Ok(") && !corpo.contains("match"), "probe_linux reinterpreta o resultado: {corpo}");
        assert_eq!(corpo, normaliza(PROBE_LINUX));
    }

    const PROBE_WIFI: &str = r#"
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
        }"#;

    #[test]
    fn r85c_l4_probe_failed_nao_vira_unavailable() {
        let corpo = item(&producao_normalizada(), "fnprobe_wifi()");
        let linux = &corpo[corpo.find("#[cfg(target_os=\"linux\")]").unwrap()..corpo.find("#[cfg(not(").unwrap()];
        assert_eq!(linux, "#[cfg(target_os=\"linux\")]returnprobe_linux();", "o ramo Linux devolve a sonda sem a tocar");
        assert_eq!(corpo, normaliza(PROBE_WIFI));
    }

    #[test]
    fn r85c_l5_resultado_da_sonda_nao_e_ignorado() {
        let corpo = item(&producao_normalizada(), "fnprobe_wifi()");
        assert!(corpo.contains("returnprobe_linux();") && corpo.contains("returnprobe_macos();"));
        assert!(!corpo.contains("let_=") && !corpo.contains("Ok(())"), "o resultado da sonda é descartado: {corpo}");
        assert_eq!(corpo, normaliza(PROBE_WIFI));
        assert_eq!(item(&producao_normalizada(), "#[cfg(target_os=\"macos\")]fnprobe_macos()"),
            "#[cfg(target_os=\"macos\")]fnprobe_macos()->Result<(),NetworkPolicyError>{probe_macos_com(networksetup_real,ifconfig_real)}");
    }

    #[test]
    fn r85c_l6_raiz_e_sys_class_net() {
        assert_eq!(RAIZ_SYSFS, "/sys/class/net");
        let f = producao_normalizada();
        assert_eq!(f.matches("\"/sys/class/net\"").count(), 1, "o literal da raiz real existe UMA vez: {f}");
        assert!(f.contains("constRAIZ_SYSFS:&str=\"/sys/class/net\";"), "e é o RAIZ_SYSFS");
        let corpo = item(&f, "#[cfg(target_os=\"linux\")]fnprobe_linux()");
        let corpo = &corpo[corpo.find('{').unwrap()..];
        assert!(corpo.contains("RAIZ_SYSFS") && !corpo.contains('"'), "probe_linux usa RAIZ_SYSFS e nenhum literal: {corpo}");
    }

    #[test]
    fn r85c_ficheiro_em_sys_class_net_nao_e_interface() {
        let sys = Sysfs::novo(&[("eth0", false, false, Some("INTERFACE=eth0\n"), Some("up\n"))]);
        std::fs::write(sys.0.join("bonding_masters"), "").unwrap();
        assert_eq!(probe_sysfs(&sys.0), Ok(()), "bonding_masters é um ficheiro do módulo bonding, não uma interface");
        assert_eq!(ler_entrada(&sys.0.join("bonding_masters"), "bonding_masters".into()), EntradaSysfs::NaoDiretorio);
    }

    #[test]
    fn r85c_metadata_ilegivel_continua_indeterminada() {
        let sys = Sysfs::novo(&[("eth0", false, false, Some("INTERFACE=eth0\n"), Some("up\n"))]);
        // um symlink pendurado: metadata() falha — não se sabe o que é, logo INDETERMINADO (fail-closed)
        std::os::unix::fs::symlink(sys.0.join("nao-existe"), sys.0.join("wlx0")).unwrap();
        assert_eq!(probe_sysfs(&sys.0), Err(NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "estado WiFi indeterminado: wlx0=metadata ilegivel".into(),
        }));
    }

    /// No /sys REAL do runner (CI ubuntu-24.04): a classificação de cada entrada presente é coerente, e o
    /// `probe_linux()` real é o veredito dessas classificações. NÃO assume que não há WiFi.
    #[cfg(target_os = "linux")]
    #[test]
    fn r85c_sys_real_classificacao_coerente() {
        let raiz = std::path::Path::new(RAIZ_SYSFS);
        let mut lidas = Vec::new();
        for e in std::fs::read_dir(raiz).expect("/sys/class/net no runner") {
            let e = e.expect("entrada legível no runner");
            let p = e.path();
            let nome = e.file_name().to_string_lossy().to_string();
            let entrada = ler_entrada(&p, nome.clone());
            let e_dir = std::fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
            eprintln!("INVENTARIO {nome}: dir={e_dir} {entrada:?}");
            match &entrada {
                EntradaSysfs::NaoDiretorio => assert!(!e_dir, "{nome}"),
                EntradaSysfs::Interface { wireless, phy80211, uevent, .. } => {
                    assert!(e_dir, "{nome}");
                    let sinais = *wireless || *phy80211 || nome.starts_with("wl")
                        || uevent.as_deref().is_some_and(|u| u.lines().any(|l| l.trim() == "DEVTYPE=wlan"));
                    assert_eq!(e_wifi_linux(&nome, *wireless, *phy80211, uevent.as_deref()).unwrap_or(true), sinais || uevent.is_none(), "{nome}");
                    if nome == "lo" {
                        assert!(!sinais, "lo nunca é WiFi");
                    }
                }
                EntradaSysfs::Ilegivel { .. } => {}
            }
            lidas.push(Ok(entrada));
        }
        assert!(!lidas.is_empty(), "o runner tem de ter pelo menos `lo`");
        assert_eq!(probe_linux(), classificar_entradas("sysfs /sys/class/net", lidas.into_iter()),
            "probe_linux real ≠ veredito das classificações do mesmo /sys");
    }

    /// A API pública do led-hal neste ficheiro continua a ser exatamente a de 8b99cef (4 itens).
    #[test]
    fn r85c_api_publica_inalterada() {
        let f = producao_normalizada();
        let mut publicos: Vec<&str> = ["pubenum", "pubtrait", "pubstruct", "pubfn", "pubconst", "pubtype", "pubmod", "pubuse", "pub("]
            .iter().flat_map(|k| f.match_indices(k).map(|(i, _)| &f[i..(i + 30).min(f.len())])).collect();
        publicos.sort();
        assert_eq!(publicos.len(), 4, "{publicos:?}");
        for nome in ["pubenumNetworkPolicyError", "pubtraitNetworkGuard", "pubstructPermissiveGuard;", "pubstructWifiBlockGuard;"] {
            assert!(publicos.iter().any(|p| p.starts_with(nome)), "{nome} em {publicos:?}");
        }
    }
}
