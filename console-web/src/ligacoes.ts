// ─────────────────────────────────────────────────────────────────────────────
// Duas camadas, dois rótulos, uma função — e é a assinatura que segura a regra.
//
// Lógica **pura**: sem React, sem rede, sem estado. Vive separada para poder ser
// testada sem montar nada, como o `eventos.ts`.
//
// # A regra que este ficheiro existe para impor (ADR-0026 §9-quinquies)
//
// `EventSource.onopen` mede **browser → console**. A subscrição mede
// **console → daemon**. São elos diferentes e divergem: o console mantém o SSE vivo
// com comentários de keep-alive, portanto a ligação do browser fica aberta **com o
// daemon morto**. Foi assim que a interface chegou a mostrar fluxo sobre silêncio.
//
// # Porque uma função com DUAS entradas, e não duas funções
//
// Porque a regressão que interessa impedir é derivar um elo do outro. Com os dois
// factos a entrarem pela mesma porta, escrever `upstream: sseAberto` fica visível num
// só sítio — e quem quiser "limpar" o parâmetro que julga não usado tem de o **apagar
// da assinatura**, o que parte a compilação do teste em vez de a deixar passar.
// ─────────────────────────────────────────────────────────────────────────────

/** Um rótulo pronto a mostrar: a camada que está a ser medida, e o seu estado. */
export interface Rotulo {
  /** Qual elo isto descreve. Aparece no ecrã — a camada nunca fica implícita. */
  readonly camada: string;
  /** O estado, já com o marcador. `○` = não medido; `●` = medido. */
  readonly texto: string;
}

/**
 * O par de rótulos das duas camadas de eventos.
 *
 * `null` em qualquer entrada é **NOT_MEASURED** — ainda não perguntámos. Não é `false`:
 * ausência de resposta e resposta negativa são coisas diferentes, e colapsá-las é a
 * mesma classe de erro que o `stale_ms()` evita ao ser `Option<u64>` em vez de `0`.
 *
 * @param sseAberto  a ligação **browser → console** está aberta? (`EventSource`)
 * @param upstream   existe subscrição **console → daemon**? (`GET /api/upstream`)
 */
export function rotulosDeFluxo(
  sseAberto: boolean | null,
  upstream: boolean | null,
): { readonly browser: Rotulo; readonly upstream: Rotulo } {
  return {
    browser: {
      camada: "Browser stream",
      texto: marca(sseAberto, "Open", "Closed"),
    },
    upstream: {
      camada: "Daemon subscription",
      // Deriva de `upstream`, e **só** de `upstream`. Se algum dia esta linha ler
      // `sseAberto`, o caso (true, false) do teste reprova — é esse o alarme.
      texto: marca(upstream, "Live", "Down"),
    },
  };
}

/**
 * `○` para não medido, `●` para medido. O símbolo distingue **ausência de resposta** de
 * veredito, e é o único sítio onde essa forma é escrita — sem isto, o terceiro ramo
 * teria de ser lembrado em cada chamador, e o dia em que um se esquecesse dele
 * produziria um `?:` de dois ramos com `null` a cair no lado errado.
 */
export function marcaDeEstado(valor: boolean | null, sim: string, nao: string): string {
  if (valor === null) return "○ …";
  return valor ? `● ${sim}` : `● ${nao}`;
}

const marca = marcaDeEstado;

/**
 * O rótulo de fluxo confirmado a montante. **Existe para o teste o poder nomear** sem
 * repetir a string — se ele fosse escrito à mão no teste, mudar o texto aqui deixaria
 * o teste verde a comparar contra algo que já ninguém mostra.
 */
export const UPSTREAM_CONFIRMADO = "● Live";
