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
import { lerEstado, type EstadoDoDaemon, type Ligacao } from "./transport/api";

/** Cadência do polling. SSE (`/api/events`) é Phase 3 — aqui ainda não. */
const INTERVALO_MS = 1000;

export function App() {
  // `null` = ainda não perguntámos. **Não é** offline, e não é ok: é ausência de
  // resposta, e o ecrã di-lo em vez de escolher um dos dois.
  const [ligacao, setLigacao] = useState<Ligacao | null>(null);

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
    </main>
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
} as const;
