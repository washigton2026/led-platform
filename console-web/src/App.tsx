// ─────────────────────────────────────────────────────────────────────────────
// Application Shell — Phase 1.
//
// Mostra APENAS o que `/api/state` prova. Não há saúde de hardware, de controlador
// nem de rede; não há certificação, evidência física nem frescura. Nenhum desses
// tem produtor (ADR-0028 D3), e inventá-los seria a mentira que o ADR-0026 §9 e o
// Operational Truth Boundary existem para impedir.
//
// Os componentes partilhados e os tokens vivem em `ui.tsx`. Foram extraídos DEPOIS
// de a repetição existir e ser contada — `Seccao` tinha cinco cópias do par
// `aria-labelledby`/`id`, `Indicador` tinha duas do tri-estado.
// ─────────────────────────────────────────────────────────────────────────────

import { useEffect, useState } from "react";
import { caminhoMudou, caminhoUtilizavel, cliqueArmar, LIMPA } from "./confirmacao";
import { descreveEvento, ehProgresso } from "./eventos";
import { marcaDeEstado, rotulosDeFluxo } from "./ligacoes";
import { Campo, estilos, Indicador, Seccao } from "./ui";
import {
  carregar,
  comandar,
  lerEstado,
  lerUpstream,
  subscreverEventos,
  TRANSPORTE,
  type ComandoTransporte,
  type EstadoDoDaemon,
  type EventoCru,
  type Ligacao,
  type Resultado,
} from "./transport/api";

/**
 * Cadência do polling do **estado**.
 *
 * O SSE traz os eventos, mas **não** traz o estado: o `status` é um snapshot que se
 * consulta, e os eventos são transições que se recebem. São coisas diferentes, e derivar
 * o estado a partir dos eventos seria reconstruir a máquina do ADR-0023 no browser —
 * exactamente a segunda fonte de verdade que o ADR-0026 §15 proíbe.
 */
const INTERVALO_MS = 1000;

/** Quantas TRANSIÇÕES manter à vista. O fluxo é infinito; a lista não pode ser. */
const EVENTOS_VISIVEIS = 12;

export function App() {
  // `null` = ainda não perguntámos. **Não é** offline, e não é ok: é ausência de
  // resposta, e o ecrã di-lo em vez de escolher um dos dois.
  const [ligacao, setLigacao] = useState<Ligacao | null>(null);
  const [eventos, setEventos] = useState<readonly EventoCru[]>([]);
  const [fluxo, setFluxo] = useState<boolean | null>(null);
  // O elo a MONTANTE, medido por `/api/upstream`. `null` = ainda não perguntámos.
  // Vive separado do `fluxo` de propósito: são camadas diferentes e caem em momentos
  // diferentes (ADR-0026 §9-quinquies).
  const [upstream, setUpstream] = useState<boolean | null>(null);
  const [resultado, setResultado] = useState<Resultado | null>(null);
  const [seekMs, setSeekMs] = useState("0");
  const [caminho, setCaminho] = useState("");

  useEffect(() => {
    let vivo = true;
    const perguntar = async () => {
      // Os dois na mesma cadência: são leituras baratas e correlacioná-las no tempo é
      // o que torna a divergência entre elas legível para quem olha.
      const [r, u] = await Promise.all([lerEstado(), lerUpstream()]);
      if (!vivo) return;
      setLigacao(r);
      setUpstream(u);
    };
    void perguntar();
    const t = setInterval(() => void perguntar(), INTERVALO_MS);
    return () => {
      vivo = false;
      clearInterval(t);
    };
  }, []);

  const [progresso, setProgresso] = useState<{
    ultima: EventoCru | null;
    total: number;
  }>({ ultima: null, total: 0 });

  useEffect(
    () =>
      subscreverEventos((e) => {
        if (ehProgresso(e.payload)) {
          setProgresso((a) => ({ ultima: e, total: a.total + 1 }));
        } else {
          setEventos((anteriores) => [e, ...anteriores].slice(0, EVENTOS_VISIVEIS));
        }
      }, setFluxo),
    [],
  );

  return (
    <main style={estilos.pagina}>
      <h1 style={estilos.marca}>LUMYX</h1>
      <hr style={estilos.regua} />

      <Seccao id="h-console" titulo="CONSOLE">
        <Indicador
          camada="Console API"
          texto={marcaDeEstado(
            ligacao === null ? null : ligacao.tipo === "dados",
            "Connected",
            "Offline",
          )}
        />
      </Seccao>

      <hr style={estilos.regua} />

      {ligacao?.tipo === "dados" ? (
        <Daemon estado={ligacao.estado} />
      ) : ligacao?.tipo === "offline" ? (
        <Indisponivel code={ligacao.code} detail={ligacao.detail} />
      ) : null}

      <hr style={estilos.regua} />
      <Gestao
        caminho={caminho}
        aoMudarCaminho={setCaminho}
        aoCarregar={(assumir) => void carregar(caminho.trim(), assumir).then(setResultado)}
        aoDescarregar={() => void comandar("unload").then(setResultado)}
      />

      <hr style={estilos.regua} />
      <Transporte
        seekMs={seekMs}
        aoMudarSeek={setSeekMs}
        aoComandar={(cmd) => {
          const args = cmd === "seek" ? { to_ms: Number(seekMs) || 0 } : undefined;
          void comandar(cmd, args).then(setResultado);
        }}
        resultado={resultado}
      />

      <hr style={estilos.regua} />
      <Eventos eventos={eventos} fluxo={fluxo} upstream={upstream} progresso={progresso} />
    </main>
  );
}

/**
 * A superfície de comando — **transporte apenas**.
 *
 * Os botões estão SEMPRE activos. Desactivá-los consoante o estado seria reimplementar a
 * matriz de 80 pares do ADR-0023 no browser, e ela divergiria no dia em que a matriz
 * mudasse. Quem decide se um comando se aplica é o daemon; o que a UI faz é **mostrar a
 * resposta dele** — incluindo a recusa, com o código verbatim.
 *
 * `load` e `unload` não estão aqui: mudam **o que está carregado**, não a posição no tempo.
 * Vivem em `Gestao`, e a separação é a mesma que o ADR-0023 faz.
 */
export function Transporte({
  seekMs,
  aoMudarSeek,
  aoComandar,
  resultado,
}: {
  seekMs: string;
  aoMudarSeek: (v: string) => void;
  aoComandar: (cmd: ComandoTransporte) => void;
  resultado: Resultado | null;
}) {
  return (
    <Seccao id="h-transporte" titulo="TRANSPORT">
      <div style={estilos.botoes}>
        {TRANSPORTE.map((cmd) => (
          <button key={cmd} type="button" style={estilos.botao} onClick={() => aoComandar(cmd)}>
            {cmd}
          </button>
        ))}
        <label style={estilos.rotuloSeek}>
          to_ms
          <input
            type="number"
            min={0}
            value={seekMs}
            onChange={(e) => aoMudarSeek(e.target.value)}
            style={estilos.entrada}
          />
        </label>
      </div>
      {resultado === null ? null : (
        <p style={estilos.linha} role="status" aria-live="polite">
          {resultado.tipo === "aceite" ? (
            <>
              <span style={estilos.rotulo}>{resultado.cmd}</span> aceite
            </>
          ) : (
            <>
              <span style={estilos.rotulo}>{resultado.cmd}</span> recusado —{" "}
              {/* O código do daemon, verbatim: é ele que diz PORQUÊ. */}
              <span style={estilos.codigo}>{resultado.code}</span>
            </>
          )}
        </p>
      )}
      {resultado?.tipo === "recusado" ? (
        <p style={estilos.detalhe}>{resultado.detail}</p>
      ) : null}
    </Seccao>
  );
}


/**
 * A gestão do show — **o que está carregado**, não a posição no tempo.
 *
 * # Duas acções, e nunca uma caixa (ADR-0028 D8)
 *
 * `assume_integrity` faz duas coisas: afirma a integridade e **arma** o show. Uma caixa
 * pré-marcada faria o operador afirmar sem saber que afirmou — o colapso que o `enum`
 * `Integrity` existe para impedir. Uma desmarcada dá um `load` que parece funcionar e um
 * `play` que recusa com `not_armed` sem explicação no ecrã. As duas falham, ao contrário.
 *
 * Por isso são **duas acções com nome próprio**, e a que afirma integridade **nomeia a
 * consequência** e exige confirmação: o operador tem de dizer duas vezes que assume algo
 * que o daemon não verifica.
 *
 * # A matriz de estados NÃO é replicada aqui (ADR-0028 D9)
 *
 * `load` só é aceite em `idle`; `unload` em tudo menos `idle` e `playing`. Os botões ficam
 * activos, e o que se mostra é a **recusa real** — `show_already_loaded`, `not_applicable`.
 */
export function Gestao({
  caminho,
  aoMudarCaminho,
  aoCarregar,
  aoDescarregar,
}: {
  caminho: string;
  aoMudarCaminho: (v: string) => void;
  aoCarregar: (assumirIntegridade: boolean) => void;
  aoDescarregar: () => void;
}) {
  // A decisao vive em `confirmacao.ts` — puro, testado sem montar nada. Aqui so o estado.
  const [confirmacao, setConfirmacao] = useState(LIMPA);
  const utilizavel = caminhoUtilizavel(caminho);

  return (
    <Seccao id="h-gestao" titulo="SHOW">
      {/* Caminho escrito, nao escolhido de uma lista: nenhuma rota lista shows, e
          inventar uma seria o ADR-0028 D3. O daemon recusa o que nao existir. */}
      <label style={estilos.rotuloSeek}>
        path
        <input
          type="text"
          value={caminho}
          placeholder="/caminho/para/show.lumyx"
          onChange={(e) => {
            aoMudarCaminho(e.target.value);
            // Mudar o caminho invalida uma confirmacao pendente: senao o operador
            // confirmaria um ficheiro e carregaria outro.
            setConfirmacao(caminhoMudou());
          }}
          style={estilos.entradaCaminho}
        />
      </label>

      <div style={estilos.botoes}>
        <button
          type="button"
          style={estilos.botao}
          disabled={!utilizavel}
          onClick={() => aoCarregar(false)}
        >
          carregar sem armar
        </button>

        <button
          type="button"
          style={estilos.botao}
          disabled={!utilizavel}
          onClick={() => {
            const r = cliqueArmar(confirmacao);
            setConfirmacao(r.proximo);
            if (r.envia) aoCarregar(true);
          }}
        >
          {confirmacao.aConfirmar ? "confirmar: assumo a integridade" : "assumir integridade e armar"}
        </button>

        <button type="button" style={estilos.botao} onClick={aoDescarregar}>
          unload
        </button>
      </div>

      {confirmacao.aConfirmar ? (
        // A consequencia, escrita. O daemon NAO verifica integridade — `pixel_hash` exige
        // o show inteiro em RAM (GS2) — portanto quem a afirma e o operador, e tem de o
        // ler antes de o fazer.
        <p style={estilos.detalhe}>
          O daemon <strong>não verifica</strong> a integridade deste ficheiro. Confirmar
          significa que <strong>o operador a afirma</strong>, e o show fica armado.
        </p>
      ) : null}
    </Seccao>
  );
}

/**
 * O registo de eventos.
 *
 * O payload é agora **tipado pelo contrato gerado**, e por isso pode ser lido em vez de
 * despejado. Um evento que não analise mostra a linha crua — não se deita fora.
 */
export function Eventos({
  eventos,
  fluxo,
  upstream,
  progresso,
}: {
  eventos: readonly EventoCru[];
  fluxo: boolean | null;
  upstream: boolean | null;
  progresso: { ultima: EventoCru | null; total: number };
}) {
  // As DUAS entradas atravessam a mesma funcao. Derivar uma da outra ficaria visivel
  // num so sitio, e o teste (sseAberto: true, upstream: false) reprova se acontecer.
  const rotulos = rotulosDeFluxo(fluxo, upstream);
  return (
    <Seccao id="h-eventos" titulo="EVENTS">
      {/* Browser -> console. Fica aberta com o daemon morto: o console mantem-na viva
          com comentarios de keep-alive. Por si so, NAO prova que chegam eventos. */}
      <Indicador camada={rotulos.browser.camada} texto={rotulos.browser.texto} />
      {/* Console -> daemon. E este o elo que diz se ha fluxo a montante de verdade. */}
      <Indicador camada={rotulos.upstream.camada} texto={rotulos.upstream.texto} />
      {/* O progresso, colapsado. A contagem existe para nada parecer escondido. */}
      {progresso.ultima === null ? null : (
        <p style={estilos.detalhe}>
          <span style={estilos.instante}>{progresso.ultima.payload?.t_ms}</span>
          {progresso.ultima.payload !== null ? descreveEvento(progresso.ultima.payload) : ""} ·{" "}
          {progresso.total} evento{progresso.total === 1 ? "" : "s"} de posição
        </p>
      )}
      {eventos.length === 0 ? (
        // Silêncio é silêncio. Um daemon parado não emite transições, e dizer isso é mais
        // honesto do que uma lista vazia sem explicação.
        <p style={estilos.detalhe}>sem transições desde que esta ligação abriu</p>
      ) : (
        <ol style={estilos.eventos}>
          {eventos.map((e) => (
            <li key={e.seq} style={estilos.evento}>
              {e.payload === null ? (
                // Ilegível: mostra-se o que veio, sem fingir que se entendeu.
                <span style={estilos.detalhe}>{e.linha}</span>
              ) : (
                <>
                  <span style={estilos.instante}>{e.payload.t_ms}</span>
                  <span>{descreveEvento(e.payload)}</span>
                </>
              )}
            </li>
          ))}
        </ol>
      )}
    </Seccao>
  );
}

export function Daemon({ estado }: { estado: EstadoDoDaemon }) {
  return (
    <Seccao id="h-daemon" titulo="DAEMON">
      <dl style={estilos.lista}>
        <Campo rotulo="State" valor={estado.state.toUpperCase()} />
        <Campo rotulo="Position" valor={`${estado.position_ms} ms`} />
        <Campo rotulo="Duration" valor={`${estado.duration_ms} ms`} />
        <Campo rotulo="Ticks" valor={String(estado.ticks)} />
        {/* `null` significa SEM SHOW — e é isso que se escreve, não `0`. */}
        <Campo rotulo="Show" valor={estado.show_id === null ? "none" : String(estado.show_id)} />
      </dl>
    </Seccao>
  );
}

export function Indisponivel({ code, detail }: { code: string; detail: string }) {
  return (
    <Seccao id="h-erro" titulo="DAEMON">
      <p style={estilos.linha}>Daemon unavailable</p>
      {/* O código do backend, verbatim. É o que diz PORQUÊ. */}
      <p style={estilos.codigo}>{code}</p>
      <p style={estilos.detalhe}>{detail}</p>
    </Seccao>
  );
}


