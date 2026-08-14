// ─────────────────────────────────────────────────────────────────────────────
// A propriedade que o ADR-0028 D8 exige: **assumir integridade custa dois gestos.**
//
// O daemon NUNCA verifica integridade — `pixel_hash` exige o show inteiro em RAM e hash
// em fluxo não existe (GS2). Quem a afirma é o operador, e `Integrity` é um `enum` e não
// um `bool` precisamente para que "assumido" e "verificado" não fiquem indistinguíveis.
//
// A superfície tem de preservar isso. Um gesto acidental não se distingue de uma decisão.
// ─────────────────────────────────────────────────────────────────────────────

import { describe, expect, it } from "vitest";
import { caminhoMudou, caminhoUtilizavel, cliqueArmar, LIMPA } from "./confirmacao";

describe("cliqueArmar", () => {
  it("UM clique NAO envia — pede confirmacao", () => {
    const r = cliqueArmar(LIMPA);
    expect(r.envia).toBe(false);
    expect(r.proximo.aConfirmar).toBe(true);
  });

  it("DOIS cliques enviam, e limpam a confirmacao", () => {
    const primeiro = cliqueArmar(LIMPA);
    const segundo = cliqueArmar(primeiro.proximo);
    expect(segundo.envia).toBe(true);
    // Limpar importa: sem isso, o clique SEGUINTE enviaria de imediato, e a terceira
    // afirmação de integridade sairia sem confirmação nenhuma.
    expect(segundo.proximo.aConfirmar).toBe(false);
  });

  it("nunca envia com um numero IMPAR de cliques a partir do limpo", () => {
    // A propriedade, em vez de dois exemplos: percorre uma sequência e afirma que cada
    // envio acontece exactamente no 2.º, 4.º, 6.º clique — nunca antes.
    let estado = LIMPA;
    const envios: number[] = [];
    for (let i = 1; i <= 8; i += 1) {
      const r = cliqueArmar(estado);
      estado = r.proximo;
      if (r.envia) envios.push(i);
    }
    expect(envios).toEqual([2, 4, 6, 8]);
  });
});

describe("caminhoMudou", () => {
  it("mudar o caminho DERRUBA uma confirmacao pendente", () => {
    // Sem isto: o operador confirma um ficheiro, muda o caminho, e o clique seguinte
    // carrega OUTRO ficheiro com a integridade afirmada para o primeiro. A confirmação é
    // sobre um artefacto concreto, não sobre a intenção genérica de armar.
    const pendente = cliqueArmar(LIMPA).proximo;
    expect(pendente.aConfirmar).toBe(true);
    expect(caminhoMudou().aConfirmar).toBe(false);

    // E o efeito real: depois de mudar, voltam a ser precisos DOIS cliques.
    const depois = cliqueArmar(caminhoMudou());
    expect(depois.envia).toBe(false);
  });
});

describe("caminhoUtilizavel", () => {
  it("espaco em branco NAO e caminho", () => {
    for (const vazio of ["", " ", "\t", "   \n  "]) {
      expect(caminhoUtilizavel(vazio)).toBe(false);
    }
  });

  it("um caminho real e utilizavel", () => {
    expect(caminhoUtilizavel("/shows/robot.lumyx")).toBe(true);
    // Sem validação de extensão nem de existência: quem decide é o daemon, com
    // `load_failed` e o erro real do loader no `detail`. Adivinhar aqui seria domínio.
    expect(caminhoUtilizavel("qualquer-coisa")).toBe(true);
  });
});
