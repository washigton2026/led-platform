// ─────────────────────────────────────────────────────────────────────────────
// O ÚNICO sítio onde esta aplicação faz `fetch`.
//
// Se aparecer um segundo, a fronteira deixa de ser auditável: hoje basta ler este
// ficheiro para saber tudo o que a UI pede ao backend.
//
// Os tipos vêm do contrato **gerado** (ADR-0027), importados do ficheiro real — não
// copiados. Uma cópia seria a segunda fonte de verdade que o ADR-0026 §15 proíbe.
// ─────────────────────────────────────────────────────────────────────────────

import type {
  CodigoErro,
  EstadoDoDaemon,
} from "../../../crates/led-console-bin/contract/lumyx-contract.generated";

/** Reexportado do contrato **gerado** — nunca redeclarado. */
export type { EstadoDoDaemon };

/**
 * O que a UI sabe sobre a ligação. **Dois estados, e nenhum inventado.**
 *
 * Não existe "healthy", "degraded" nem "connecting": nada no backend os produz. O que
 * existe é uma resposta com dados, ou uma falha — e a falha traz o código real.
 */
export type Ligacao =
  | { readonly tipo: "dados"; readonly estado: EstadoDoDaemon }
  | { readonly tipo: "offline"; readonly code: string; readonly detail: string };

/** O corpo de erro que o console emite (`http.rs::Saida::erro`). */
interface CorpoDeErro {
  readonly ok: false;
  readonly error: { readonly code: CodigoErro | string; readonly detail: string };
}

function pareceErro(x: unknown): x is CorpoDeErro {
  if (typeof x !== "object" || x === null) return false;
  const e = (x as { error?: unknown }).error;
  return typeof e === "object" && e !== null && typeof (e as { code?: unknown }).code === "string";
}

/**
 * `GET /api/state`.
 *
 * **Nunca inventa um estado.** Se o daemon não responder, o console devolve 503 com
 * `console.daemon_offline` (ADR-0026 §7: OFFLINE é um estado, não um erro), e é esse
 * código que sobe — não um booleano nosso.
 *
 * Um `fetch` que rebenta (console em baixo, rede) também é offline, mas com um código
 * **do cliente**, prefixado `console-web.` para nunca se confundir com um do backend.
 */
export async function lerEstado(): Promise<Ligacao> {
  let r: Response;
  try {
    r = await fetch("/api/state", { headers: { accept: "application/json" } });
  } catch (e) {
    return {
      tipo: "offline",
      code: "console-web.unreachable",
      detail: e instanceof Error ? e.message : String(e),
    };
  }

  let corpo: unknown;
  try {
    corpo = await r.json();
  } catch {
    return {
      tipo: "offline",
      code: "console-web.bad_response",
      detail: `HTTP ${r.status} sem corpo JSON`,
    };
  }

  if (!r.ok) {
    // O código do backend atravessa VERBATIM. O console já o preservou desde o daemon
    // (ADR-0026 §6); reescrevê-lo aqui apagaria a razão da falha.
    if (pareceErro(corpo)) {
      return { tipo: "offline", code: corpo.error.code, detail: corpo.error.detail };
    }
    return { tipo: "offline", code: "console-web.bad_response", detail: `HTTP ${r.status}` };
  }

  return { tipo: "dados", estado: corpo as EstadoDoDaemon };
}
