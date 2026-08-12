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
 * Um evento tal como chegou — **sem interpretação**.
 *
 * O `Evento` do contrato declara `payload: unknown`, e é honesto: o payload tem sete
 * formas (`transitioned`, `show_loaded`, `show_unloaded`, `position_changed`,
 * `reached_end`, `faulted`, `fault_cleared`), todas produzidas por `event_to_json` no
 * daemon — mas **nenhuma está tipada no contrato**.
 *
 * Enquanto não estiver, esta UI mostra a linha **crua**. Não inventa campos, não adivinha
 * formas, e não escreve à mão o que o gerador ainda não emite. Tipar o payload é a fatia
 * seguinte, e segue o mesmo caminho do `EstadoDoDaemon`: Rust → contrato gerado → frontend.
 */
export interface EventoCru {
  /** Monotónico, só para dar ordem estável na lista. Não vem do backend. */
  readonly seq: number;
  /** O JSON do `payload`, tal como veio no fio. */
  readonly linha: string;
}

/**
 * `GET /api/events` — o fluxo SSE.
 *
 * **Uma ligação por browser, e o console faz o fan-out** a partir de uma única subscrição
 * no daemon (ADR-0026 §4). Reconectar aqui **não** abre nada a montante: o `EventSource`
 * religa-se sozinho, e o supervisor do console mantém a sua subscrição independente disso.
 *
 * Devolve a função de cancelamento. Sem ela, uma tela que desmonta deixaria a ligação viva.
 */
export function subscreverEventos(
  aoEvento: (e: EventoCru) => void,
  aoLigacao: (ligado: boolean) => void,
): () => void {
  let seq = 0;
  const fonte = new EventSource("/api/events");

  fonte.onopen = () => aoLigacao(true);

  // O `EventSource` religa-se sozinho; `onerror` é o sinal de que está em baixo AGORA.
  // Não o tratamos como fim — tratá-lo assim faria a UI dizer "offline" para sempre depois
  // de um soluço de rede.
  fonte.onerror = () => aoLigacao(false);

  fonte.onmessage = (e: MessageEvent<string>) => {
    seq += 1;
    aoEvento({ seq, linha: e.data });
  };

  return () => fonte.close();
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
