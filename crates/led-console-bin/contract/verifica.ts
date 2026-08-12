// ─────────────────────────────────────────────────────────────────────────────
// F7/B — as asserções de COMPILAÇÃO do contrato (ADR-0027).
//
// Este ficheiro é escrito à mão e **não** é gerado. Existe porque `tsc` sobre um
// ficheiro só de tipos prova pouco: prova que o texto é sintaxe válida. O que
// interessa é se os tipos **fazem o que dizem** — e isso só se prova usando-os.
//
// Cada `@ts-expect-error` é uma asserção invertida: se o erro **não** acontecer,
// `tsc` reprova. É assim que se prova, em tempo de compilação, que uma união é
// fechada e que `| null` não é o mesmo que opcional.
//
// Nada aqui corre. É o compilador que é o teste.
// ─────────────────────────────────────────────────────────────────────────────

import {
  type CodigoErro,
  type DaemonState,
  type Elo,
  type EstadoUi,
  type Evento,
  type EstadoDoDaemon,
  type Instantaneo,
  type Resposta,
  type Rota,
  aprova,
  DAEMON_STATES,
  ELOS_POR_FORCA,
  ESTADOS_QUE_APROVAM,
  ESTADOS_UI,
  ROTAS,
} from "./lumyx-contract.generated";

// ── 1. As uniões são FECHADAS ────────────────────────────────────────────────
// Se o backend passar a emitir um valor novo e o contrato não for regenerado, o
// frontend não compila — que é exactamente o efeito desejado.

const estadoValido: EstadoUi = "NOT_MEASURED";

// @ts-expect-error — `HEALTHY` não existe no contrato, e é precisamente o nome que
// alguém inventaria para dizer "está tudo bem". O ADR-0026 proíbe-o; aqui ele não
// é sequer representável.
const estadoInventado: EstadoUi = "HEALTHY";

// @ts-expect-error — `hardware_ok` colapsaria a cadeia de evidência num booleano.
const eloInventado: Elo = "hardware_ok";

// @ts-expect-error — o daemon tem oito estados, e a string vazia não é um deles.
// Foi este o defeito real que a F5 corrigiu na origem (`Snapshot.state`).
const estadoVazio: DaemonState = "";

// @ts-expect-error — um código de erro que o backend nunca emite.
const codigoInventado: CodigoErro = "console.tudo_bem";

// ── 2. O `switch` exaustivo é o alarme ───────────────────────────────────────
// Sem `default`, e com `noFallthroughCasesInSwitch`, um membro novo na união deixa
// esta função sem retorno em todos os caminhos — e `tsc` reprova. É este o
// mecanismo que obriga o frontend a **tratar** o estado novo em vez de o ignorar.

function rotulo(e: EstadoUi): string {
  switch (e) {
    case "PASS":
      return "PASS";
    case "FAIL":
      return "FAIL";
    case "NOT_MEASURED":
      return "NÃO MEDIDO";
    case "BLOCKED":
      return "BLOQUEADO";
    case "RUNNING":
      return "A CORRER";
    case "OFFLINE":
      return "OFFLINE";
    case "DEGRADED":
      return "DEGRADADO";
    case "UNKNOWN":
      return "DESCONHECIDO";
    case "SIMULATION":
      return "SIMULAÇÃO";
  }
}

/** O mesmo para os elos: um elo novo obriga alguém a decidir o que mostrar. */
function rotuloElo(l: Elo): string {
  switch (l) {
    case "software_sent":
      return "enviado pelo software";
    case "network_delivered":
      return "entregue na rede";
    case "controller_received":
      return "recebido pelo controlador";
    case "controller_acknowledged":
      return "confirmado pelo controlador";
    case "led_verified":
      return "LED verificado";
  }
}

// ── 3. `| null` NÃO é opcional ───────────────────────────────────────────────
// A distinção é semântica: "nunca houve instantâneo" contra "idade zero".

const semDado: Instantaneo<number> = { dado: null, estado: "NOT_MEASURED", staleMs: null };
const comDado: Instantaneo<number> = { dado: 42, estado: "PASS", staleMs: 120 };

// @ts-expect-error — `staleMs` é obrigatório. Omitir não é o mesmo que `null`, e se
// fosse opcional este erro não aconteceria — é isso que esta linha prova.
const semIdade: Instantaneo<number> = { dado: 1, estado: "PASS" };

// @ts-expect-error — `undefined` não é `null`. Com `exactOptionalPropertyTypes`, a
// diferença é mantida em vez de apagada.
const idadeUndefined: Instantaneo<number> = { dado: 1, estado: "PASS", staleMs: undefined };

// ── 4. Resposta contra evento: a chave `id` ──────────────────────────────────
// Um evento não tem a chave; uma resposta tem-na, mesmo a `null`.

const respostaRecusada: Resposta = {
  v: 1,
  id: null, // a recusa não-atribuível (linha demasiado longa)
  ok: false,
  error: { code: "bad_request", detail: "linha demasiado longa" },
};

const eventoAssincrono: Evento = { v: 1, async: true, payload: { event: "position_changed" } };

// @ts-expect-error — um evento com `id` deixaria de ser distinguível de uma resposta.
const eventoComId: Evento = { v: 1, async: true, payload: null, id: 7 };

// @ts-expect-error — `id` é obrigatório numa resposta: omiti-lo torna-a um evento.
const respostaSemId: Resposta = { v: 1, ok: true };

// ── 4-bis. O corpo de /api/state ─────────────────────────────────────────────
// O tipo tem de descrever o que o daemon REALMENTE envia — nem mais, nem menos.

const estadoReal: EstadoDoDaemon = {
  v: 1,
  id: 2,
  ok: true,
  state: "idle",
  position_ms: 0,
  duration_ms: 0,
  ticks: 0,
  show_id: null, // sem show carregado
};

const comShow: EstadoDoDaemon = {
  v: 1,
  id: 3,
  ok: true,
  state: "playing",
  position_ms: 1250,
  duration_ms: 8100,
  ticks: 50,
  show_id: 7,
};

// @ts-expect-error — `state` é a união fechada dos oito estados do ADR-0023. Um valor
// inventado não compila, e é isso que impede o frontend de exibir um nono estado.
const estadoImpossivel: EstadoDoDaemon = { ...estadoReal, state: "conectado" };

// @ts-expect-error — `show_id` é `number | null`. `undefined` não é `null`: "sem show" é
// uma afirmação, "não sei" é outra, e o Rust distingue-as com `Option<u64>`.
const showUndefined: EstadoDoDaemon = { ...estadoReal, show_id: undefined };

// @ts-expect-error — omitir `ticks` torna o objecto incompleto. Se o campo fosse opcional,
// este erro não aconteceria — é isso que esta linha prova.
const semTicks: EstadoDoDaemon = {
  v: 1,
  id: 4,
  ok: true,
  state: "idle",
  position_ms: 0,
  duration_ms: 0,
  show_id: null,
};

// @ts-expect-error — os campos são `readonly`: a UI apresenta o estado, não o edita.
estadoReal.position_ms = 999;

/** O `state` alimenta um `switch` exaustivo — o mesmo alarme dos outros enums. */
function descreve(e: EstadoDoDaemon): string {
  switch (e.state) {
    case "idle":
      return "sem show";
    case "loaded":
      return "carregado";
    case "ready":
      return "pronto";
    case "playing":
      return "a tocar";
    case "paused":
      return "em pausa";
    case "stopped":
      return "parado";
    case "finished":
      return "terminado";
    case "error":
      return "em falha";
  }
}

// ── 5. Os dados gerados são usáveis, e imutáveis ─────────────────────────────

const todos: readonly EstadoUi[] = ESTADOS_UI;
const aprovados: readonly EstadoUi[] = ESTADOS_QUE_APROVAM;
const elos: readonly Elo[] = ELOS_POR_FORCA;
const estadosDaemon: readonly DaemonState[] = DAEMON_STATES;
const rotas: readonly Rota[] = ROTAS;

// @ts-expect-error — `readonly` impede alguém de acrescentar um estado em runtime.
ESTADOS_UI.push("PASS");

// @ts-expect-error — nem de reordenar a cadeia de evidência, cuja ordem é do Rust.
ELOS_POR_FORCA[0] = "led_verified";

// ── 6. Consumo mínimo, para nada disto ficar "não usado" ─────────────────────

export const prova = {
  estadoValido,
  estadoInventado,
  eloInventado,
  estadoVazio,
  codigoInventado,
  semDado,
  comDado,
  semIdade,
  idadeUndefined,
  respostaRecusada,
  eventoAssincrono,
  eventoComId,
  respostaSemId,
  rotulos: [rotulo("PASS"), rotuloElo("led_verified")],
  aprovaPass: aprova("PASS"),
  estados: [estadoReal, comShow, estadoImpossivel, showUndefined, semTicks],
  descricao: descreve(estadoReal),
  contagens: [todos.length, aprovados.length, elos.length, estadosDaemon.length, rotas.length],
};
