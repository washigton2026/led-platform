// ─────────────────────────────────────────────────────────────────────────────
// A protecção contra o falso-verde do F-01.
//
// O defeito que estes testes existem para impedir tem um nome exacto:
//
//     upstream = sseAberto
//
// Ele foi observado ao vivo — o daemon encerrou, a subscrição morreu, e o ecrã
// continuou a afirmar fluxo, porque o `EventSource` **estava** mesmo aberto (o console
// mantém-no vivo com comentários de keep-alive).
//
// A rede tem duas camadas, e falham por razões diferentes:
//
//   1. TIPOS — `rotulosDeFluxo` recebe DUAS entradas. Quem quiser derivar um elo do
//      outro tem de apagar um parâmetro da assinatura, e aí este ficheiro deixa de
//      COMPILAR. O alarme é o compilador, não uma asserção.
//   2. LÓGICA — o caso (sseAberto: true, upstream: false) abaixo. Uma implementação
//      que ignorasse o segundo argumento passaria pelos tipos e reprova aqui.
//
// Se algum dia só uma das duas reprovar numa falsificação, a rede está mais fraca do
// que o especificado, e isso é motivo para parar.
// ─────────────────────────────────────────────────────────────────────────────

import { describe, expect, it } from "vitest";
import { marcaDeEstado, rotulosDeFluxo, UPSTREAM_CONFIRMADO } from "./ligacoes";

describe("rotulosDeFluxo", () => {
  it("CASO B: browser ligado e upstream em baixo NAO pode afirmar fluxo confirmado", () => {
    // **O teste discriminante.** É a única combinação em que a fonte errada parece
    // saudável: a ligação do browser está genuinamente aberta e o daemon está morto.
    // Foi exactamente isto que apareceu no ecrã.
    const r = rotulosDeFluxo(true, false);

    expect(r.upstream.texto).not.toBe(UPSTREAM_CONFIRMADO);
    // E não basta o texto diferir: não pode conter a palavra que afirma o elo vivo.
    expect(r.upstream.texto).not.toContain("Live");
    // O browser continua a dizer a verdade sobre a SUA camada — não se apaga um facto
    // real para tapar o outro.
    expect(r.browser.texto).toContain("Open");
  });

  it("as duas camadas sao INDEPENDENTES nas quatro combinacoes", () => {
    // Se qualquer rótulo derivasse do outro elo, uma destas quatro linhas cairia.
    const casos = [
      { sse: true, up: true, browser: "Open", upstream: "Live" },
      { sse: true, up: false, browser: "Open", upstream: "Down" },
      { sse: false, up: true, browser: "Closed", upstream: "Live" },
      { sse: false, up: false, browser: "Closed", upstream: "Down" },
    ] as const;

    for (const c of casos) {
      const r = rotulosDeFluxo(c.sse, c.up);
      expect(r.browser.texto).toBe(`● ${c.browser}`);
      expect(r.upstream.texto).toBe(`● ${c.upstream}`);
    }
  });

  it("CASO E: o browser cair NAO derruba o upstream", () => {
    // O `EventSource` de um separador morre; o supervisor do console não sabe disso e
    // continua subscrito. Colapsar isto em `upstream: false` seria inventar uma queda.
    const r = rotulosDeFluxo(false, true);
    expect(r.upstream.texto).toBe(UPSTREAM_CONFIRMADO);
  });

  it("null e NOT_MEASURED, e nunca é convertido em false", () => {
    // Antes da primeira resposta não há valor. Se `null` caísse no ramo negativo, o
    // ecrã afirmaria "em baixo" sobre algo que ainda não foi perguntado — que é a
    // mesma mentira, ao contrário.
    const nada = rotulosDeFluxo(null, null);
    expect(nada.browser.texto).toBe("○ …");
    expect(nada.upstream.texto).toBe("○ …");

    // E independentemente: um `null` não contamina o outro elo.
    const meio = rotulosDeFluxo(null, true);
    expect(meio.browser.texto).toBe("○ …");
    expect(meio.upstream.texto).toBe(UPSTREAM_CONFIRMADO);
  });

  it("cada rotulo NOMEIA a sua camada, e as duas sao distintas", () => {
    // Sem o nome, dois indicadores lado a lado voltam a ser lidos como uma cadeia
    // contínua — que é como o defeito passou despercebido.
    const r = rotulosDeFluxo(true, true);
    expect(r.browser.camada).not.toBe(r.upstream.camada);
    expect(r.browser.camada.length).toBeGreaterThan(0);
    expect(r.upstream.camada.length).toBeGreaterThan(0);
  });
});

describe("marcaDeEstado", () => {
  it("distingue nao-medido de medido pelo simbolo", () => {
    // `○` e `●` não são decoração: são a diferença entre "não sei" e "sei".
    expect(marcaDeEstado(null, "Sim", "Nao")).toBe("○ …");
    expect(marcaDeEstado(true, "Sim", "Nao")).toBe("● Sim");
    expect(marcaDeEstado(false, "Sim", "Nao")).toBe("● Nao");
  });
});
