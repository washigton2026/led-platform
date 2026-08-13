// ─────────────────────────────────────────────────────────────────────────────
// O ÚNICO sítio onde esta aplicação faz `fetch`.
//
// Se aparecer um segundo, a fronteira deixa de ser auditável: hoje basta ler este
// ficheiro para saber tudo o que a UI pede ao backend.
//
// Os tipos vêm do contrato **gerado** (ADR-0027), importados do ficheiro real — não
// copiados. Uma cópia seria a segunda fonte de verdade que o ADR-0026 §15 proíbe.
// ─────────────────────────────────────────────────────────────────────────────

import {
  ROTAS,
  type CodigoErro,
  type EstadoDoDaemon,
  type EstadoUpstream,
  type EventoPayload,
  type EventoTipado,
} from "../../../crates/led-console-bin/contract/lumyx-contract.generated";

/** Reexportados do contrato **gerado** — nunca redeclarados. */
export type { EstadoDoDaemon, EventoPayload };

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
 * Extrai `code`/`detail` de uma resposta de erro — **função pura**, para poder ser testada
 * sem rede e sem duplos.
 *
 * O `code` do backend atravessa VERBATIM: o console já o preservou desde o daemon
 * (ADR-0026 §6), e reescrevê-lo aqui apagaria a razão da falha. Só quando o corpo é
 * ilegível é que se usa um código **do cliente**, prefixado `console-web.` para nunca se
 * confundir com um do backend.
 */
export function interpretarErro(status: number, texto: string): { code: string; detail: string } {
  try {
    const j: unknown = JSON.parse(texto);
    if (pareceErro(j)) return { code: j.error.code, detail: j.error.detail };
  } catch {
    // corpo ilegível: cai no genérico, sem inventar um código do backend
  }
  return { code: "console-web.bad_response", detail: `HTTP ${status}` };
}

/**
 * Um evento recebido, com o payload **tipado pelo contrato gerado**.
 *
 * As sete formas (`transitioned`, `show_loaded`, `show_unloaded`, `position_changed`,
 * `reached_end`, `faulted`, `fault_cleared`) são uma união discriminada por `event`, gerada
 * a partir de `event_to_json` — o produtor real — e verificada por um gate que reprova se
 * o daemon emitir uma forma que o contrato desconhece.
 *
 * A linha crua fica **sempre**: se o JSON não analisar, é ela que se mostra. Um evento
 * ilegível é informação; escondê-lo não é.
 */
export interface EventoCru {
  /** Monotónico, só para dar ordem estável na lista. **Não vem do backend.** */
  readonly seq: number;
  /** O payload **tipado**, quando a linha é analisável. */
  readonly payload: EventoPayload | null;
  /** A linha crua. Fica sempre — é o que se mostra quando o payload não analisa. */
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
    // A linha crua fica SEMPRE. Se o JSON não analisar, mostra-se o que veio em vez de
    // deitar fora o evento: um evento ilegível é informação, e escondê-lo não é.
    let payload: EventoPayload | null = null;
    try {
      const env = JSON.parse(e.data) as EventoTipado;
      // `payload` vem do backend e o contrato descreve-o; a asserção é o limite da
      // fronteira, não uma invenção — a alternativa seria validar aqui a forma que o
      // gerador já garante, e isso seria a segunda fonte de verdade.
      payload = env.payload ?? null;
    } catch {
      payload = null;
    }
    aoEvento({ seq, payload, linha: e.data });
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
    const { code, detail } = interpretarErro(r.status, JSON.stringify(corpo));
    return { tipo: "offline", code, detail };
  }

  return { tipo: "dados", estado: corpo as EstadoDoDaemon };
}

/**
 * `GET /api/upstream` — **existe subscrição console→daemon agora?** (ADR-0026 §9-quinquies)
 *
 * Isto **não** é o estado do `EventSource`. O `subscreverEventos` acima mede
 * *browser→console*; esta função mede *console→daemon*, que é outro elo e cai noutro
 * momento. O console mantém o SSE vivo com keep-alive, portanto a ligação do browser
 * continua aberta com o daemon morto — e foi assim que o ecrã chegou a afirmar fluxo
 * sobre silêncio.
 *
 * Devolve `null` para **não medido**: console inalcançável ou corpo ilegível. Nunca
 * `false`, que seria afirmar que a subscrição não existe quando o que não houve foi
 * resposta. É a mesma distinção que o `Ligacao | null` já faz do outro lado.
 */
export async function lerUpstream(): Promise<boolean | null> {
  const rota = ROTAS.find((r) => r.caminho === "/api/upstream" && r.verbo === "GET");
  if (!rota) return null; // o contrato não declara a rota: não há nada a medir
  try {
    const r = await fetch(rota.caminho, { headers: { accept: "application/json" } });
    if (!r.ok) return null;
    // O corpo é `{upstream: boolean}` e mais nada — sem `v`/`ok`/`id`, porque não
    // atravessa o IPC v1. A asserção é o limite da fronteira: o contrato descreve a
    // forma, e revalidá-la aqui seria a segunda fonte de verdade do ADR-0026 §15.
    const corpo = (await r.json()) as EstadoUpstream;
    return typeof corpo.upstream === "boolean" ? corpo.upstream : null;
  } catch {
    return null;
  }
}

// ── Comandos de transporte ───────────────────────────────────────────────────

/**
 * Os comandos que esta UI expõe: **transporte**, e só.
 *
 * A linha não é "destrutivo vs seguro" — é **mover no tempo** contra **mudar o que está
 * carregado**. O ADR-0023 decisão 3 é explícito: `Stop` e `Pause` NÃO apagam o palco; em
 * `paused`/`stopped` o heartbeat continua a reenviar o último quadro válido. Por isso
 * `stop` é transporte, não destruição.
 *
 * `load` e `unload` mudam o show carregado, e `load` precisa de um caminho de ficheiro —
 * uma superfície de entrada que pertence à gestão de shows, não a esta fatia.
 */
export const TRANSPORTE = ["play", "pause", "stop", "seek"] as const;
export type ComandoTransporte = (typeof TRANSPORTE)[number];

/**
 * O caminho de um comando, **derivado do contrato**.
 *
 * Não há string de rota escrita à mão aqui: se o console mudar um caminho, o contrato é
 * regenerado e isto acompanha. Um comando sem rota correspondente é um erro de programação,
 * e rebenta no arranque em vez de dar 404 em produção.
 */
function caminhoDe(cmd: ComandoTransporte): string {
  const sufixo = `/api/transport/${cmd}`;
  const rota = ROTAS.find((r) => r.caminho === sufixo && r.verbo === "POST");
  if (!rota) throw new Error(`sem rota POST para \`${cmd}\` no contrato`);
  return rota.caminho;
}

/** O que aconteceu a um comando. O `code` de uma recusa é o do daemon, verbatim. */
export type Resultado =
  | { readonly tipo: "aceite"; readonly cmd: ComandoTransporte; readonly corpo: string }
  | {
      readonly tipo: "recusado";
      readonly cmd: ComandoTransporte;
      readonly code: string;
      readonly detail: string;
    };

/**
 * Envia um comando de transporte.
 *
 * **A UI não decide se o comando é válido.** Quem decide é a máquina de estados do
 * ADR-0023, e ela tem 80 pares estado×comando. Desactivar botões consoante o estado seria
 * reimplementar essa matriz no browser — a segunda fonte de verdade que o ADR-0026 §15
 * proíbe, e que divergiria no dia em que a matriz mudasse.
 *
 * Por isso os botões estão **sempre activos**, e o que se mostra é a resposta REAL: um
 * `play` sem show devolve `no_show_loaded`, e é isso que o operador vê.
 */
export async function comandar(cmd: ComandoTransporte, args?: object): Promise<Resultado> {
  const corpo = args === undefined ? "" : JSON.stringify(args);
  let r: Response;
  try {
    r = await fetch(caminhoDe(cmd), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: corpo,
    });
  } catch (e) {
    return {
      tipo: "recusado",
      cmd,
      code: "console-web.unreachable",
      detail: e instanceof Error ? e.message : String(e),
    };
  }

  const texto = await r.text();
  if (r.ok) return { tipo: "aceite", cmd, corpo: texto };

  const { code, detail } = interpretarErro(r.status, texto);
  return { tipo: "recusado", cmd, code, detail };
}
