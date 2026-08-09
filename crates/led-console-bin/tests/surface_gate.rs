//! **Gates estruturais da superfície** (ADR-0026 §14, §15).
//!
//! Correm sobre a tabela `ROTAS` e sobre o **texto** do crate. Um `grep` prova o estado de um
//! instante; isto prova-o em cada `cargo test` — a mesma técnica de
//! `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida` (GS4.4).

use led_console_bin::surface::{Verbo, NUNCA_EXPOSTOS, ROTAS};

/// Palavras que **não podem** aparecer enquanto o ADR-0017 estiver adiado.
///
/// Vive **no gate**, não na produção: uma lista de proibidos declarada no código que ela
/// própria vigia apanha-se a si mesma — foi o que a primeira versão deste teste fez.
const PROIBIDOS_ADR_0017: &[&str] =
    &["blackout", "dbo", "grand_master", "grandmaster", "intensity"];

const FONTES: &[(&str, &str)] = &[
    ("lib.rs", include_str!("../src/lib.rs")),
    ("limits.rs", include_str!("../src/limits.rs")),
    ("surface.rs", include_str!("../src/surface.rs")),
    ("truth.rs", include_str!("../src/truth.rs")),
    ("fanout.rs", include_str!("../src/fanout.rs")),
    ("ipc.rs", include_str!("../src/ipc.rs")),
];

/// Linhas de código (comentários e doc-comments são legítimos e podem citar os nomes).
fn linhas_de_codigo(fonte: &str) -> impl Iterator<Item = &str> {
    fonte.lines().filter(|l| {
        let t = l.trim_start();
        !t.starts_with("//") && !t.starts_with("*") && !t.is_empty()
    })
}

/// **`shutdown` não é alcançável por nenhuma rota.**
///
/// É irreversível, tem duas fases e não há auth (ADR-0014). Um browser não pode desligar o
/// show; isso fica no `ledctl`, que exige acesso ao socket `0600`.
#[test]
fn shutdown_nao_e_alcancavel_pela_superficie_http() {
    for r in ROTAS {
        assert_ne!(r.cmd_ipc, Some("shutdown"), "{} expoe shutdown", r.caminho);
        assert!(
            !r.caminho.contains("shutdown"),
            "{}: nem o caminho pode sugerir shutdown",
            r.caminho
        );
    }
    assert!(
        NUNCA_EXPOSTOS.iter().any(|(c, _)| *c == "shutdown"),
        "a ausencia tem de ser AFIRMADA na tabela, nao apenas omitida"
    );
}

/// **Blackout não aparece — o ADR-0017 está adiado, e a ausência é a decisão.**
///
/// Espelha o gate que o `led-daemon` já tem sobre a lista de comandos.
#[test]
fn nenhum_blackout_na_superficie_nem_no_codigo() {
    for r in ROTAS {
        for p in PROIBIDOS_ADR_0017 {
            assert!(!r.caminho.to_lowercase().contains(p), "{}: contem `{p}`", r.caminho);
            assert!(
                r.cmd_ipc.map(|c| !c.to_lowercase().contains(p)).unwrap_or(true),
                "{}: comando contem `{p}`",
                r.caminho
            );
        }
    }
    for (nome, fonte) in FONTES {
        for linha in linhas_de_codigo(fonte) {
            for p in PROIBIDOS_ADR_0017 {
                assert!(
                    !linha.to_lowercase().contains(p),
                    "{nome}: `{p}` em codigo — o ADR-0017 esta ADIADO\n  {linha}"
                );
            }
        }
    }
}

/// **Nenhuma segunda fonte de verdade** (ADR-0026 §15).
///
/// O console transporta contratos; não os reimplementa. Se algum destes nomes aparecer em
/// código aqui, é domínio a escorregar para dentro do tradutor.
#[test]
fn nenhuma_segunda_fonte_de_verdade_no_console() {
    // Nomes de domínio que só podem existir a montante.
    const PROIBIDOS: &[&str] = &[
        "CalibrationLut",
        "HardwareProfile",
        "mtu_bytes",
        "refresh_hz",
        "pixels_per_datagram",
        "pixels_per_universe",
        "RgbOrder",
        "ColorFormat",
        "ShowRuntime",
        "OutputManager",
    ];
    // Valores físicos que já têm dono (GS4.3/GS4.4).
    const NUMEROS: &[&str] = &["487", "1462", "170", "2400", "800"];

    for (nome, fonte) in FONTES {
        for linha in linhas_de_codigo(fonte) {
            for p in PROIBIDOS {
                assert!(
                    !linha.contains(p),
                    "{nome}: `{p}` e dominio a montante — o console transporta, nao reimplementa\
                     \n  {linha}"
                );
            }
            for n in NUMEROS {
                assert!(
                    !linha.contains(n),
                    "{nome}: valor fisico `{n}` escrito a mao\n  {linha}"
                );
            }
        }
    }
}

/// **Nenhum timeout escrito à mão.** O único `Duration::from_secs` legítimo é a margem
/// nomeada; o timeout HTTP é derivado do `REPLY_TIMEOUT`.
#[test]
fn nenhum_timeout_duplicado_a_mao() {
    for (nome, fonte) in FONTES {
        for linha in linhas_de_codigo(fonte) {
            if linha.contains("Duration::from_secs") {
                assert!(
                    linha.contains("MARGEM_HTTP"),
                    "{nome}: timeout escrito a mao; derive do REPLY_TIMEOUT\n  {linha}"
                );
            }
        }
    }
}

/// A superfície é pequena **de propósito**, e cada rota tem a sua razão escrita.
#[test]
fn toda_rota_declara_a_sua_razao() {
    assert!(!ROTAS.is_empty());
    for r in ROTAS {
        assert!(!r.razao.is_empty(), "{}: rota sem razao", r.caminho);
        assert!(r.caminho.starts_with("/api/"), "{}: fora do namespace", r.caminho);
    }
    let comandos = ROTAS.iter().filter(|r| r.verbo == Verbo::Post).count();
    assert_eq!(comandos, 6, "load/unload/play/pause/stop/seek — e mais nenhum");
}
