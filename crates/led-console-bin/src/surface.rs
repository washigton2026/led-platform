//! ADR-0026 §5 e §14 — a **superfície declarada** do console.
//!
//! # Porque isto é uma tabela, e não um router
//!
//! O transporte HTTP ainda não existe (F1-B é fundação). Mas a *superfície* pode ser
//! declarada agora — e declarada como **dado** ela fica auditável: um gate percorre esta
//! tabela e reprova se `shutdown` ou blackout aparecerem, em vez de depender de alguém ler
//! um router à mão daqui a três fatias.
//!
//! É a mesma disciplina do `presets.rs` do ADR-0018: uma tabela `const`, sem ramificação.

/// O método HTTP. Comandos são sempre `Post`; leitura é sempre `Get`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbo {
    Get,
    Post,
}

/// Uma rota da superfície do console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rota {
    pub verbo: Verbo,
    pub caminho: &'static str,
    /// O comando do IPC v1 que esta rota traduz, se traduzir algum.
    ///
    /// `None` = rota puramente de leitura ou de fluxo (não enfileira nada no laço).
    pub cmd_ipc: Option<&'static str>,
    /// Porque esta rota existe. Não é decoração: é o que se lê quando alguém pergunta se
    /// pode acrescentar uma.
    pub razao: &'static str,
}

/// **A superfície completa.** Acrescentar uma linha aqui é a única forma de acrescentar uma
/// rota — e o gate de `tests/surface_gate.rs` corre sobre esta tabela.
pub const ROTAS: &[Rota] = &[
    Rota {
        verbo: Verbo::Get,
        caminho: "/api/state",
        cmd_ipc: Some("status"),
        razao: "snapshot publicado pelo laco; nao compete com comandar (GS3)",
    },
    Rota {
        verbo: Verbo::Get,
        caminho: "/api/version",
        cmd_ipc: Some("version"),
        razao: "versao do protocolo e do engine",
    },
    Rota {
        verbo: Verbo::Get,
        caminho: "/api/events",
        cmd_ipc: None,
        razao: "SSE; o console ja subscreveu uma vez e faz fan-out (ADR-0026 §4)",
    },
    Rota {
        verbo: Verbo::Get,
        caminho: "/api/profiles",
        cmd_ipc: None,
        razao: "catalogo de presets, estatico; nenhum valor fisico e recalculado aqui",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/load",
        cmd_ipc: Some("load"),
        razao: "o gate do pre-voo fica visivel: sem integridade afirmada, play devolve not_armed",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/unload",
        cmd_ipc: Some("unload"),
        razao: "transporte (ADR-0023)",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/play",
        cmd_ipc: Some("play"),
        razao: "transporte (ADR-0023)",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/pause",
        cmd_ipc: Some("pause"),
        razao: "transporte; NAO apaga o palco (ADR-0023 decisao 3)",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/stop",
        cmd_ipc: Some("stop"),
        razao: "transporte; NAO apaga o palco (ADR-0023 decisao 3)",
    },
    Rota {
        verbo: Verbo::Post,
        caminho: "/api/transport/seek",
        cmd_ipc: Some("seek"),
        razao: "transporte (ADR-0023)",
    },
];

/// Comandos do IPC v1 que a superfície HTTP **nunca** expõe, com a razão de cada um.
///
/// Existe como tabela para que a ausência seja **afirmada**, não apenas omitida — uma rota
/// que falta por esquecimento e uma que falta por decisão são indistinguíveis sem isto.
pub const NUNCA_EXPOSTOS: &[(&str, &str)] = &[
    (
        "shutdown",
        "irreversivel, duas fases, e nao ha auth (ADR-0014). Fica no ledctl, que exige \
         acesso ao socket 0600",
    ),
    ("hello", "handshake do console com o daemon; nao e do dominio do operador"),
    ("subscribe", "o console possui a subscricao unica e faz fan-out (ADR-0026 §4)"),
    ("ping", "diagnostico da ligacao; /api/state ja cobre o que o operador precisa"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toda_rota_de_comando_e_post_e_toda_leitura_e_get() {
        for r in ROTAS {
            match r.verbo {
                Verbo::Post => assert!(
                    r.caminho.starts_with("/api/transport/"),
                    "{}: POST so em transporte",
                    r.caminho
                ),
                Verbo::Get => assert!(
                    !r.caminho.starts_with("/api/transport/"),
                    "{}: transporte nao se le por GET",
                    r.caminho
                ),
            }
        }
    }

    /// **Nenhum caminho duplicado** — dois handlers para a mesma rota seriam dois caminhos.
    #[test]
    fn nenhuma_rota_repetida() {
        let mut c: Vec<&str> = ROTAS.iter().map(|r| r.caminho).collect();
        let n = c.len();
        c.sort_unstable();
        c.dedup();
        assert_eq!(c.len(), n, "ha caminhos repetidos na superficie");
    }

    /// A tabela de ausências e a tabela de rotas **não podem intersectar-se**.
    #[test]
    fn o_que_nunca_e_exposto_nao_esta_exposto() {
        for (cmd, razao) in NUNCA_EXPOSTOS {
            assert!(!razao.is_empty(), "{cmd}: a ausencia tem de ter razao escrita");
            assert!(
                !ROTAS.iter().any(|r| r.cmd_ipc == Some(cmd)),
                "`{cmd}` esta na tabela de nunca-expostos E numa rota"
            );
        }
    }
}
