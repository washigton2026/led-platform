//! Matriz **exaustiva** de transições: todos os 8 estados × 10 comandos = **80 pares**.
//!
//! ## Por que exaustiva, e não "os casos importantes"
//!
//! Uma máquina de estados testada por caminhos felizes prova que os caminhos felizes
//! funcionam. O que interessa numa superfície de transporte de palco é o oposto: **o que
//! acontece quando se carrega no botão errado**. Esta matriz declara o resultado esperado de
//! **cada** par e verifica todos.
//!
//! ## O que torna este gate falsificável (KB-012)
//!
//! 1. A tabela tem de cobrir **exatamente** `8 × 10` pares — acrescentar um estado ou um
//!    comando sem declarar as suas linhas **falha o teste**, em vez de deixar um buraco.
//! 2. Cada par declara o **estado de destino** (ou o **código de recusa**) — não basta
//!    "não entrou em pânico".
//! 3. Depois de **cada** aplicação, as invariantes estruturais são verificadas: uma recusa
//!    não pode mudar o estado, e combinações impossíveis (`Idle` com show, `Error` sem
//!    falha) são apanhadas.

use led_daemon::*;

const DURATION_MS: u64 = 10_000;

fn show() -> ShowDescriptor {
    ShowDescriptor { id: ShowId(42), frame_count: 240, pixel_count: 720, duration_ms: DURATION_MS }
}

/// Os dez comandos, com argumentos representativos e **válidos** — para que uma recusa seja
/// sempre por causa do estado, nunca do argumento.
fn commands() -> Vec<Command> {
    vec![
        Command::Load(show()),
        Command::Unload,
        Command::Arm(PreflightReport::all_clear()),
        Command::Play,
        Command::Pause,
        Command::Stop,
        Command::Seek { to_ms: 1_000 },
        Command::Tick,
        Command::Fault(FaultCode::DeviceLost),
        Command::ClearFault,
    ]
}

/// Constrói uma máquina **no estado pedido**, por um caminho conhecido.
///
/// Devolve também o instante corrente, para que o comando sob teste seja aplicado num tempo
/// coerente com o caminho percorrido.
fn runtime_in(state: State) -> (ShowRuntime, u64) {
    let mut rt = ShowRuntime::new();
    match state {
        State::Idle => (rt, 0),
        State::Loaded => {
            rt.apply(Command::Load(show()), 0).unwrap();
            (rt, 0)
        }
        State::Ready => {
            rt.apply(Command::Load(show()), 0).unwrap();
            rt.apply(Command::Arm(PreflightReport::all_clear()), 0).unwrap();
            (rt, 0)
        }
        State::Playing => {
            rt.apply(Command::Load(show()), 0).unwrap();
            rt.apply(Command::Arm(PreflightReport::all_clear()), 0).unwrap();
            rt.apply(Command::Play, 1_000).unwrap();
            (rt, 1_000)
        }
        State::Paused => {
            let (mut rt, t) = runtime_in(State::Playing);
            rt.apply(Command::Pause, t + 500).unwrap();
            (rt, t + 500)
        }
        State::Stopped => {
            let (mut rt, t) = runtime_in(State::Playing);
            rt.apply(Command::Stop, t + 500).unwrap();
            (rt, t + 500)
        }
        State::Finished => {
            let (mut rt, t) = runtime_in(State::Playing);
            rt.apply(Command::Tick, t + DURATION_MS + 1).unwrap();
            (rt, t + DURATION_MS + 1)
        }
        State::Error => {
            let (mut rt, t) = runtime_in(State::Playing);
            rt.apply(Command::Fault(FaultCode::DeviceLost), t).unwrap();
            (rt, t)
        }
    }
}

/// O que se espera de um par `(estado, comando)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Aceite; a máquina fica neste estado.
    Ok(State),
    /// Recusado com este código; a máquina **não se move**.
    Err(&'static str),
}

/// A tabela declarada. **É a especificação executável do ADR-0023.**
fn expected(state: State, cmd: &Command) -> Expect {
    use Expect::{Err as E, Ok as O};
    match (state, cmd.name()) {
        // ── Idle: sem show, quase tudo é recusado ────────────────────────────
        (State::Idle, "load") => O(State::Loaded),
        (State::Idle, "unload") => E("no_show_loaded"),
        (State::Idle, "arm") => E("no_show_loaded"),
        (State::Idle, "play") => E("no_show_loaded"),
        (State::Idle, "pause") => E("not_applicable"),
        (State::Idle, "stop") => E("no_show_loaded"),
        (State::Idle, "seek") => E("no_show_loaded"),
        (State::Idle, "tick") => O(State::Idle),
        (State::Idle, "fault") => E("no_show_loaded"),
        (State::Idle, "clear_fault") => E("not_applicable"),

        // ── Loaded: carregado, não armado ────────────────────────────────────
        (State::Loaded, "load") => E("show_already_loaded"),
        (State::Loaded, "unload") => O(State::Idle),
        (State::Loaded, "arm") => O(State::Ready),
        (State::Loaded, "play") => E("not_armed"),
        (State::Loaded, "pause") => E("not_applicable"),
        (State::Loaded, "stop") => E("not_applicable"),
        (State::Loaded, "seek") => O(State::Loaded),
        (State::Loaded, "tick") => O(State::Loaded),
        (State::Loaded, "fault") => O(State::Error),
        (State::Loaded, "clear_fault") => E("not_applicable"),

        // ── Ready: armado ────────────────────────────────────────────────────
        (State::Ready, "load") => E("show_already_loaded"),
        (State::Ready, "unload") => O(State::Idle),
        (State::Ready, "arm") => O(State::Ready),
        (State::Ready, "play") => O(State::Playing),
        (State::Ready, "pause") => E("not_applicable"),
        (State::Ready, "stop") => E("not_applicable"),
        (State::Ready, "seek") => O(State::Ready),
        (State::Ready, "tick") => O(State::Ready),
        (State::Ready, "fault") => O(State::Error),
        (State::Ready, "clear_fault") => E("not_applicable"),

        // ── Playing ──────────────────────────────────────────────────────────
        (State::Playing, "load") => E("show_already_loaded"),
        (State::Playing, "unload") => E("not_applicable"),
        (State::Playing, "arm") => E("not_applicable"),
        (State::Playing, "play") => E("not_applicable"),
        (State::Playing, "pause") => O(State::Paused),
        (State::Playing, "stop") => O(State::Stopped),
        (State::Playing, "seek") => O(State::Playing),
        (State::Playing, "tick") => O(State::Playing),
        (State::Playing, "fault") => O(State::Error),
        (State::Playing, "clear_fault") => E("not_applicable"),

        // ── Paused ───────────────────────────────────────────────────────────
        (State::Paused, "load") => E("show_already_loaded"),
        (State::Paused, "unload") => O(State::Idle),
        (State::Paused, "arm") => E("not_applicable"),
        (State::Paused, "play") => O(State::Playing),
        (State::Paused, "pause") => E("not_applicable"),
        (State::Paused, "stop") => O(State::Stopped),
        (State::Paused, "seek") => O(State::Paused),
        (State::Paused, "tick") => O(State::Paused),
        (State::Paused, "fault") => O(State::Error),
        (State::Paused, "clear_fault") => E("not_applicable"),

        // ── Stopped ──────────────────────────────────────────────────────────
        (State::Stopped, "load") => E("show_already_loaded"),
        (State::Stopped, "unload") => O(State::Idle),
        (State::Stopped, "arm") => O(State::Ready),
        (State::Stopped, "play") => O(State::Playing),
        (State::Stopped, "pause") => E("not_applicable"),
        (State::Stopped, "stop") => E("not_applicable"),
        (State::Stopped, "seek") => O(State::Stopped),
        (State::Stopped, "tick") => O(State::Stopped),
        (State::Stopped, "fault") => O(State::Error),
        (State::Stopped, "clear_fault") => E("not_applicable"),

        // ── Finished: `play` recusado de propósito (ADR-0023 §4) ─────────────
        (State::Finished, "load") => E("show_already_loaded"),
        (State::Finished, "unload") => O(State::Idle),
        (State::Finished, "arm") => O(State::Ready),
        (State::Finished, "play") => E("not_applicable"),
        (State::Finished, "pause") => E("not_applicable"),
        (State::Finished, "stop") => O(State::Stopped),
        (State::Finished, "seek") => O(State::Finished),
        (State::Finished, "tick") => O(State::Finished),
        (State::Finished, "fault") => O(State::Error),
        (State::Finished, "clear_fault") => E("not_applicable"),

        // ── Error: absorvente ────────────────────────────────────────────────
        (State::Error, "load") => E("in_error_state"),
        (State::Error, "unload") => O(State::Idle),
        (State::Error, "arm") => E("in_error_state"),
        (State::Error, "play") => E("in_error_state"),
        (State::Error, "pause") => E("in_error_state"),
        (State::Error, "stop") => E("in_error_state"),
        (State::Error, "seek") => E("in_error_state"),
        (State::Error, "tick") => E("in_error_state"),
        (State::Error, "fault") => E("in_error_state"),
        (State::Error, "clear_fault") => O(State::Loaded),

        (s, c) => panic!("par ({s:?}, {c}) não declarado na tabela — declare-o"),
    }
}

/// Invariantes estruturais que **nenhum** caminho pode violar.
fn assert_invariants(rt: &ShowRuntime, ctx: &str) {
    match rt.state() {
        State::Idle => {
            assert!(rt.show().is_none(), "{ctx}: Idle não pode ter show");
            assert_eq!(rt.position_ms(), 0, "{ctx}: Idle tem de estar em zero");
            assert!(rt.fault().is_none(), "{ctx}: Idle não pode ter falha");
        }
        State::Error => {
            assert!(rt.fault().is_some(), "{ctx}: Error TEM de ter falha registada");
        }
        s => {
            assert!(rt.show().is_some(), "{ctx}: {s:?} exige show carregado");
            assert!(rt.fault().is_none(), "{ctx}: só Error carrega falha");
        }
    }
    assert!(rt.state().has_show() == rt.show().is_some(), "{ctx}: has_show() diverge do descritor");
    assert_eq!(
        rt.is_advancing(),
        rt.state() == State::Playing,
        "{ctx}: só Playing avança"
    );
    if let Some(s) = rt.show() {
        assert!(rt.position_ms() <= s.duration_ms, "{ctx}: posição excedeu a duração");
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// **O gate.** Percorre os 80 pares e verifica cada um contra a tabela declarada.
#[test]
fn matriz_completa_de_transicoes() {
    let cmds = commands();
    let mut checked = 0usize;

    for state in State::ALL {
        for cmd in &cmds {
            let (mut rt, now) = runtime_in(state);
            assert_eq!(rt.state(), state, "o construtor não produziu {state:?}");
            assert_invariants(&rt, &format!("antes de ({state:?}, {})", cmd.name()));

            let before = rt.state();
            let result = rt.apply(*cmd, now);
            let ctx = format!("({state:?}, {})", cmd.name());

            match expected(state, cmd) {
                Expect::Ok(target) => {
                    assert!(result.is_ok(), "{ctx}: esperava aceitação, veio {result:?}");
                    assert_eq!(rt.state(), target, "{ctx}: estado de destino errado");
                }
                Expect::Err(code) => {
                    let err = result.expect_err(&format!("{ctx}: esperava recusa"));
                    assert_eq!(err.code(), code, "{ctx}: código de recusa errado ({err:?})");
                    assert_eq!(
                        rt.state(),
                        before,
                        "{ctx}: RECUSA MUDOU O ESTADO — invariante central violada"
                    );
                }
            }

            assert_invariants(&rt, &format!("depois de {ctx}"));
            checked += 1;
        }
    }

    // Exaustividade: um estado ou comando novo sem linhas declaradas falha aqui.
    assert_eq!(
        checked,
        State::ALL.len() * cmds.len(),
        "a matriz tem de cobrir TODOS os pares"
    );
    assert_eq!(checked, 80, "8 estados × 10 comandos");
}

/// Controle negativo do próprio gate: se a tabela estivesse errada, o teste apanharia?
///
/// Sem isto, `matriz_completa_de_transicoes` poderia estar a comparar a implementação
/// consigo mesma. Aqui afirmamos, **à mão**, factos que uma tabela gerada da implementação
/// não poderia inventar.
#[test]
fn controle_negativo_a_tabela_nao_e_derivada_da_implementacao() {
    // Play a partir de Finished é recusado — decisão do ADR-0023 §4, não acidente.
    let (mut rt, t) = runtime_in(State::Finished);
    assert!(rt.apply(Command::Play, t).is_err(), "Finished+Play tem de recusar");

    // ...mas a partir de Stopped é aceite. Se as duas se comportassem igual, a decisão
    // não estaria implementada.
    let (mut rt2, t2) = runtime_in(State::Stopped);
    assert!(rt2.apply(Command::Play, t2).is_ok(), "Stopped+Play tem de aceitar");

    // Unload durante Playing é recusado, mas aceite em Paused — a distinção é deliberada.
    let (mut a, ta) = runtime_in(State::Playing);
    let (mut b, tb) = runtime_in(State::Paused);
    assert!(a.apply(Command::Unload, ta).is_err());
    assert!(b.apply(Command::Unload, tb).is_ok());
}

/// **Transporte não é saída.** Nenhum estado do transporte implica rig apagado, e não existe
/// comando capaz de zerar saída — apagar é blackout, bloqueado pelo ADR-0017.
#[test]
fn nenhum_estado_de_transporte_implica_apagar_o_palco() {
    for state in [State::Paused, State::Stopped, State::Finished] {
        let (rt, _) = runtime_in(state);
        assert!(!rt.is_advancing(), "{state:?} não avança o tempo");
        // A API não expõe nada que apague: `is_advancing` fala de tempo, não de luz.
        // Este teste existe para que acrescentar um comando de blackout aqui obrigue a
        // olhar para o ADR-0017 antes.
        assert!(rt.show().is_some(), "{state:?} continua com o show carregado");
    }
    let nomes: Vec<&str> = commands().iter().map(|c| c.name()).collect();
    for proibido in ["blackout", "dbo", "grand_master", "intensity"] {
        assert!(
            !nomes.contains(&proibido),
            "comando `{proibido}` não pode existir aqui — é saída, e está no ADR-0017"
        );
    }
}
