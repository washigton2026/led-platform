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
/// As causas de `position_changed` (`PositionCause::as_str`, ADR-0023 F2).
///
/// **Três causas para quatro origens, de propósito:** `pause` e `tick` são ambos
/// `advanced` — pausar *avança* até ao instante da pausa. Uma quarta variante descreveria o
/// *comando*, não a *causa*.
const CAUSAS_DE_POSICAO: &[&str] = &["advanced", "sought", "reset"];

/// Os códigos de falha (`FaultCode::as_str`).
const CODIGOS_DE_FALHA: &[&str] =
    &["device_lost", "source_failed", "policy_violation", "output_stalled"];

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
         }\n\n",
    );

    // ── O payload dos eventos SSE ───────────────────────────────────────────
    //
    // Sete formas, todas produzidas por `event_to_json` no daemon. Cada uma discrimina-se
    // pelo campo `event`, o que faz delas uma uniao discriminada em TypeScript: um `switch`
    // sobre `event` estreita o tipo, e o compilador obriga a tratar as sete.
    let causas: Vec<String> = CAUSAS_DE_POSICAO.iter().map(|c| (*c).to_string()).collect();
    s.push_str(&uniao(
        "CausaDePosicao",
        "Porque a posicao mudou. `pause` e `tick` sao ambos `advanced` (ADR-0023 F2).",
        &causas,
    ));
    s.push('\n');

    let falhas: Vec<String> = CODIGOS_DE_FALHA.iter().map(|c| (*c).to_string()).collect();
    s.push_str(&uniao("CodigoDeFalha", "Os codigos de falha do runtime.", &falhas));
    s.push('\n');

    s.push_str(
        "/**\n \
         * O `payload` de um evento SSE — as SETE formas que `event_to_json` produz.\n \
         *\n \
         * Uniao DISCRIMINADA por `event`: um `switch` sobre esse campo estreita o tipo, e\n \
         * o compilador obriga a tratar todas. Uma forma nova no daemon deixa o `switch`\n \
         * incompleto e o frontend nao compila — que e exactamente o efeito desejado.\n \
         *\n \
         * `t_ms` e o instante do DAEMON, nao do browser: dois relogios diferentes, e este\n \
         * e o que ordena os eventos entre si.\n \
         */\n\
         export type EventoPayload =\n  \
         | { readonly t_ms: number; readonly event: \"transitioned\"; readonly from: DaemonState; readonly to: DaemonState }\n  \
         | { readonly t_ms: number; readonly event: \"show_loaded\"; readonly show_id: number }\n  \
         | { readonly t_ms: number; readonly event: \"show_unloaded\"; readonly show_id: number }\n  \
         | { readonly t_ms: number; readonly event: \"position_changed\"; readonly ms: number; readonly cause: CausaDePosicao }\n  \
         | { readonly t_ms: number; readonly event: \"reached_end\" }\n  \
         | { readonly t_ms: number; readonly event: \"faulted\"; readonly code: CodigoDeFalha }\n  \
         | { readonly t_ms: number; readonly event: \"fault_cleared\" };\n\n\
         /** Um evento SSE completo: o envelope do IPC v1 com o payload tipado. */\n\
         export interface EventoTipado {\n  \
         readonly v: number;\n  \
         readonly async: true;\n  \
         readonly payload: EventoPayload;\n\
         }\n\n",
    );

    // ── O corpo de GET /api/state ───────────────────────────────────────────
    //
    // Os nomes sao os do FIO, em snake_case, porque e assim que o daemon os escreve
    // (`Cmd::Status` em `server.rs`) e o console repassa a linha **verbatim**. Renomea-los
    // para camelCase seria uma traducao — e o console traduz transporte, nao vocabulario
    // (ADR-0026 §15). O `Instantaneo` usa camelCase porque NAO e um tipo de fio: e
    // vocabulario da UI. Os dois nao seguem a mesma convencao de propósito.
    s.push_str(
        "/**\n \
         * O corpo de `GET /api/state` — a resposta do comando `status` do IPC v1,\n \
         * repassada VERBATIM pelo console.\n \
         *\n \
         * Cada campo tem produtor real: o laco publica um `Snapshot` a cada tick\n \
         * (`run.rs`) e `Cmd::Status` le-o (`server.rs`). Nenhum e calculado no console\n \
         * nem no frontend.\n \
         *\n \
         * `show_id: number | null` — `null` significa SEM SHOW CARREGADO, nunca `0`.\n \
         * A distincao e do Rust (`Option<u64>`) e sobrevive ate aqui de proposito.\n \
         */\n\
         export interface EstadoDoDaemon {\n  \
         readonly v: number;\n  \
         readonly id: number;\n  \
         readonly ok: true;\n  \
         readonly state: DaemonState;\n  \
         readonly position_ms: number;\n  \
         readonly duration_ms: number;\n  \
         readonly ticks: number;\n  \
         readonly show_id: number | null;\n  \
         readonly outputs: readonly SaidaPorAlvo[];\n\
         }\n\n",
    );

    // ── A contabilidade por no (ADR-0029 §8) ────────────────────────────────
    //
    // O daemon envia N nos porque o rig tem N nos. Um AGREGADO nao distingue cinco a
    // funcionar de quatro a funcionar e um morto — e e essa distincao que o §5 obriga a
    // manter observavel ate ao operador.
    s.push_str(
        "/**\n \
         * A contabilidade de UM no da saida (ADR-0029 §8).\n \
         *\n \
         * `addr` e o endereco do no, e existe para que a perda seja ATRIBUIVEL: com cinco\n \
         * robos, \"houve erros\" sem dizer de quem manda procurar em cinco sitios.\n \
         *\n \
         * `frames` e `errors` sao deste no e so deste no. Um `sendto` com sucesso NAO\n \
         * prova que o controlador recebeu, e menos ainda que o LED acendeu — a cadeia de\n \
         * evidencia do ADR-0026 §8 continua intacta: isto e OBSERVABILIDADE, nao\n \
         * EVIDENCIA FISICA.\n \
         *\n \
         * Lista `outputs` VAZIA significa SEM SAIDA CONFIGURADA. Nao e o mesmo que uma\n \
         * saida parada, e por isso nao se fabrica uma entrada com zeros.\n \
         */\n\
         export interface SaidaPorAlvo {\n  \
         readonly addr: string;\n  \
         readonly frames: number;\n  \
         readonly errors: number;\n\
         }\n\n",
    );

    // ── /api/upstream ────────────────────────────────────────────────────────
    //
    // O PRIMEIRO corpo JSON que o console AUTORA. Todos os outros de sucesso sao a linha
    // do daemon repassada verbatim — e e por isso que este nao leva `v` nem `ok`: esses
    // pertencem ao envelope do IPC v1 e sao escritos pelo daemon em `proto.rs`. Um corpo
    // do console que os incluisse afirmaria uma proveniencia que nao tem.
    s.push_str(
        "/**\n \
         * O corpo de `GET /api/upstream` (ADR-0026 §9-quinquies).\n \
         *\n \
         * `upstream: true` significa EXATAMENTE: existe agora uma subscricao\n \
         * estabelecida entre o console e o daemon. Nada mais.\n \
         *\n \
         * NAO significa HEALTHY, STREAMING_READY, OUTPUT_OK, NETWORK_OK, HARDWARE_OK,\n \
         * LED_OK nem SHOW_RUNNING — nenhuma dessas conclusoes e derivavel daqui, e a\n \
         * cadeia de evidencia do ADR-0026 §8 continua intacta.\n \
         *\n \
         * E NAO e o estado da ligacao SSE do browser: o `EventSource` fica aberto com o\n \
         * daemon morto, porque o console o mantem vivo com keep-alive. Sao camadas\n \
         * diferentes, e usar uma como proxy da outra e o defeito que esta rota corrige.\n \
         *\n \
         * Sem `v`, sem `ok`, sem `id`: este corpo nao atravessa o IPC v1, e nao tem modo\n \
         * de falha proprio (a medicao e a leitura de um booleano local).\n \
         */\n\
         export interface EstadoUpstream {\n  \
         readonly upstream: boolean;\n\
         }\n\n",
    );

    // ── Argumentos dos comandos (ADR-0027, Emenda 2) ─────────────────────────
    //
    // Ate aqui o contrato so descrevia o que se RECEBE. Os argumentos de um comando —
    // o que o browser ENVIA — nao tinham tipo nenhum, e a assimetria era invisivel
    // porque o unico comando com argumentos era o `seek` (`{to_ms}`, dois caracteres
    // dificeis de errar). Com o `load` deixa de ser trivial.
    //
    // Cobre-se so o que a superficie HTTP expoe. `hello`, `subscribe`, `ping` e
    // `shutdown` estao em `NUNCA_EXPOSTOS` (surface.rs): descrever os argumentos de
    // comandos que o browser nao pode enviar seria alargar o contrato para la desta
    // fronteira.
    //
    // Os nomes dos campos sao escritos aqui — nao ha reflexao em runtime sobre campos de
    // um enum. E por isso que existe o caminho B: `tests/contract_gate.rs` extrai-os do
    // TEXTO-FONTE do `enum Cmd` e reprova se algum ficar de fora. A mesma disciplina que
    // o `EstadoDoDaemon` ja usa contra o arm `Cmd::Status`.
    s.push_str(
        "/**\n \
         * Os argumentos de `POST /api/transport/load` — `Cmd::Load` do IPC v1.\n \
         *\n \
         * `path` e um caminho de ficheiro. NAO ha catalogo de shows: nenhuma rota os\n \
         * lista, e inventar uma lista no console seria a segunda fonte de verdade que o\n \
         * ADR-0026 §15 proibe. O daemon recusa o que nao existir com `load_failed`, e o\n \
         * `detail` traz o erro real do loader.\n \
         *\n \
         * `assume_integrity` NAO e uma opcao de conveniencia. Faz DUAS coisas: afirma a\n \
         * integridade (`Integrity::AssumedByOperator`) e dispara o pre-voo e o `Arm`. Sem\n \
         * ela o show fica em `loaded` e o `play` seguinte devolve `not_armed`.\n \
         *\n \
         * E o daemon NUNCA verifica: `pixel_hash` exige o show inteiro em RAM e hash em\n \
         * fluxo nao existe (GS2). Por isso `Integrity` e um enum e nao um booleano — para\n \
         * que \"assumido\" e \"verificado\" nao fiquem indistinguiveis. A UI expoe isto\n \
         * como DUAS accoes nomeadas, nunca como caixa (ADR-0028 D8).\n \
         */\n\
         export interface ArgsLoad {\n  \
         readonly path: string;\n  \
         readonly assume_integrity: boolean;\n\
         }\n\n\
         /** Os argumentos de `POST /api/transport/seek` — `Cmd::Seek` do IPC v1. */\n\
         export interface ArgsSeek {\n  \
         readonly to_ms: number;\n\
         }\n",
    );

    s
}
