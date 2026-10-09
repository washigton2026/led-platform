// Oráculos LINUX do falsificador, ronda 14 — TD-029 R8.5b, commit 8b99cef.
//
// ONDE COLAR: no FIM de `crates/led-hal/src/network_guard.rs` (depois do `mod tests { … }` que fecha o
// ficheiro), tal como está — é um módulo irmão de `tests`, por isso vê as funções privadas via `super::*`.
// SÓ numa cópia de medição (PR de medição), nunca no ramo do TD-029.
//
// CORRER (ubuntu-24.04):
//   cargo test -p led-hal --locked --lib oraculo_linux_r14 -- --nocapture
// C0 (sem mutante): os 5 têm de passar. O `--nocapture` imprime o inventário de /sys/class/net do runner
// (nome, é-dir, wireless/, phy80211/, DEVTYPE, operstate) — guardar o log: é a evidência do «o que se sabe
// do runner». Se `o14l_probe_linux_no_runner_real_e_ok` falhar em C0, NÃO é falha do oráculo: é uma
// entrada real mal classificada (ver o inventário; candidato óbvio: `bonding_masters`, ficheiro e não dir).
// Por mutante (linux/mutantes.py): o oráculo indicado na lista tem de ficar VERMELHO com `panicked at`.

#[cfg(all(test, target_os = "linux"))]
mod oraculo_linux_r14 {
    use super::*;

    fn inventario() -> String {
        let mut out = String::new();
        for e in std::fs::read_dir("/sys/class/net").expect("/sys/class/net no runner") {
            let e = e.expect("entrada legível");
            let p = e.path();
            let uevent = std::fs::read_to_string(p.join("uevent")).ok();
            let devtype = uevent
                .as_deref()
                .and_then(|u| u.lines().find(|l| l.starts_with("DEVTYPE=")).map(str::to_string));
            out.push_str(&format!(
                "{:?} dir={} wireless={} phy80211={} uevent={} devtype={:?} operstate={:?}\n",
                e.file_name(),
                p.is_dir(),
                p.join("wireless").exists(),
                p.join("phy80211").exists(),
                uevent.is_some(),
                devtype,
                std::fs::read_to_string(p.join("operstate")).ok().map(|s| s.trim().to_string()),
            ));
        }
        out
    }

    /// O que se sabe do runner ubuntu: VM sem rádio — nenhuma interface com wireless/, phy80211/,
    /// DEVTYPE=wlan ou nome wl*. Logo a sonda REAL tem de dar Ok. Apanha L1 (raiz errada ⇒ ProbeFailed).
    #[test]
    fn o14l_probe_linux_no_runner_real_e_ok() {
        let inv = inventario();
        eprintln!("INVENTARIO /sys/class/net:\n{inv}");
        assert!(!inv.contains("wireless=true") && !inv.contains("phy80211=true") && !inv.contains("DEVTYPE=wlan"),
                "premissa do oráculo: o runner não tem WiFi\n{inv}");
        assert_eq!(probe_linux(), Ok(()), "runner sem WiFi ⇒ Ok\n{inv}");
    }

    /// A guarda de produção, no runner: Ok. Apanha L1.
    #[test]
    fn o14l_probe_wifi_no_runner_real_e_ok() {
        assert_eq!(WifiBlockGuard.check(), Ok(()));
    }

    /// Linux é plataforma SUPORTADA: a guarda nunca diz ProbeUnavailable aqui.
    #[test]
    fn o14l_linux_e_plataforma_suportada() {
        assert!(!matches!(probe_wifi(), Err(NetworkPolicyError::ProbeUnavailable { .. })));
    }

    fn sem_comentarios_nem_espacos(s: &str) -> String {
        s.lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// ESTRUTURAL — o probe_linux de produção é EXATAMENTE `probe_sysfs` sobre `/sys/class/net`, e o seu
    /// resultado não é reinterpretado. Apanha L1, L2, L3, L6 (o runner sem WiFi não distingue L2/L3/L6).
    #[test]
    fn o14l_probe_linux_e_so_probe_sysfs_da_raiz_real() {
        let f = sem_comentarios_nem_espacos(include_str!("network_guard.rs"));
        let i = f.find("#[cfg(target_os=\"linux\")]fnprobe_linux()").expect("probe_linux");
        let corpo = &f[i..i + f[i..].find("}").unwrap() + 1];
        assert_eq!(
            corpo,
            "#[cfg(target_os=\"linux\")]fnprobe_linux()->Result<(),NetworkPolicyError>{probe_sysfs(std::path::Path::new(\"/sys/class/net\"))}"
        );
    }

    /// ESTRUTURAL — no Linux, probe_wifi devolve o resultado de probe_linux sem o tocar. Apanha L4, L5.
    #[test]
    fn o14l_probe_wifi_linux_chama_probe_linux() {
        let f = sem_comentarios_nem_espacos(include_str!("network_guard.rs"));
        let i = f.find("fnprobe_wifi()").expect("probe_wifi");
        let fim = f[i..].find("#[cfg(any(test,target_os=\"linux\",target_os=\"macos\"))]").expect("fim de probe_wifi");
        let corpo = &f[i..i + fim];
        assert!(corpo.contains("#[cfg(target_os=\"linux\")]returnprobe_linux();#[cfg(not("), "{corpo}");
    }
}
