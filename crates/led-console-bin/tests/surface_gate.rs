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
    ("metrics.rs", include_str!("../src/metrics.rs")),
    ("contract.rs", include_str!("../src/contract.rs")),
    ("http.rs", include_str!("../src/http.rs")),
    // A superfície da CLI. Ficou de fora desde o COMMAND 03 e **escapava aos três gates
    // textuais** deste ficheiro — no único sítio onde um operador escreve flags. Só pôde
    // entrar depois de `linhas_de_codigo` aprender a parar nos testes: o `mod tests` do
    // `main.rs` nomeia `blackout` de propósito, para o proibir no `--help`.
    ("main.rs", include_str!("../src/main.rs")),
];

/// Linhas de **produção**. Duas exclusões, por razões diferentes.
///
/// **Comentários** (`//`, `*`) são legítimos e podem citar os nomes: é preciso poder
/// escrever *"o ADR-0017 proíbe blackout"* sem que o gate reprove por isso.
///
/// **O `mod tests` e tudo o que vem depois**, porque um gate não pode reprovar por causa
/// de um teste que nomeia o proibido **para o proibir**. É exactamente o caso do
/// `main.rs`: o seu `a_ajuda_descreve_as_flags_reais_e_os_limites_reais` percorre
/// `["blackout", "--auth", "--cors", "0.0.0.0"]` para afirmar que o `--help` não os
/// menciona. Sem este corte, acrescentá-lo às `FONTES` faria o gate do ADR-0017 reprovar
/// contra um teste que impõe a **mesma** regra — a segunda vez que este repositório
/// tropeçaria em *"um gate não pode ser o sítio onde o proibido é escrito"* (F1-B).
///
/// O corte é por `"mod tests"` e **não** por `#[cfg(test)]` de propósito: o `main.rs`
/// declara-o como `#[cfg(all(test, unix))]`, que um filtro pelo atributo não apanharia.
/// O idioma é o que o próprio `main.rs` já usa contra si mesmo
/// (`FONTE.split("mod tests")`), e reusá-lo evita uma segunda regra para o mesmo fim.
fn linhas_de_codigo(fonte: &str) -> impl Iterator<Item = &str> {
    let producao = fonte.split("mod tests").next().unwrap_or(fonte);
    producao.lines().filter(|l| {
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

// ── ADR-0026 §9-bis — o browser tem UMA origem ───────────────────────────────

/// **O browser nunca vê o exporter diretamente.**
///
/// O `led_hal::serve_metrics` é um `TcpListener` **noutro processo**. Se a superfície do
/// console alguma vez expuser `/metrics` (o caminho do exporter) em vez de `/api/metrics`,
/// o browser passa a ter uma **segunda origem** — um caminho que não atravessa o tradutor, e
/// ao qual nada do que este crate garante se aplica.
///
/// O teste é sobre a **forma do caminho**, não sobre o nome: qualquer rota que não comece por
/// `/api/` já é recusada por `toda_rota_declara_a_sua_razao`; esta fecha o caso específico de
/// alguém acrescentar o caminho do exporter tal e qual.
#[test]
fn o_exporter_nao_e_alcancavel_diretamente_pelo_browser() {
    for r in ROTAS {
        assert_ne!(
            r.caminho, "/metrics",
            "o caminho do exporter esta exposto ao browser: seria uma SEGUNDA ORIGEM \
             (ADR-0026 §9-bis). O browser fala com /api/metrics, e o console e que fala \
             com o exporter"
        );
        // E nenhuma rota pode apontar para fora do namespace do console.
        assert!(
            r.caminho.starts_with("/api/"),
            "{}: fora de /api/ — o browser passaria a ter duas origens",
            r.caminho
        );
    }
    // A rota do proxy tem de existir: sem ela, a única forma de o browser ver métricas
    // seria falar com o exporter — exatamente o que este gate proíbe.
    assert!(
        ROTAS.iter().any(|r| r.caminho == "/api/metrics" && r.verbo == Verbo::Get),
        "sem /api/metrics, o browser nao tem por onde ver metricas sem abrir 2.a origem"
    );
}

/// **O proxy não pode ganhar lógica de métricas.**
///
/// O §9-bis proíbe recalcular, agregar e reescrever o formato. Um proxy que somasse séries
/// seria uma **segunda fonte de verdade** sobre observabilidade, e a divergência entre o que
/// o Prometheus raspa e o que o browser vê não seria visível de nenhum dos lados.
#[test]
fn o_proxy_de_metricas_nao_calcula_nada() {
    const CALCULO: &[&str] = &[
        "prometheus_text", // reimplementar a formatação
        "MetricsEmitter",  // ler o emitter em vez de o exporter
        "percentile",
        "p99",
        "p50",
        "histogram",
        ".sum()",
        "fold(",
    ];
    let fonte = FONTES
        .iter()
        .find(|(n, _)| *n == "metrics.rs")
        .expect("metrics.rs tem de estar nas FONTES, senao escapa a todos os gates")
        .1;
    for linha in linhas_de_codigo(fonte) {
        for c in CALCULO {
            assert!(
                !linha.contains(c),
                "metrics.rs: `{c}` — o proxy repassa, nao calcula (ADR-0026 §9-bis)\n  {linha}"
            );
        }
    }
}
