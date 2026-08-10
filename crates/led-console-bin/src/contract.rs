//! ADR-0027 — o contrato TypeScript, **gerado** a partir do Rust.
//!
//! # A regra
//!
//! O Rust é a fonte de verdade. O `.ts` é **artefacto**, versionado para que o frontend não
//! precise de `cargo` e para que um PR mostre a mudança de contrato como diff legível.
//!
//! # Este módulo é o **caminho A**
//!
//! Emite a partir dos **valores Rust compilados**: `EstadoUi::ALL`, `Elo::ALL`,
//! `State::ALL`, as `const` de `proto::code`. Sozinho, **não seria suficiente** — uma
//! variante acrescentada ao `enum` e esquecida no `ALL` sairia daqui em falta, e o ficheiro
//! versionado (gerado por este mesmo caminho) concordaria com o erro.
//!
//! Por isso existe o **caminho B** em `tests/contract_gate.rs`: lê o **texto-fonte** e
//! confronta. Os dois caminhos não partilham a lista, logo não partilham o engano.
//!
//! # Onde este módulo é obrigado a escrever listas à mão
//!
//! `Rejected::code()` é um `match` exaustivo e `proto::code::*` são `const` soltas — nenhum
//! dos dois é enumerável, e ambos vivem a montante (`led-daemon` está **congelado**). As
//! listas abaixo referenciam as `const` **pelo nome** sempre que possível, para que um valor
//! renomeado viaje sozinho; o que elas não conseguem garantir — uma entrada **em falta** — é
//! exatamente o que o caminho B apanha.

use crate::surface::{Verbo, ROTAS};
use crate::truth::{Elo, EstadoUi};
use led_daemon::State;
use led_daemon_bin::proto::code;

/// Os códigos de erro do **protocolo** (ADR-0026 §6 — atravessam verbatim).
///
/// Referenciados pela `const`, nunca pelo literal: renomear o valor a montante chega aqui
/// sozinho. Uma entrada **em falta** é apanhada pelo caminho B.
const CODIGOS_PROTOCOLO: &[&str] = &[
    code::UNAUTHENTICATED,
    code::UNSUPPORTED_VERSION,
    code::UNKNOWN_COMMAND,
    code::INVALID_ARGS,
    code::CONFIRMATION_REQUIRED,
    code::REFUSED_BY_POLICY,
    code::ENGINE_BUSY,
    code::LOAD_FAILED,
    code::BAD_REQUEST,
];

/// Os códigos de recusa do **runtime** (ADR-0023, congelados na GS1.6).
///
/// Aqui não há `const` a que se agarrar: `Rejected::code()` é um `match`, e o crate está
/// congelado. São literais — e é precisamente por isso que o caminho B os extrai do
/// `match` da fonte e os confronta nos dois sentidos.
const CODIGOS_RUNTIME: &[&str] = &[
    "no_show_loaded",
    "show_already_loaded",
    "preflight_failed",
    "not_armed",
    "seek_out_of_range",
    "in_error_state",
    "not_applicable",
];

fn uniao(nome: &str, doc: &str, valores: &[String]) -> String {
    let mut s = format!("/** {doc} */\nexport type {nome} =\n");
    for (i, v) in valores.iter().enumerate() {
        let fim = if i + 1 == valores.len() { ";" } else { "" };
        s.push_str(&format!("  | \"{v}\"{fim}\n"));
    }
    s
}

fn lista(nome: &str, tipo: &str, doc: &str, valores: &[String]) -> String {
    let itens =
        valores.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(", ");
    format!("/** {doc} */\nexport const {nome}: readonly {tipo}[] = [{itens}] as const;\n")
}

/// Gera o contrato. **Determinístico**: a mesma árvore produz sempre os mesmos bytes.
pub fn gerar_typescript() -> String {
    let mut s = String::new();

    s.push_str(
        "// ───────────────────────────────────────────────────────────────────────────\n\
         // GERADO — NÃO EDITAR À MÃO.\n\
         //\n\
         // Fonte de verdade: Rust (ADR-0027). Regenerar com:\n\
         //   cargo run -p led-console-bin --example gerar_contrato\n\
         //\n\
         // O gate `crates/led-console-bin/tests/contract_gate.rs` reprova se este ficheiro\n\
         // divergir do Rust — por edição manual ou por falta de regeneração.\n\
         // ───────────────────────────────────────────────────────────────────────────\n\n",
    );

    // ── EstadoUi ────────────────────────────────────────────────────────────
    let estados: Vec<String> = EstadoUi::ALL.iter().map(|e| e.as_str().to_string()).collect();
    s.push_str(&uniao(
        "EstadoUi",
        "Os estados que a UI apresenta. Nenhum e calculado no frontend (ADR-0026).",
        &estados,
    ));
    s.push('\n');
    s.push_str(&lista("ESTADOS_UI", "EstadoUi", "Todos os estados, na ordem do Rust.", &estados));
    s.push('\n');

    // Quais aprovam vem da **funcao Rust**, nao de uma regra reescrita aqui: se
    // `EstadoUi::aprova` mudar, esta lista muda com ela.
    let aprovam: Vec<String> = EstadoUi::ALL
        .iter()
        .filter(|e| e.aprova())
        .map(|e| e.as_str().to_string())
        .collect();
    s.push_str(&lista(
        "ESTADOS_QUE_APROVAM",
        "EstadoUi",
        "Derivado de `EstadoUi::aprova()`. So PASS aprova — e a regra vive no Rust.",
        &aprovam,
    ));
    s.push_str(
        "\nexport function aprova(e: EstadoUi): boolean {\n  \
         return (ESTADOS_QUE_APROVAM as readonly string[]).includes(e);\n}\n\n",
    );

    // ── Elo ─────────────────────────────────────────────────────────────────
    let elos: Vec<String> = Elo::ALL.iter().map(|e| e.as_str().to_string()).collect();
    s.push_str(&uniao(
        "Elo",
        "A cadeia de evidencia. Confirmar um elo NAO implica nenhum outro (ADR-0026).",
        &elos,
    ));
    s.push('\n');
    s.push_str(&lista(
        "ELOS_POR_FORCA",
        "Elo",
        "Do mais fraco ao mais forte. A ordem e do Rust e nao pode ser reordenada aqui.",
        &elos,
    ));
    s.push('\n');

    // ── State do daemon ─────────────────────────────────────────────────────
    let states: Vec<String> = State::ALL.iter().map(|e| e.as_str().to_string()).collect();
    s.push_str(&uniao(
        "DaemonState",
        "Os estados do transporte (ADR-0023, contrato congelado na GS1.6).",
        &states,
    ));
    s.push('\n');
    s.push_str(&lista("DAEMON_STATES", "DaemonState", "Todos os estados do daemon.", &states));
    s.push('\n');

    // ── Codigos de erro ─────────────────────────────────────────────────────
    let mut codigos: Vec<String> =
        CODIGOS_PROTOCOLO.iter().map(|c| (*c).to_string()).collect();
    codigos.extend(CODIGOS_RUNTIME.iter().map(|c| (*c).to_string()));
    s.push_str(&uniao(
        "CodigoErro",
        "Codigos enumerados, nunca string livre. Os do runtime atravessam verbatim.",
        &codigos,
    ));
    s.push('\n');

    // ── Rotas ───────────────────────────────────────────────────────────────
    s.push_str(
        "/** Uma rota da superficie do console. */\n\
         export interface Rota {\n  \
         readonly verbo: \"GET\" | \"POST\";\n  \
         readonly caminho: string;\n\
         }\n\n\
         /** A superficie COMPLETA. O browser nao tem outra origem (ADR-0026 §9-bis). */\n\
         export const ROTAS: readonly Rota[] = [\n",
    );
    for r in ROTAS {
        let v = match r.verbo {
            Verbo::Get => "GET",
            Verbo::Post => "POST",
        };
        s.push_str(&format!(
            "  {{ verbo: \"{v}\", caminho: \"{}\" }},\n",
            r.caminho
        ));
    }
    s.push_str("] as const;\n\n");

    // ── Semantica de nulo ───────────────────────────────────────────────────
    //
    // `Option<T>` atravessa como `T | null` EXPLICITO. Um campo ausente e um campo a `null`
    // sao indistinguiveis para quem le com `?.`, e a distincao aqui e semantica.
    s.push_str(
        "/**\n \
         * Um instantaneo com IDADE. `dado: null` e `staleMs: null` significam\n \
         * \"nunca houve\" — nunca \"idade zero\", que seria o zero artificial que o\n \
         * ADR-0026 proibe. O `| null` e explicito de proposito: um campo ausente e um\n \
         * campo a null nao podem ser confundidos.\n \
         */\n\
         export interface Instantaneo<T> {\n  \
         readonly dado: T | null;\n  \
         readonly estado: EstadoUi;\n  \
         readonly staleMs: number | null;\n\
         }\n\n\
         /**\n \
         * Uma resposta do IPC v1.\n \
         *\n \
         * `id: number | null` — `null` e a recusa que NAO se consegue atribuir (linha\n \
         * demasiado longa). Os clientes distinguem resposta de evento pela PRESENCA da\n \
         * chave `id`, nao pelo seu valor: um evento nao tem a chave.\n \
         */\n\
         export interface Resposta {\n  \
         readonly v: number;\n  \
         readonly id: number | null;\n  \
         readonly ok: boolean;\n  \
         readonly error?: { readonly code: CodigoErro; readonly detail: string };\n\
         }\n\n\
         /** Um evento assincrono. NAO tem `id` — e assim que se distingue de uma resposta. */\n\
         export interface Evento {\n  \
         readonly v: number;\n  \
         readonly async: true;\n  \
         readonly payload: unknown;\n\
         }\n",
    );

    s
}
