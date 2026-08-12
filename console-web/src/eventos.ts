// ─────────────────────────────────────────────────────────────────────────────
// Lógica **pura** sobre eventos. Sem React, sem rede, sem estado.
//
// Vive separada da renderização para poder ser testada sem montar nada — e porque
// "como se descreve um evento" é uma decisão de produto, não de layout.
// ─────────────────────────────────────────────────────────────────────────────

import type { EventoPayload } from "./transport/api";

/**
 * Traduz um evento para uma linha legível.
 *
 * `switch` exaustivo **sem `default`**: se o daemon ganhar uma forma nova e o contrato for
 * regenerado, esta função deixa de compilar. É esse o alarme — obriga alguém a decidir o
 * que mostrar, em vez de o evento cair num ramo genérico e desaparecer do ecrã.
 */
export function descreveEvento(p: EventoPayload): string {
  switch (p.event) {
    case "transitioned":
      return `${p.from} → ${p.to}`;
    case "show_loaded":
      return `show ${p.show_id} carregado`;
    case "show_unloaded":
      return `show ${p.show_id} descarregado`;
    case "position_changed":
      return `posição ${p.ms} ms (${p.cause})`;
    case "reached_end":
      return "fim do show";
    case "faulted":
      return `falha: ${p.code}`;
    case "fault_cleared":
      return "falha resolvida";
  }
}

/**
 * É progresso (posição a avançar) ou uma transição?
 *
 * `position_changed` chega a cada tick (~20 ms): um show de 8 s produz 401 desses contra 3
 * transições. Separar não é estética — sem isto, o que importa nunca está no ecrã.
 */
export function ehProgresso(p: EventoPayload | null): boolean {
  return p?.event === "position_changed";
}
