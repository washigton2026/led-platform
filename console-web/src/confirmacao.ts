// ─────────────────────────────────────────────────────────────────────────────
// A confirmação de "assumir integridade", como lógica **pura**.
//
// Vive fora do React pela mesma razão que o `eventos.ts` e o `ligacoes.ts`: para poder
// ser exercitada sem montar nada. A primeira versão desta fatia tentou testar a
// confirmação renderizando marcação e afirmando que uma espia não fora chamada — mas
// `renderToStaticMarkup` não clica, portanto a espia nunca poderia ter sido chamada e o
// teste passava **sem exercitar nada**. Teatro, e do pior tipo: verde por não olhar.
//
// Com a decisão aqui, "são precisos dois cliques" deixa de ser uma afirmação sobre
// pixels e passa a ser uma propriedade verificável.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * O estado da confirmação pendente.
 *
 * Deliberadamente um tipo e não um `boolean` solto: `true` sozinho não diz *confirmar o
 * quê*, e o dia em que houver uma segunda acção confirmável os dois booleanos seriam
 * indistinguíveis no sítio da chamada.
 */
export interface Confirmacao {
  readonly aConfirmar: boolean;
}

/** Nada pendente. O estado inicial, e para onde se volta depois de enviar. */
export const LIMPA: Confirmacao = { aConfirmar: false };

/**
 * Um clique na acção que **assume integridade e arma**.
 *
 * O primeiro clique **não envia** — pede confirmação. O segundo envia e limpa.
 *
 * É esta a regra da decisão D8 do ADR-0028: o operador tem de afirmar duas vezes algo que
 * o daemon **não verifica**. Uma caixa de verificação daria o mesmo efeito com um só
 * gesto, e um gesto acidental não se distingue de uma decisão.
 */
export function cliqueArmar(c: Confirmacao): { readonly proximo: Confirmacao; readonly envia: boolean } {
  return c.aConfirmar
    ? { proximo: LIMPA, envia: true }
    : { proximo: { aConfirmar: true }, envia: false };
}

/**
 * O caminho mudou — qualquer confirmação pendente **cai**.
 *
 * Sem isto, o operador confirmaria um ficheiro, mudaria o caminho, e o clique seguinte
 * carregaria **outro** ficheiro com a integridade afirmada para o primeiro. A confirmação
 * é sobre um artefacto concreto, não sobre a intenção genérica de armar.
 */
export function caminhoMudou(): Confirmacao {
  return LIMPA;
}

/**
 * Um caminho utilizável? Espaço em branco não é caminho.
 *
 * Isto **não** é antecipar a matriz de estados do daemon (o ADR-0028 D9 proíbe-o): é a
 * ausência do único argumento obrigatório de `ArgsLoad`. Enviar `path: ""` gastaria uma
 * ida ao daemon para receber um erro que o browser já tem em mãos.
 */
export function caminhoUtilizavel(caminho: string): boolean {
  return caminho.trim() !== "";
}
