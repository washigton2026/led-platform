// ─────────────────────────────────────────────────────────────────────────────
// Application Shell — Phase 1.
//
// Mostra APENAS o que `/api/state` prova. Não há saúde de hardware, de controlador
// nem de rede; não há certificação, evidência física nem frescura. Nenhum desses
// tem produtor (ADR-0028 D3), e inventá-los seria a mentira que o ADR-0026 §9 e o
// Operational Truth Boundary existem para impedir.
//
// Nada de design system aqui. Os estilos são o mínimo para a informação ser
// legível; extrair componentes vem depois, quando a repetição existir de facto.
// ─────────────────────────────────────────────────────────────────────────────

import { useEffect, useState } from "react";
import { descreveEvento, ehProgresso } from "./eventos";
import {
  comandar,
  lerEstado,
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
  const [resultado, setResultado] = useState<Resultado | null>(null);
  const [seekMs, setSeekMs] = useState("0");

  useEffect(() => {
    let vivo = true;
    const perguntar = async () => {
      const r = await lerEstado();
      if (vivo) setLigacao(r);
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

      <section aria-labelledby="h-console">
        <h2 id="h-console" style={estilos.seccao}>
          CONSOLE
        </h2>
        <p style={estilos.linha} role="status" aria-live="polite">
          {ligacao === null ? "○ …" : ligacao.tipo === "dados" ? "● Connected" : "● Offline"}
        </p>
      </section>

      <hr style={estilos.regua} />

      {ligacao?.tipo === "dados" ? (
        <Daemon estado={ligacao.estado} />
      ) : ligacao?.tipo === "offline" ? (
        <Indisponivel code={ligacao.code} detail={ligacao.detail} />
      ) : null}

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
      <Eventos eventos={eventos} fluxo={fluxo} progresso={progresso} />
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
 * `load` e `unload` não estão aqui: mudam o show carregado, não a posição no tempo, e
 * pertencem à gestão de shows.
 */
function Transporte({
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
    <section aria-labelledby="h-transporte">
      <h2 id="h-transporte" style={estilos.seccao}>
        TRANSPORT
      </h2>
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
    </section>
  );
}

/**
 * O registo de eventos.
 *
 * O payload é agora **tipado pelo contrato gerado**, e por isso pode ser lido em vez de
 * despejado. Um evento que não analise mostra a linha crua — não se deita fora.
 */
function Eventos({
  eventos,
  fluxo,
  progresso,
}: {
  eventos: readonly EventoCru[];
  fluxo: boolean | null;
  progresso: { ultima: EventoCru | null; total: number };
}) {
  return (
    <section aria-labelledby="h-eventos">
      <h2 id="h-eventos" style={estilos.seccao}>
        EVENTS
      </h2>
      <p style={estilos.linha} role="status" aria-live="polite">
        {fluxo === null ? "○ …" : fluxo ? "● Streaming" : "● Stream down"}
      </p>
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
    </section>
  );
}

function Daemon({ estado }: { estado: EstadoDoDaemon }) {
  return (
    <section aria-labelledby="h-daemon">
      <h2 id="h-daemon" style={estilos.seccao}>
        DAEMON
      </h2>
      <dl style={estilos.lista}>
        <Campo rotulo="State" valor={estado.state.toUpperCase()} />
        <Campo rotulo="Position" valor={`${estado.position_ms} ms`} />
        <Campo rotulo="Duration" valor={`${estado.duration_ms} ms`} />
        <Campo rotulo="Ticks" valor={String(estado.ticks)} />
        {/* `null` significa SEM SHOW — e é isso que se escreve, não `0`. */}
        <Campo rotulo="Show" valor={estado.show_id === null ? "none" : String(estado.show_id)} />
      </dl>
    </section>
  );
}

function Indisponivel({ code, detail }: { code: string; detail: string }) {
  return (
    <section aria-labelledby="h-erro">
      <h2 id="h-erro" style={estilos.seccao}>
        DAEMON
      </h2>
      <p style={estilos.linha}>Daemon unavailable</p>
      {/* O código do backend, verbatim. É o que diz PORQUÊ. */}
      <p style={estilos.codigo}>{code}</p>
      <p style={estilos.detalhe}>{detail}</p>
    </section>
  );
}

function Campo({ rotulo, valor }: { rotulo: string; valor: string }) {
  return (
    <div style={estilos.par}>
      <dt style={estilos.rotulo}>{rotulo}</dt>
      <dd style={estilos.valor}>{valor}</dd>
    </div>
  );
}

const mono = "ui-monospace, SFMono-Regular, Menlo, monospace";

const estilos = {
  pagina: { fontFamily: mono, maxWidth: "34rem", margin: "3rem auto", padding: "0 1rem" },
  marca: { fontSize: "1rem", letterSpacing: "0.3em", margin: 0 },
  regua: { border: 0, borderTop: "1px solid currentColor", opacity: 0.25, margin: "1rem 0" },
  seccao: { fontSize: "0.7rem", letterSpacing: "0.2em", opacity: 0.6, margin: "0 0 0.5rem" },
  linha: { margin: 0 },
  lista: { margin: 0 },
  par: { display: "flex", justifyContent: "space-between", padding: "0.15rem 0" },
  rotulo: { opacity: 0.6 },
  valor: { margin: 0, fontVariantNumeric: "tabular-nums" as const },
  codigo: { margin: "0.25rem 0 0", fontWeight: 600 },
  detalhe: { margin: "0.25rem 0 0", opacity: 0.6, fontSize: "0.85rem" },
  eventos: { margin: "0.5rem 0 0", padding: 0, listStyle: "none" as const },
  botoes: { display: "flex", gap: "0.4rem", alignItems: "center", flexWrap: "wrap" as const, marginBottom: "0.6rem" },
  botao: {
    fontFamily: mono,
    fontSize: "0.8rem",
    padding: "0.25rem 0.7rem",
    border: "1px solid currentColor",
    background: "transparent",
    cursor: "pointer",
    borderRadius: 2,
  },
  rotuloSeek: { display: "flex", gap: "0.35rem", alignItems: "center", fontSize: "0.75rem", opacity: 0.7 },
  entrada: { fontFamily: mono, fontSize: "0.8rem", width: "6rem", padding: "0.2rem 0.3rem" },
  instante: { opacity: 0.45, marginRight: "0.6rem", fontVariantNumeric: "tabular-nums" as const },
  evento: {
    fontSize: "0.75rem",
    padding: "0.1rem 0",
    whiteSpace: "pre-wrap" as const,
    wordBreak: "break-all" as const,
    opacity: 0.85,
  },
} as const;
