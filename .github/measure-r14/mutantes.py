#!/usr/bin/env python3
"""Mutantes LINUX do falsificador, ronda 14 — TD-029 R8.5b, commit 8b99cef.

NÃO corridos em macOS (o código é #[cfg(target_os = "linux")]). Para a CI ubuntu-24.04, num PR de medição.

Uso (numa cópia limpa do commit, um mutante de cada vez, builds sequenciais):
    python3 -I mutantes.py lista
    python3 -I mutantes.py aplica <ID> <raiz-do-repo>
e depois, por mutante:
    cargo test --workspace --locked --no-fail-fast            # o GATE — esperado: VERDE (sobrevive)
    cargo clippy --workspace --all-targets --locked -- -D warnings
    # com os oráculos de linux/oraculos.rs colados (ver instruções lá):
    cargo test -p led-hal --locked --lib oraculo_linux_r14    # esperado: VERMELHO, `panicked at`
Antes de qualquer mutante: correr C0 (sem mutação) COM os oráculos colados — os oráculos têm de passar todos em C0.
Se `o14l_probe_linux_no_runner_real_e_ok` falhar em C0, LER o inventário impresso no log (eprintln): é uma
entrada real de /sys/class/net mal classificada (ex.: `bonding_masters`) — achado, não mutante.
"""
import sys

NG = "crates/led-hal/src/network_guard.rs"

ANCORA_PROBE_LINUX = """#[cfg(target_os = "linux")]
fn probe_linux() -> Result<(), NetworkPolicyError> {
    probe_sysfs(std::path::Path::new("/sys/class/net"))
}"""

ANCORA_PROBE_WIFI_LINUX = """    #[cfg(target_os = "linux")]
    return probe_linux();"""

# (id, ficheiro, âncora exata, substituição, gate esperado, oráculo esperado a ficar vermelho, classe se sobreviver)
MUTANTES = [
    ("L1", NG, ANCORA_PROBE_LINUX,
     ANCORA_PROBE_LINUX.replace('"/sys/class/net"', '"/sys/class/nett"'),
     "VERDE (o hal_with_guard_wifi_block_does_not_panic aceita qualquer Err)",
     "o14l_probe_linux_no_runner_real_e_ok (ProbeFailed raiz ausente) + o14l_probe_linux_e_so_probe_sysfs_da_raiz_real",
     "FORA (fail-closed: bloqueia sempre; disponibilidade, não FALSE_GREEN)"),
    ("L2", NG, ANCORA_PROBE_LINUX,
     """#[cfg(target_os = "linux")]
fn probe_linux() -> Result<(), NetworkPolicyError> {
    // «robustez»: se a sonda nova não decidir, cai para o algoritmo antigo (pre-R8.5b: só wl*, só `up`
    // é ativo, ilegível/dormant/unknown = inativo). Mantém probe_sysfs em uso (sem dead_code no clippy).
    match probe_sysfs(std::path::Path::new("/sys/class/net")) {
        Err(NetworkPolicyError::ProbeFailed { .. }) => {
            let mut ativas = Vec::new();
            if let Ok(entradas) = std::fs::read_dir("/sys/class/net") {
                for e in entradas.flatten() {
                    let nome = e.file_name().to_string_lossy().to_string();
                    if !nome.starts_with("wl") { continue; }
                    let st = std::fs::read_to_string(e.path().join("operstate")).unwrap_or_default();
                    if st.trim() == "up" { ativas.push(nome); }
                }
            }
            if ativas.is_empty() { Ok(()) } else { Err(NetworkPolicyError::WifiActive { interfaces: ativas }) }
        }
        r => r,
    }
}""",
     "VERDE (nenhum teste liga probe_linux a probe_sysfs; o runner não tem WiFi, logo Ok nos dois)",
     "o14l_probe_linux_e_so_probe_sysfs_da_raiz_real (estrutural)",
     "DENTRO (decisão (b) desfeita no código de produção Linux do led-hal: todo o indeterminado volta a ser desligado)"),
    ("L3", NG, ANCORA_PROBE_LINUX,
     """#[cfg(target_os = "linux")]
fn probe_linux() -> Result<(), NetworkPolicyError> {
    match probe_sysfs(std::path::Path::new("/sys/class/net")) {
        Err(NetworkPolicyError::ProbeFailed { .. }) => Ok(()),
        r => r,
    }
}""",
     "VERDE",
     "o14l_probe_linux_e_so_probe_sysfs_da_raiz_real (estrutural)",
     "DENTRO (D3 + decisão (b): indeterminado vira Ok, sem chegar ao pré-voo como sonda falhada)"),
    ("L4", NG, ANCORA_PROBE_WIFI_LINUX,
     """    #[cfg(target_os = "linux")]
    return probe_linux().map_err(|e| match e {
        NetworkPolicyError::ProbeFailed { probe, error } => {
            NetworkPolicyError::ProbeUnavailable { reason: format!("{probe}: {error}") }
        }
        outro => outro,
    });""",
     "VERDE (o teste de ProbeUnavailable só corre em plataformas não suportadas; o runner devolve Ok)",
     "o14l_probe_wifi_linux_chama_probe_linux (estrutural); o oráculo de /sys real NÃO distingue (runner Ok)",
     "DENTRO (D3: sonda falhada numa plataforma suportada rebaixada a «não suportada» ⇒ prossegue com aviso)"),
    ("L5", NG, ANCORA_PROBE_WIFI_LINUX,
     """    #[cfg(target_os = "linux")]
    return {
        let _ = probe_linux();
        Ok(())
    };""",
     "VERDE",
     "o14l_probe_wifi_linux_chama_probe_linux (estrutural); o oráculo de /sys real NÃO distingue (runner sem WiFi)",
     "DENTRO (ADR-0005: Linux deixa de decidir pela sonda)"),
    ("L6", NG, ANCORA_PROBE_LINUX,
     ANCORA_PROBE_LINUX.replace('"/sys/class/net"', '"/sys/devices/virtual/net"'),
     "VERDE",
     "o14l_probe_linux_e_so_probe_sysfs_da_raiz_real (estrutural); o oráculo de /sys real NÃO distingue",
     "DENTRO (só interfaces virtuais: um rádio PCI/USB nunca é visto)"),
]


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "lista":
        for (i, f, _a, _b, gate, ora, cls) in MUTANTES:
            print(f"{i}\t{f}\tgate esperado: {gate}\n\toráculo: {ora}\n\tse sobreviver: {cls}")
        return
    if len(sys.argv) != 4 or sys.argv[1] != "aplica":
        sys.exit(__doc__)
    mid, raiz = sys.argv[2], sys.argv[3]
    (i, f, a, b, *_r) = next(m for m in MUTANTES if m[0] == mid)
    p = f"{raiz}/{f}"
    s = open(p).read()
    assert s.count(a) == 1, (mid, "âncora encontrada", s.count(a))
    open(p, "w").write(s.replace(a, b))
    print(f"{mid} aplicado em {p}")


if __name__ == "__main__":
    main()
