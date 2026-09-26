// ─────────────────────────────────────────────────────────────────────────────
// Testes da lógica pura de eventos. Sem duplos, sem rede, sem React.
//
// Os payloads abaixo NÃO são fixtures inventadas: são as formas exactas que o
// `event_to_json` do daemon produz, e o contrato gerado impõe-lhes o tipo. Se o
// backend mudar uma forma, o contrato é regenerado e estes literais deixam de
// compilar — o teste morre com o contrato, que é o que se quer.
// ─────────────────────────────────────────────────────────────────────────────

import { describe, expect, it } from "vitest";
import { descreveEvento, ehProgresso } from "./eventos";
import type { EventoPayload } from "./transport/api";

describe("descreveEvento", () => {
  it("descreve as sete formas que o daemon emite", () => {
    const casos: ReadonlyArray<readonly [EventoPayload, string]> = [
      [{ t_ms: 1, event: "transitioned", from: "ready", to: "playing" }, "ready → playing"],
      [{ t_ms: 2, event: "show_loaded", show_id: 7 }, "show 7 carregado"],
      [{ t_ms: 3, event: "show_unloaded", show_id: 7 }, "show 7 descarregado"],
      [
        { t_ms: 4, event: "position_changed", ms: 4000, cause: "sought" },
        "posição 4000 ms (sought)",
      ],
      [{ t_ms: 5, event: "reached_end" }, "fim do show"],
      [{ t_ms: 6, event: "faulted", code: "device_lost" }, "falha: device_lost"],
      [{ t_ms: 7, event: "fault_cleared" }, "falha resolvida"],
    ];

    for (const [payload, esperado] of casos) {
      expect(descreveEvento(payload)).toBe(esperado);
    }

    // As sete do ADR-0023. Se o daemon ganhar uma oitava, o `switch` de
    // `descreveEvento` deixa de compilar — mas esta contagem garante que este
    // teste também não fica para trás em silêncio.
    expect(casos).toHaveLength(7);
  });

  it("preserva a causa, que o /api/state nao traz", () => {
    // `advanced`, `sought` e `reset` distinguem-se só aqui: o snapshot dá a posição,
    // nunca o motivo de ela ter mudado (ADR-0023 F2).
    const causas = ["advanced", "sought", "reset"] as const;
    for (const cause of causas) {
      expect(descreveEvento({ t_ms: 0, event: "position_changed", ms: 1, cause })).toContain(
        cause,
      );
    }
  });
});

describe("ehProgresso", () => {
  it("so `position_changed` e progresso", () => {
    expect(ehProgresso({ t_ms: 0, event: "position_changed", ms: 1, cause: "advanced" })).toBe(
      true,
    );
    expect(ehProgresso({ t_ms: 0, event: "transitioned", from: "idle", to: "loaded" })).toBe(
      false,
    );
    expect(ehProgresso({ t_ms: 0, event: "reached_end" })).toBe(false);
    expect(ehProgresso({ t_ms: 0, event: "faulted", code: "output_stalled" })).toBe(false);
  });

  it("um evento ilegivel NAO conta como progresso", () => {
    // Se contasse, uma linha que não analisa seria colapsada no contador de posição e
    // desapareceria do registo — exactamente o que não pode acontecer com um evento que
    // não se percebeu.
    expect(ehProgresso(null)).toBe(false);
  });
});
