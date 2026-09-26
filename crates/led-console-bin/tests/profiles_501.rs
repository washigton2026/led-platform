//! **F7/A — `/api/profiles` responde 501, e isso é uma decisão, não uma omissão.**
//!
//! # O que 501 significa aqui
//!
//! *"A rota é conhecida pelo contrato, mas a capacidade ainda não está disponível através da
//! fronteira autorizada."*
//!
//! As três respostas possíveis dizem coisas **diferentes**, e a diferença é o ponto:
//!
//! | Resposta | Significa | Verdadeiro? |
//! |---|---|---|
//! | `404` | a rota não existe | **não** — existe, e está no contrato gerado |
//! | `200 []` | o catálogo existe e está vazio | **não** — o catálogo tem 8 presets |
//! | `501` | a rota existe, a capacidade não chega aqui | **sim** |
//!
//! `200 []` é a pior das três: um operador que veja uma lista vazia conclui que **não há
//! hardware configurado**, quando o que não há é caminho até ao catálogo. Manda-o procurar o
//! defeito no sítio errado — a mesma classe de *blame* invertido que o `413` do
//! `PedidoDemasiadoGrande` já corrigiu.
//!
//! # Porque não se resolve importando o catálogo
//!
//! O catálogo vive no `led-hardware-profile`. Trazê-lo para cá é trazer **domínio** para o
//! tradutor, e o gate `nenhuma_segunda_fonte_de_verdade_no_console` recusa-o pelo nome. A
//! outra saída seria um comando novo no IPC v1 — que está **fechado**. Nenhuma das duas é uma
//! edição; ambas são decisões, e é por isso que a rota fica 501 até haver uma.

#![cfg(unix)]

use led_console_bin::surface::{Verbo, ROTAS};

const HTTP_RS: &str = include_str!("../src/http.rs");
const SURFACE_RS: &str = include_str!("../src/surface.rs");
const CARGO_TOML: &str = include_str!("../Cargo.toml");

/// Linhas de código — comentários podem discutir o que o código não pode fazer.
fn codigo(fonte: &str) -> impl Iterator<Item = &str> {
    fonte.lines().map(|l| l.trim()).filter(|t| !t.starts_with("//") && !t.starts_with('*'))
}

/// **A rota existe no contrato.** É isto que torna o 404 uma mentira.
#[test]
fn a_rota_existe_e_e_de_leitura() {
    let r = ROTAS
        .iter()
        .find(|r| r.caminho == "/api/profiles")
        .expect("/api/profiles tem de estar na superficie declarada");
    assert_eq!(r.verbo, Verbo::Get, "catalogo e leitura");
    assert!(!r.razao.is_empty(), "a rota tem de dizer porque existe");
}

/// **Nenhum comando IPC foi inventado para servir perfis.**
///
/// O IPC v1 está fechado. Se alguém acrescentasse `cmd_ipc: Some("profiles")`, teria de
/// existir um comando `profiles` no protocolo — e não existe.
#[test]
fn nenhum_comando_ipc_foi_inventado_para_perfis() {
    let r = ROTAS.iter().find(|r| r.caminho == "/api/profiles").expect("rota");
    assert_eq!(
        r.cmd_ipc, None,
        "`/api/profiles` mapeia para o comando IPC {:?} — o IPC v1 esta FECHADO e nao tem \
         comando de perfis",
        r.cmd_ipc
    );

    // E nenhuma rota inventou um comando que o protocolo não define.
    let conhecidos = [
        "hello", "ping", "version", "status", "load", "unload", "play", "pause", "stop",
        "seek", "subscribe", "shutdown",
    ];
    for rota in ROTAS {
        if let Some(cmd) = rota.cmd_ipc {
            assert!(
                conhecidos.contains(&cmd),
                "{}: `{cmd}` nao e um comando do IPC v1",
                rota.caminho
            );
        }
    }
}

/// **O console não importa o catálogo, nem por dependência nem por uso.**
///
/// Duas verificações, porque uma só não basta: a dependência pode estar declarada sem uso, e
/// o uso pode aparecer sem a dependência (por reexportação de outro crate).
#[test]
fn o_catalogo_nao_entra_no_tradutor() {
    // Nas dependências: `led-hardware-profile` não é dependência de produção do console.
    let deps: String = CARGO_TOML
        .lines()
        .take_while(|l| !l.trim().starts_with("[dev-dependencies]"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !deps.contains("led-hardware-profile"),
        "`led-hardware-profile` entrou nas dependencias de producao do console — e dominio a \
         escorregar para dentro do tradutor"
    );

    // No código: nem o crate, nem os tipos do catálogo.
    for proibido in ["led_hardware_profile", "HardwareProfile", "PresetRow", "PRESETS"] {
        assert!(
            !codigo(HTTP_RS).any(|l| l.contains(proibido)),
            "`{proibido}` aparece no servidor HTTP — o catalogo nao pode ser lido daqui"
        );
    }
}

/// **Nenhum perfil está escrito à mão no console.**
///
/// Um catálogo duplicado é pior que um 501: divergiria em silêncio do verdadeiro, e ninguém
/// saberia qual dos dois o rig está a seguir.
#[test]
fn nenhum_perfil_esta_escrito_a_mao() {
    // Nomes reais de presets do catálogo. Se algum aparecer aqui, foi copiado.
    for preset in [
        "esp32-poe-wled-ddp",
        "esp32-poe-wled-rgbw-ddp",
        "generic-sk6812-rgbw-sacn",
        "falcon",
        "advatek",
    ] {
        for (nome, fonte) in [("http.rs", HTTP_RS), ("surface.rs", SURFACE_RS)] {
            assert!(
                !codigo(fonte).any(|l| l.contains(preset)),
                "o preset `{preset}` esta escrito a mao em {nome} — catalogo duplicado"
            );
        }
    }
}

/// **A resposta é 501, e o corpo diz porquê.**
///
/// O gate é textual porque não há servidor a correr aqui; o comportamento sobre HTTP real é
/// verificado em `tests/http_server.rs`. Este teste guarda o **código** e a **razão**.
#[test]
fn a_resposta_e_501_e_nunca_200_nem_404() {
    let perfis = HTTP_RS
        .split("fn perfis()")
        .nth(1)
        .expect("a funcao `perfis` tem de existir")
        .split("\nfn ")
        .next()
        .expect("corpo de `perfis`");

    assert!(perfis.contains("501"), "`perfis` tem de responder 501:\n{perfis}");
    assert!(
        !perfis.contains("200"),
        "`perfis` responde 200 — um catalogo vazio diria que NAO HA hardware, quando o que \
         nao ha e caminho ate ao catalogo:\n{perfis}"
    );
    assert!(
        !perfis.contains("404"),
        "`perfis` responde 404 — mas a rota EXISTE e esta no contrato gerado:\n{perfis}"
    );
    // E não devolve coleção nenhuma.
    for vazio in ["[]", "vec![]", "Vec::new()"] {
        assert!(
            !perfis.contains(vazio),
            "`perfis` devolve a colecao vazia `{vazio}` — 200 com lista vazia e a pior das \
             tres respostas possiveis"
        );
    }
}
