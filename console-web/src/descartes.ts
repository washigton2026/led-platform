// ─────────────────────────────────────────────────────────────────────────────
// `console.dropped` no cliente (ADR-0026 §13-bis, TD-014).
//
// O console expõe um contador GLOBAL e cumulativo (`dropped`) e o seu instante de
// arranque (`since`). O DELTA é deste lado — e é aqui que a regra de reinício vive:
// quem diz que o console reiniciou é o `since`, nunca a direção do número.
// ─────────────────────────────────────────────────────────────────────────────

import type { EstadoDescartes } from "../../crates/led-console-bin/contract/lumyx-contract.generated";

/**
 * Cadência do polling de `/api/dropped`: **no máximo 1 Hz** (§13-bis). A rota não empurra
 * nada; o tecto é esta constante, e uma rajada de perdas não gera um pedido a mais.
 */
export const INTERVALO_DESCARTES_MS = 1000;

/**
 * Quantos eventos se perderam **entre a leitura anterior e esta**.
 *
 * - Primeira leitura: `null` — não há referência, e inventar `0` seria afirmar que nada se
 *   perdeu.
 * - `since` **mudou**: o console reiniciou. O delta desta leitura é o `dropped` inteiro,
 *   porque tudo o que ele conta aconteceu depois do reinício.
 * - Mesmo `since`: a diferença de `dropped`.
 * - Mesmo `since` e `dropped` **desceu**: o contrato diz que isso não acontece. `null`
 *   (não medido) em vez de um número negativo, ou de adivinhar um reinício — a regra
 *   «contador que desce ⇒ reinício» está proibida, porque um reinício seguido de mais
 *   perdas do que antes daria um número MAIOR e passaria despercebido.
 */
export function deltaDescartes(
  anterior: EstadoDescartes | null,
  atual: EstadoDescartes,
): number | null {
  if (anterior === null) return null;
  if (atual.since !== anterior.since) return atual.dropped;
  if (atual.dropped < anterior.dropped) return null;
  return atual.dropped - anterior.dropped;
}

/**
 * Pergunta já, e depois a cada `intervaloMs`. Devolve a função que pára.
 *
 * Existe à parte para o tecto ser **testável**: os pedidos são guiados pelo relógio, nunca
 * pelas perdas — e isso só se prova a contar chamadas com o tempo controlado.
 */
export function iniciarPolling<T>(
  ler: () => Promise<T>,
  aplicar: (valor: T) => void,
  intervaloMs: number,
): () => void {
  let vivo = true;
  const perguntar = async () => {
    const v = await ler();
    if (vivo) aplicar(v);
  };
  void perguntar();
  const t = setInterval(() => void perguntar(), intervaloMs);
  return () => {
    vivo = false;
    clearInterval(t);
  };
}
