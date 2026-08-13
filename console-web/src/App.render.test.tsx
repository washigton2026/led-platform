// ─────────────────────────────────────────────────────────────────────────────
// A rede que segura o refactor.
//
// Os outros testes cobrem `eventos.ts` e `api.ts` — lógica pura, sem React. Um
// refactor que trocasse o ecrã inteiro passaria por eles sem os acordar. Estes olham
// para a marcação.
//
// Os cenários não são inventados: `state` vem dos 8 valores do contrato gerado, os
// códigos (`console.daemon_offline`, `no_show_loaded`) são os que o backend emite, e
// a linha ilegível existe porque o `api.ts` promete guardá-la em vez de a deitar fora.
// ─────────────────────────────────────────────────────────────────────────────

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { App, Daemon, Eventos, Indisponivel, Transporte } from "./App";
import { ESPERADO } from "./marcacao.esperada";
import type { EventoCru } from "./transport/api";

const EVENTOS: readonly EventoCru[] = [
  {
    seq: 2,
    payload: { t_ms: 1200, event: "transitioned", from: "ready", to: "playing" },
    linha: "{}",
  },
  // Ilegível de propósito: o `api.ts` promete que a linha crua fica, e isto é o que
  // verifica que ela chega ao ecrã em vez de ser silenciosamente descartada.
  { seq: 1, payload: null, linha: "{isto nao analisa" },
];

const PROGRESSO = {
  ultima: {
    seq: 9,
    payload: { t_ms: 4000, event: "position_changed", ms: 4000, cause: "sought" },
    linha: "{}",
  } as EventoCru,
  total: 3,
};

const NADA = () => {};

/**
 * O envelope do IPC v1, que o `/api/state` traz junto com o instantâneo. `v: 1` é a
 * versão do protocolo e `ok: true` é literal no contrato — não é um booleano qualquer.
 *
 * O `Daemon` não lê nenhum destes três, e é por isso que acrescentá-los não mudou uma
 * única marcação congelada. Estão aqui porque o TIPO os exige, e um teste que não
 * compila não estava a exercitar o contrato que diz exercitar.
 */
const ENVELOPE = { v: 1, id: 1, ok: true } as const;

/**
 * Um cenário por linha. A chave é a do registo em `marcacao.esperada.ts`, e o
 * `satisfies` garante que não se escreve aqui um nome que lá não exista — nem se
 * esquece um que exista (ver a contagem no fim).
 */
const CASOS: ReadonlyArray<readonly [keyof typeof ESPERADO, JSX.Element]> = [
  // O primeiro fotograma, antes de qualquer resposta: `ligacao` e `fluxo` a `null`.
  // Não é "offline" nem "ok" — é ausência de resposta, e o ecrã tem de o dizer.
  ["APP_INICIAL", <App />],
  [
    "DAEMON",
    <Daemon
      estado={{
        ...ENVELOPE,
        state: "playing",
        position_ms: 4000,
        duration_ms: 8000,
        ticks: 200,
        show_id: 7,
      }}
    />,
  ],
  // `show_id: null` é SEM SHOW. Se isto alguma vez renderizar `0`, o operador lê
  // "show número zero" onde não há show nenhum.
  [
    "DAEMON_SEM_SHOW",
    <Daemon
      estado={{ ...ENVELOPE, state: "idle", position_ms: 0, duration_ms: 0, ticks: 0, show_id: null }}
    />,
  ],
  ["INDISPONIVEL", <Indisponivel code="console.daemon_offline" detail="/tmp/x.sock" />],
  ["EVENTOS_VAZIO", <Eventos eventos={[]} fluxo={null} progresso={{ ultima: null, total: 0 }} />],
  ["EVENTOS_CHEIO", <Eventos eventos={EVENTOS} fluxo={true} progresso={PROGRESSO} />],
  [
    "EVENTOS_EM_BAIXO",
    <Eventos eventos={[]} fluxo={false} progresso={{ ultima: null, total: 0 }} />,
  ],
  [
    "TRANSPORTE",
    <Transporte
      seekMs="4000"
      aoMudarSeek={NADA}
      aoComandar={NADA}
      resultado={{ tipo: "aceite", cmd: "play", corpo: "{}" }}
    />,
  ],
  [
    "TRANSPORTE_RECUSADO",
    <Transporte
      seekMs="0"
      aoMudarSeek={NADA}
      aoComandar={NADA}
      resultado={{ tipo: "recusado", cmd: "play", code: "no_show_loaded", detail: "NoShowLoaded" }}
    />,
  ],
];

describe("marcacao", () => {
  for (const [nome, elemento] of CASOS) {
    it(`${nome} nao mudou`, () => {
      expect(renderToStaticMarkup(elemento)).toBe(ESPERADO[nome]);
    });
  }

  it("nenhum cenario congelado ficou por correr", () => {
    // Sem isto, apagar uma linha de `CASOS` deixaria o registo em
    // `marcacao.esperada.ts` a guardar uma superfície que já ninguém verifica — e o
    // conjunto ficaria verde por não olhar, que é o modo de falha destes testes.
    expect(CASOS.map(([n]) => n).sort()).toEqual(Object.keys(ESPERADO).sort());
  });
});
