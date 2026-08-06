//! Gera a **tabela de contrato** do daemon executando a máquina real (GS1.5).
//!
//! ```sh
//! cargo run -p led-daemon --example contract_table
//! ```
//!
//! ## Por que gerar em vez de escrever à mão
//!
//! Uma tabela escrita à mão está correta no instante em que é escrita e começa a apodrecer
//! no commit seguinte. Esta percorre os **80 pares** `(estado × comando)` aplicando-os à
//! `ShowRuntime` de produção e imprime o que ela realmente faz — resultado, eventos emitidos
//! e estado seguinte. Se a implementação mudar, a tabela muda com ela.
//!
//! **Isto não é gate e não é produção.** É ferramenta de documentação: não corre na CI e não
//! afirma veredito nenhum. O gate que trava as transições é
//! `tests/transition_matrix.rs`; este programa só torna o contrato legível.

use led_daemon::*;

const DURATION_MS: u64 = 10_000;

fn show() -> ShowDescriptor {
    ShowDescriptor { id: ShowId(42), frame_count: 240, pixel_count: 720, duration_ms: DURATION_MS }
}

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

/// Constrói a máquina no estado pedido — mesmo caminho do gate, para que a tabela descreva
/// exatamente o que o gate verifica.
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

/// Nome curto e estável do evento, para caber na tabela.
fn ev(e: &Event) -> String {
    match e {
        Event::Transitioned { from, to } => format!("Transitioned({from}→{to})"),
        Event::ShowLoaded(id) => format!("ShowLoaded({})", id.0),
        Event::ShowUnloaded(id) => format!("ShowUnloaded({})", id.0),
        Event::PositionChanged { ms } => format!("PositionChanged({ms})"),
        Event::ReachedEnd => "ReachedEnd".into(),
        Event::Faulted(c) => format!("Faulted({})", c.as_str()),
        Event::FaultCleared => "FaultCleared".into(),
    }
}

fn main() {
    println!("<!-- GERADO por `cargo run -p led-daemon --example contract_table`. Não editar à mão. -->\n");
    println!("| # | Estado atual | Comando | Resultado | Evento(s) emitido(s) | Próximo estado |");
    println!("|--:|---|---|---|---|---|");

    let cmds = commands();
    let mut n = 0usize;
    // Contadores para a auditoria: quantos comandos produzem cada evento.
    let mut pos_changed_por_comando: Vec<&'static str> = Vec::new();
    let mut auto_transicoes: Vec<String> = Vec::new();

    for state in State::ALL {
        for cmd in &cmds {
            n += 1;
            let (mut rt, now) = runtime_in(state);
            let before = rt.state();
            let out = rt.apply(*cmd, now);

            let (resultado, eventos, proximo) = match &out {
                Ok(evs) => {
                    if evs.iter().any(|e| matches!(e, Event::PositionChanged { .. }))
                        && !pos_changed_por_comando.contains(&cmd.name())
                    {
                        pos_changed_por_comando.push(cmd.name());
                    }
                    for e in evs {
                        if let Event::Transitioned { from, to } = e {
                            if from == to {
                                auto_transicoes.push(format!("{}+{}", from, cmd.name()));
                            }
                        }
                    }
                    let lista = if evs.is_empty() {
                        "*(nenhum)*".to_string()
                    } else {
                        evs.iter().map(ev).collect::<Vec<_>>().join(" · ")
                    };
                    ("✅ aceite".to_string(), lista, rt.state().to_string())
                }
                Err(r) => (
                    format!("❌ `{}`", r.code()),
                    "*(nenhum)*".to_string(),
                    format!("{} *(inalterado)*", before),
                ),
            };

            println!(
                "| {n} | `{}` | `{}` | {resultado} | {eventos} | `{proximo}` |",
                state,
                cmd.name()
            );
        }
    }

    println!("\n**{n} pares** — {} estados × {} comandos.\n", State::ALL.len(), cmds.len());

    // ── Sinais para a auditoria do contrato ──────────────────────────────────
    println!("## Sinais extraídos da execução\n");
    println!(
        "- **`PositionChanged` tem {} origens distintas:** `{}`. Um consumidor que receba \
         só o evento **não distingue** um avanço contínuo de um salto do operador.",
        pos_changed_por_comando.len(),
        pos_changed_por_comando.join("`, `")
    );
    if auto_transicoes.is_empty() {
        println!("- Nenhuma auto-transição emite `Transitioned`.");
    } else {
        println!(
            "- **Auto-transições que emitem `Transitioned` com `from == to`:** `{}`. \
             O consumidor recebe um evento de mudança onde nada mudou.",
            auto_transicoes.join("`, `")
        );
    }

    // Injetividade de (from,to) → comando: se duas linhas diferentes produzirem o mesmo par
    // (from,to) por comandos diferentes, o evento `Transitioned` fica ambíguo.
    let mut pares: Vec<(String, &'static str)> = Vec::new();
    for state in State::ALL {
        for cmd in &cmds {
            let (mut rt, now) = runtime_in(state);
            if let Ok(evs) = rt.apply(*cmd, now) {
                for e in &evs {
                    if let Event::Transitioned { from, to } = e {
                        pares.push((format!("{from}→{to}"), cmd.name()));
                    }
                }
            }
        }
    }
    let mut ambiguos: Vec<String> = Vec::new();
    for (par, c) in &pares {
        for (par2, c2) in &pares {
            if par == par2 && c != c2 && !ambiguos.contains(par) {
                ambiguos.push(par.clone());
            }
        }
    }
    if ambiguos.is_empty() {
        println!(
            "- **`Transitioned` é inequívoco:** cada par `(from→to)` é produzido por \
             **exatamente um** comando — o consumidor deduz a causa sem campo extra."
        );
    } else {
        println!("- ⚠️ Pares `(from→to)` produzidos por mais de um comando: `{}`", ambiguos.join("`, `"));
    }
}
