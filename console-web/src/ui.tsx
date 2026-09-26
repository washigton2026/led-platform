// ─────────────────────────────────────────────────────────────────────────────
// O sistema de design — o que EXISTE, não o que se possa vir a querer.
//
// Não há paleta. Esta interface não tem uma única cor escrita: tudo é
// `currentColor` com opacidade, o que a faz herdar o tema do browser sem uma linha
// de código para isso. Inventar cores agora seria escolher um tema que ninguém
// pediu e perder essa propriedade.
//
// Os componentes aqui saíram de repetição **medida** nas três superfícies reais
// (`Daemon`, `Transporte`, `Eventos`), não de um catálogo. O `Botao` ficou de fora
// de propósito: tem UM sítio de uso, e um componente com um só chamador é uma
// indirecção que não paga a viagem.
// ─────────────────────────────────────────────────────────────────────────────

import type { ReactNode } from "react";

const MONO = "ui-monospace, SFMono-Regular, Menlo, monospace";

/**
 * A escala de ênfase. **Cinco degraus, e todos já estavam no ecrã** — o que isto faz
 * é dar-lhes nome e um sítio.
 *
 * KNOWN GAP: `secundario` (0.6) e `rotuloDeCampo` (0.7) são ambos rótulos e deviam
 * ser o mesmo degrau. Uni-los muda pixels, e este ficheiro nasceu de um refactor
 * que tinha de não mudar nenhum. Fica nomeado em vez de alisado em silêncio.
 */
const ENFASE = {
  regua: 0.25,
  discreto: 0.45,
  secundario: 0.6,
  rotuloDeCampo: 0.7,
  corpoDeEvento: 0.85,
} as const;

/** Números que se leem em coluna alinham; sem isto, a posição a correr dança. */
const NUMERICO = { fontVariantNumeric: "tabular-nums" } as const;

export const estilos = {
  pagina: { fontFamily: MONO, maxWidth: "34rem", margin: "3rem auto", padding: "0 1rem" },
  marca: { fontSize: "1rem", letterSpacing: "0.3em", margin: 0 },
  regua: {
    border: 0,
    borderTop: "1px solid currentColor",
    opacity: ENFASE.regua,
    margin: "1rem 0",
  },
  seccao: {
    fontSize: "0.7rem",
    letterSpacing: "0.2em",
    opacity: ENFASE.secundario,
    margin: "0 0 0.5rem",
  },
  linha: { margin: 0 },
  lista: { margin: 0 },
  par: { display: "flex", justifyContent: "space-between", padding: "0.15rem 0" },
  rotulo: { opacity: ENFASE.secundario },
  valor: { margin: 0, ...NUMERICO },
  codigo: { margin: "0.25rem 0 0", fontWeight: 600 },
  detalhe: { margin: "0.25rem 0 0", opacity: ENFASE.secundario, fontSize: "0.85rem" },
  eventos: { margin: "0.5rem 0 0", padding: 0, listStyle: "none" },
  botoes: {
    display: "flex",
    gap: "0.4rem",
    alignItems: "center",
    flexWrap: "wrap",
    marginBottom: "0.6rem",
  },
  botao: {
    fontFamily: MONO,
    fontSize: "0.8rem",
    padding: "0.25rem 0.7rem",
    border: "1px solid currentColor",
    background: "transparent",
    cursor: "pointer",
    borderRadius: 2,
  },
  rotuloSeek: {
    display: "flex",
    gap: "0.35rem",
    alignItems: "center",
    fontSize: "0.75rem",
    opacity: ENFASE.rotuloDeCampo,
  },
  entrada: { fontFamily: MONO, fontSize: "0.8rem", width: "6rem", padding: "0.2rem 0.3rem" },
  // Um caminho de ficheiro nao cabe em 6rem, e um campo que corta o que o operador
  // escreveu esconde precisamente a parte que ele precisa de conferir antes de carregar.
  entradaCaminho: {
    fontFamily: MONO,
    fontSize: "0.8rem",
    flex: 1,
    minWidth: "18rem",
    padding: "0.2rem 0.3rem",
  },
  instante: { opacity: ENFASE.discreto, marginRight: "0.6rem", ...NUMERICO },
  evento: {
    fontSize: "0.75rem",
    padding: "0.1rem 0",
    whiteSpace: "pre-wrap",
    wordBreak: "break-all",
    opacity: ENFASE.corpoDeEvento,
  },
} as const;

/**
 * Uma secção com título.
 *
 * A razão de existir não é a poupança de linhas: é o par `aria-labelledby`/`id`. Ele
 * estava escrito à mão em **cinco** sítios, e uma letra trocada num deles rompe o nome
 * acessível da secção **sem** partir nada visível — não há teste que apanhe uma string
 * que deixou de casar com outra string. Aqui o par vem do mesmo argumento e não pode
 * divergir.
 */
export function Seccao({
  id,
  titulo,
  children,
}: {
  id: string;
  titulo: string;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id}>
      <h2 id={id} style={estilos.seccao}>
        {titulo}
      </h2>
      {children}
    </section>
  );
}

/**
 * O estado de **uma** ligação — e o nome da camada que ele descreve.
 *
 * A `camada` não é decoração: é o que impede este ecrã de repetir o defeito que o
 * ADR-0026 §9-quinquies corrigiu. Havia dois indicadores lado a lado a medir elos
 * diferentes — *browser→console* e *console→daemon* — e nenhum dizia qual; o operador
 * lia-os como uma cadeia contínua, e um deles estava a afirmar fluxo sobre silêncio.
 * Com a camada escrita, nenhum indicador pode ser confundido com o vizinho.
 *
 * Este componente **não decide** o texto: recebe-o já feito de `ligacoes.ts`. Calcular
 * aqui daria um segundo sítio a saber traduzir estado em rótulo, e o dia em que os dois
 * divergissem seria invisível.
 *
 * `aria-live="polite"` porque isto muda sozinho: um leitor de ecrã tem de saber que a
 * ligação caiu sem o operador ter feito nada.
 */
export function Indicador({ camada, texto }: { camada: string; texto: string }) {
  return (
    <p style={estilos.linha} role="status" aria-live="polite">
      <span style={estilos.rotulo}>{camada}</span> {texto}
    </p>
  );
}

/** Um par rótulo/valor dentro de uma `<dl>`. O `<dt>`/`<dd>` é o que o torna legível fora do ecrã. */
export function Campo({ rotulo, valor }: { rotulo: string; valor: string }) {
  return (
    <div style={estilos.par}>
      <dt style={estilos.rotulo}>{rotulo}</dt>
      <dd style={estilos.valor}>{valor}</dd>
    </div>
  );
}
