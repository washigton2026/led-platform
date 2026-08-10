// ───────────────────────────────────────────────────────────────────────────
// GERADO — NÃO EDITAR À MÃO.
//
// Fonte de verdade: Rust (ADR-0027). Regenerar com:
//   cargo run -p led-console-bin --example gerar_contrato
//
// O gate `crates/led-console-bin/tests/contract_gate.rs` reprova se este ficheiro
// divergir do Rust — por edição manual ou por falta de regeneração.
// ───────────────────────────────────────────────────────────────────────────

/** Os estados que a UI apresenta. Nenhum e calculado no frontend (ADR-0026). */
export type EstadoUi =
  | "PASS"
  | "FAIL"
  | "NOT_MEASURED"
  | "BLOCKED"
  | "RUNNING"
  | "OFFLINE"
  | "DEGRADED"
  | "UNKNOWN"
  | "SIMULATION";

/** Todos os estados, na ordem do Rust. */
export const ESTADOS_UI: readonly EstadoUi[] = ["PASS", "FAIL", "NOT_MEASURED", "BLOCKED", "RUNNING", "OFFLINE", "DEGRADED", "UNKNOWN", "SIMULATION"] as const;

/** Derivado de `EstadoUi::aprova()`. So PASS aprova — e a regra vive no Rust. */
export const ESTADOS_QUE_APROVAM: readonly EstadoUi[] = ["PASS"] as const;

export function aprova(e: EstadoUi): boolean {
  return (ESTADOS_QUE_APROVAM as readonly string[]).includes(e);
}

/** A cadeia de evidencia. Confirmar um elo NAO implica nenhum outro (ADR-0026). */
export type Elo =
  | "software_sent"
  | "network_delivered"
  | "controller_received"
  | "controller_acknowledged"
  | "led_verified";

/** Do mais fraco ao mais forte. A ordem e do Rust e nao pode ser reordenada aqui. */
export const ELOS_POR_FORCA: readonly Elo[] = ["software_sent", "network_delivered", "controller_received", "controller_acknowledged", "led_verified"] as const;

/** Os estados do transporte (ADR-0023, contrato congelado na GS1.6). */
export type DaemonState =
  | "idle"
  | "loaded"
  | "ready"
  | "playing"
  | "paused"
  | "stopped"
  | "finished"
  | "error";

/** Todos os estados do daemon. */
export const DAEMON_STATES: readonly DaemonState[] = ["idle", "loaded", "ready", "playing", "paused", "stopped", "finished", "error"] as const;

/** Codigos enumerados, nunca string livre. Os do runtime atravessam verbatim. */
export type CodigoErro =
  | "unauthenticated"
  | "unsupported_version"
  | "unknown_command"
  | "invalid_args"
  | "confirmation_required"
  | "refused_by_policy"
  | "engine_busy"
  | "load_failed"
  | "bad_request"
  | "no_show_loaded"
  | "show_already_loaded"
  | "preflight_failed"
  | "not_armed"
  | "seek_out_of_range"
  | "in_error_state"
  | "not_applicable";

/** Uma rota da superficie do console. */
export interface Rota {
  readonly verbo: "GET" | "POST";
  readonly caminho: string;
}

/** A superficie COMPLETA. O browser nao tem outra origem (ADR-0026 §9-bis). */
export const ROTAS: readonly Rota[] = [
  { verbo: "GET", caminho: "/api/state" },
  { verbo: "GET", caminho: "/api/version" },
  { verbo: "GET", caminho: "/api/events" },
  { verbo: "GET", caminho: "/api/profiles" },
  { verbo: "GET", caminho: "/api/metrics" },
  { verbo: "POST", caminho: "/api/transport/load" },
  { verbo: "POST", caminho: "/api/transport/unload" },
  { verbo: "POST", caminho: "/api/transport/play" },
  { verbo: "POST", caminho: "/api/transport/pause" },
  { verbo: "POST", caminho: "/api/transport/stop" },
  { verbo: "POST", caminho: "/api/transport/seek" },
] as const;

/**
 * Um instantaneo com IDADE. `dado: null` e `staleMs: null` significam
 * "nunca houve" — nunca "idade zero", que seria o zero artificial que o
 * ADR-0026 proibe. O `| null` e explicito de proposito: um campo ausente e um
 * campo a null nao podem ser confundidos.
 */
export interface Instantaneo<T> {
  readonly dado: T | null;
  readonly estado: EstadoUi;
  readonly staleMs: number | null;
}

/**
 * Uma resposta do IPC v1.
 *
 * `id: number | null` — `null` e a recusa que NAO se consegue atribuir (linha
 * demasiado longa). Os clientes distinguem resposta de evento pela PRESENCA da
 * chave `id`, nao pelo seu valor: um evento nao tem a chave.
 */
export interface Resposta {
  readonly v: number;
  readonly id: number | null;
  readonly ok: boolean;
  readonly error?: { readonly code: CodigoErro; readonly detail: string };
}

/** Um evento assincrono. NAO tem `id` — e assim que se distingue de uma resposta. */
export interface Evento {
  readonly v: number;
  readonly async: true;
  readonly payload: unknown;
}
