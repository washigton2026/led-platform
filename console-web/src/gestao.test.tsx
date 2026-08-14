// ─────────────────────────────────────────────────────────────────────────────
// A superfície de gestão do show — os factos ESTRUTURAIS.
//
// A regra dos **dois gestos** é exercitada em `confirmacao.test.ts`, sobre lógica pura.
// Aqui prova-se o que só a marcação pode provar: que não existe caixa de verificação,
// que as duas acções têm nomes distintos, e que o `unload` nunca é desactivado.
//
// A separação não é arrumação: `renderToStaticMarkup` **não clica**. Um teste que
// afirmasse aqui "o primeiro clique não envia" passaria sem exercitar nada — verde por
// não olhar, que é o modo de falha destes testes.
// ─────────────────────────────────────────────────────────────────────────────

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Gestao } from "./App";

const NADA = () => {};
const base = {
  caminho: "/shows/robot.lumyx",
  aoMudarCaminho: NADA,
  aoCarregar: NADA,
  aoDescarregar: NADA,
};

const render = (p: Partial<Parameters<typeof Gestao>[0]> = {}) =>
  renderToStaticMarkup(<Gestao {...base} {...p} />);

describe("Gestao", () => {
  it("NAO ha caixa de verificacao — a decisao D8 e estrutural, nao estilistica", () => {
    // Uma caixa pré-marcada faria o operador afirmar integridade sem saber que afirmou;
    // uma desmarcada daria um `play` com `not_armed` sem explicação no ecrã. As duas
    // falham, ao contrário — e por isso este tipo de entrada está proibido nesta
    // superfície, não desencorajado.
    expect(render()).not.toContain('type="checkbox"');
  });

  it("as duas accoes tem nomes distintos, e a que arma NOMEIA a consequencia", () => {
    const html = render();
    expect(html).toContain("carregar sem armar");
    expect(html).toContain("assumir integridade e armar");
  });

  it("antes de confirmar, o ecra NAO promete que ja assumiu", () => {
    const html = render();
    expect(html).not.toContain("confirmar: assumo a integridade");
    // E a consequência — "o daemon não verifica" — só aparece quando há o que confirmar.
    expect(html).not.toContain("não verifica");
  });

  it("sem caminho, as accoes de CARREGAR ficam indisponiveis", () => {
    // Isto NÃO é antecipar a matriz de estados do daemon (o ADR-0028 D9 proíbe-o): é a
    // ausência do único argumento obrigatório de `ArgsLoad`. Enviar `path: ""` gastaria
    // uma ida ao daemon para receber um erro que o browser já tem em mãos.
    const html = render({ caminho: "   " });
    expect(html.split("<button").length - 1).toBe(3); // carregar · assumir · unload
    expect(html.split("disabled=").length - 1).toBe(2); // só os dois de carregar
  });

  it("o UNLOAD nunca e desactivado — a recusa vem do daemon (D9)", () => {
    // `unload` é recusado em `idle` e `playing`, mas quem decide é a matriz do ADR-0023.
    // Antecipá-la aqui seria reimplementar 80 pares no browser — a segunda fonte de
    // verdade do ADR-0026 §15, que divergiria no dia em que a matriz mudasse.
    for (const caminho of ["", "/shows/robot.lumyx"]) {
      const html = render({ caminho });
      const antesDoUnload = html.slice(0, html.indexOf(">unload<"));
      const ultimoBotao = antesDoUnload.slice(antesDoUnload.lastIndexOf("<button"));
      expect(ultimoBotao).not.toContain("disabled");
    }
  });

  it("o caminho e ESCRITO, nao escolhido de uma lista", () => {
    // Nenhuma rota lista shows. Uma lista fabricada no console seria o ADR-0028 D3 outra
    // vez, noutro campo — e o operador leria "estes são os shows que existem".
    const html = render();
    expect(html).toContain('type="text"');
    expect(html).not.toContain("<select");
    expect(html).not.toContain("<option");
  });
});
