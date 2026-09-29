// `console.dropped` no cliente (ADR-0026 §13-bis, TD-014): o delta, o reinício e o tecto.

import { afterEach, describe, expect, it, vi } from "vitest";
import { deltaDescartes, iniciarPolling, INTERVALO_DESCARTES_MS } from "./descartes";

const T0 = 1_790_000_000_000;

describe("deltaDescartes", () => {
  it("primeira leitura: null — não há referência, e 0 afirmaria que nada se perdeu", () => {
    expect(deltaDescartes(null, { dropped: 7, since: T0 })).toBeNull();
  });

  it("mesmo arranque: a diferença", () => {
    expect(deltaDescartes({ dropped: 10, since: T0 }, { dropped: 25, since: T0 })).toBe(15);
    expect(deltaDescartes({ dropped: 25, since: T0 }, { dropped: 25, since: T0 })).toBe(0);
  });

  it("reinício com contador MAIOR que o anterior é detetado pelo `since`", () => {
    // O caso que a regra «contador que desce ⇒ reinício» deixaria passar: o console
    // reiniciou e perdeu MAIS do que antes. A diferença daria 40; o certo é 50 — tudo o
    // que o contador novo conta aconteceu depois do reinício.
    expect(deltaDescartes({ dropped: 10, since: T0 }, { dropped: 50, since: T0 + 5_000 })).toBe(50);
  });

  it("reinício com contador menor também é pelo `since`", () => {
    expect(deltaDescartes({ dropped: 90, since: T0 }, { dropped: 3, since: T0 + 5_000 })).toBe(3);
  });

  it("mesmo arranque e contador a descer: violação do contrato ⇒ null, nunca negativo", () => {
    expect(deltaDescartes({ dropped: 30, since: T0 }, { dropped: 12, since: T0 })).toBeNull();
  });
});

describe("tecto de frequência do polling (≤ 1 Hz)", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("a constante não passa de 1 Hz", () => {
    expect(INTERVALO_DESCARTES_MS).toBeGreaterThanOrEqual(1000);
  });

  it("uma rajada de perdas não gera mais pedidos: são guiados pelo relógio", async () => {
    vi.useFakeTimers();
    let dropped = 0;
    let pedidos = 0;
    const ler = async () => {
      pedidos += 1;
      dropped += 10_000; // uma rajada enorme entre cada leitura
      return { dropped, since: T0 };
    };
    const parar = iniciarPolling(ler, () => {}, INTERVALO_DESCARTES_MS);
    await vi.advanceTimersByTimeAsync(5_000);
    parar();
    // 1 imediato + 1 por segundo: 6 em 5 s. Nunca mais do que 1 + segundos decorridos.
    expect(pedidos).toBeLessThanOrEqual(1 + 5);
    expect(pedidos).toBeGreaterThan(1); // e o polling corre mesmo — não é um gate vácuo
  });
});
