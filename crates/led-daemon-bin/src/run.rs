//! O laço principal do daemon.
//!
//! Tudo aqui é parametrizado por [`Pacer`] e por um sinal de encerramento, para que o laço
//! **inteiro** seja testável sem relógio de parede e sem sinais do SO.

use crate::journal::{event_to_json, notice_to_json, state_to_json, Journal};
use crate::loader::Integrity;
use crate::pacer::Pacer;
use crate::preflight::{preflight, ArtPollPresence, DevicePresence};
use crate::stage::{Stage, StageTick};
use led_daemon::{Command, ShowDescriptor, ShowRuntime, State};
use led_hal::{NetworkGuard, PermissiveGuard, WifiBlockGuard};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// Como o daemon corre.
///
/// Deixou de ser `Copy` quando ganhou `output`: a especificação da saída é uma `String` vinda
/// da CLI. É passada por referência em todo o lado, por isso não custa nada.
#[derive(Clone, Debug)]
pub struct Config {
    /// Período do tick, em ms.
    pub tick_ms: u64,
    /// Encerra após N ticks. `None` = corre até pedirem para parar.
    pub max_ticks: Option<u64>,
    /// Arma e toca logo após carregar.
    pub autoplay: bool,
    /// Encerra ao chegar ao fim do show.
    pub exit_on_finish: bool,
    /// O que se sabe sobre a integridade do artefato.
    pub integrity: Integrity,
    /// `host[:porta]` (ou `proto://host[:porta]`). `None` = **nenhum frame deixa o processo**.
    pub output: Option<String>,
    /// Nome do preset do `HardwareProfile`. **Obrigatório sempre que há `output`**: é dele que
    /// vêm protocolo, ordem de canais, universos, MTU e heartbeat.
    pub profile: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_ms: 20,
            max_ticks: None,
            autoplay: true,
            exit_on_finish: true,
            integrity: Integrity::NotVerified,
            output: None,
            profile: None,
        }
    }
}

/// Por que o laço terminou.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    ShutdownRequested,
    MaxTicks,
    ReachedEnd,
    /// Não conseguiu sequer chegar a tocar (carregar ou armar falhou).
    NeverStarted,
}

/// O que aconteceu na execução.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    pub ticks: u64,
    /// Ticks **saltados** por atraso — nunca acumulados como dívida.
    pub skipped: u64,
    pub final_state: State,
    pub final_position_ms: u64,
    pub reason: ExitReason,
}

/// A guarda de rede a usar. **Com saída é a real** — `WifiBlockGuard`, o gate do ADR-0005.
/// Sem saída não há fio para o WiFi corromper, e a permissiva é a escolha correta (é a mesma
/// regra que o repo já aplica ao `SimulatorDevice`).
fn guarda_para(cfg: &Config) -> Box<dyn NetworkGuard> {
    if cfg.output.is_some() {
        Box::new(WifiBlockGuard)
    } else {
        Box::new(PermissiveGuard)
    }
}

/// Corre o pré-voo, escreve as razões no journal e devolve o relatório para a máquina.
fn preflight_e_registar<P: Pacer, W: Write>(
    cfg: &Config,
    stage: Option<&Stage>,
    pacer: &P,
    journal: &mut Journal<W>,
    presence: &dyn DevicePresence,
) -> led_daemon::PreflightReport {
    let guard = guarda_para(cfg);
    let pf = preflight(cfg.integrity, stage.map(|s| s.output().config()), &*guard, presence);
    let t = pacer.now_ms();
    for (chave, detalhe) in &pf.notices {
        journal.line(&notice_to_json(t, chave, detalhe));
    }
    pf.report
}

/// Abre o palco, se houver saída configurada. Falhar aqui é **não arrancar**: um show que não
/// consegue alcançar o rig não deve fingir que está a tocar.
fn abrir_palco<P: Pacer, W: Write>(
    cfg: &Config,
    path: &str,
    pacer: &P,
    journal: &mut Journal<W>,
) -> Result<Option<Stage>, ()> {
    let Some(spec) = &cfg.output else { return Ok(None) };
    let Some(nome) = &cfg.profile else {
        journal.line(&notice_to_json(
            pacer.now_ms(),
            "output_failed",
            "--output exige --profile: o protocolo e a ordem de canais vem do HardwareProfile",
        ));
        return Err(());
    };
    let perfil = match crate::output::profile_by_name(nome) {
        Ok(p) => p,
        Err(e) => {
            journal.line(&notice_to_json(pacer.now_ms(), "output_failed", &e));
            return Err(());
        }
    };
    journal.line(&notice_to_json(
        pacer.now_ms(),
        "profile",
        &format!(
            "{nome} · {} · {:?} · {} px/universo · MTU {} · heartbeat {} ms",
            perfil.identity.model,
            perfil.capabilities.color,
            perfil.limits.pixels_per_universe,
            perfil.transport.mtu_bytes,
            perfil.transport.heartbeat_ms
        ),
    ));
    match Stage::open(path, spec, &perfil) {
        Ok(s) => {
            journal.line(&notice_to_json(pacer.now_ms(), "output_open", spec));
            Ok(Some(s))
        }
        Err(e) => {
            journal.line(&notice_to_json(pacer.now_ms(), "output_failed", &e));
            Err(())
        }
    }
}

/// Abre o palco no caminho do IPC. **Mesma resolução, sem journal** — o cliente recebe o erro
/// na resposta, que é o canal certo quando quem pediu foi ele.
#[cfg(unix)]
fn abrir_palco_ipc(cfg: &Config, path: &str) -> Result<Option<Stage>, String> {
    let Some(spec) = &cfg.output else { return Ok(None) };
    let nome = cfg.profile.as_ref().ok_or("--output exige --profile")?;
    let perfil = crate::output::profile_by_name(nome)?;
    Stage::open(path, spec, &perfil).map(Some)
}

/// A frase do modo. **Tem de deixar de mentir** quando a saída existe.
fn modo(cfg: &Config, com_ipc: bool) -> String {
    let ipc = if com_ipc { " + ipc" } else { "" };
    match &cfg.output {
        Some(spec) => format!("output={spec}{ipc} — os frames saem deste processo"),
        None => format!("no-output{ipc} — nenhum frame deixa este processo (sem --output)"),
    }
}

/// Um tick do palco, com o erro registado **uma vez** e o laço a continuar.
fn tick_do_palco<P: Pacer, W: Write>(
    stage: &mut Option<Stage>,
    rt: &ShowRuntime,
    now: u64,
    pacer: &P,
    journal: &mut Journal<W>,
    ja_avisou: &mut bool,
) {
    let Some(st) = stage.as_mut() else { return };
    if let StageTick::Failed(e) = st.on_tick(rt.state(), rt.position_ms(), now) {
        // Um erro de rede a 50 Hz encheria o journal e escondia tudo o resto. Regista-se a
        // primeira ocorrência; a contagem completa vive em `OutputStats`.
        if !*ja_avisou {
            *ja_avisou = true;
            journal.line(&notice_to_json(pacer.now_ms(), "output_error", &e));
        }
    }
}

/// Corre o daemon até ao fim, seja qual for a razão.
pub fn run<P: Pacer, W: Write>(
    rt: &mut ShowRuntime,
    path: &str,
    desc: ShowDescriptor,
    cfg: &Config,
    pacer: &mut P,
    journal: &mut Journal<W>,
    shutdown: &AtomicBool,
) -> Outcome {
    macro_rules! emit {
        ($evs:expr) => {{
            let t = pacer.now_ms();
            for e in $evs {
                journal.line(&event_to_json(t, &e));
            }
        }};
    }

    journal.line(&notice_to_json(pacer.now_ms(), "mode", &modo(cfg, false)));

    // ── Carregar ─────────────────────────────────────────────────────────────
    match rt.apply(Command::Load(desc), pacer.now_ms()) {
        Ok(evs) => emit!(evs),
        Err(e) => {
            journal.line(&notice_to_json(pacer.now_ms(), "load_refused", e.code()));
            return finish(rt, pacer, journal, 0, 0, ExitReason::NeverStarted);
        }
    }

    let mut stage = match abrir_palco(cfg, path, pacer, journal) {
        Ok(s) => s,
        Err(()) => return finish(rt, pacer, journal, 0, 0, ExitReason::NeverStarted),
    };

    // ── Armar e tocar ────────────────────────────────────────────────────────
    if cfg.autoplay {
        if cfg.integrity == Integrity::AssumedByOperator {
            journal.line(&notice_to_json(
                pacer.now_ms(),
                "integrity_assumed",
                "AFIRMADA pelo operador, NAO verificada — nenhum hash foi recomputado",
            ));
        }
        let report =
            preflight_e_registar(cfg, stage.as_ref(), pacer, journal, &ArtPollPresence);
        match rt.apply(Command::Arm(report), pacer.now_ms()) {
            Ok(evs) => emit!(evs),
            Err(e) => {
                journal.line(&notice_to_json(pacer.now_ms(), "arm_refused", e.code()));
                return finish(rt, pacer, journal, 0, 0, ExitReason::NeverStarted);
            }
        }
        match rt.apply(Command::Play, pacer.now_ms()) {
            Ok(evs) => emit!(evs),
            Err(e) => {
                journal.line(&notice_to_json(pacer.now_ms(), "play_refused", e.code()));
                return finish(rt, pacer, journal, 0, 0, ExitReason::NeverStarted);
            }
        }
    }

    // ── Laço ─────────────────────────────────────────────────────────────────
    let mut ticks: u64 = 0;
    let mut skipped: u64 = 0;
    let mut ja_avisou = false;
    let period = cfg.tick_ms.max(1); // período zero seria busy-loop por construção
    let mut deadline = pacer.now_ms() + period;

    let reason = loop {
        if shutdown.load(Ordering::Relaxed) {
            break ExitReason::ShutdownRequested;
        }
        if let Some(max) = cfg.max_ticks {
            if ticks >= max {
                break ExitReason::MaxTicks;
            }
        }

        // **É esta linha que garante "sem busy-loop"**: cada iteração espera pelo prazo antes
        // de fazer trabalho. Um pacer de teste conta as esperas, e o gate exige uma por tick.
        pacer.sleep_until(deadline);

        let now = pacer.now_ms();
        emit!(rt.apply(Command::Tick, now).unwrap_or_default());
        tick_do_palco(&mut stage, rt, now, pacer, journal, &mut ja_avisou);
        ticks += 1;

        // Prazo absoluto, não `now + period`: o período não deriva com o custo do trabalho.
        deadline += period;
        // Atraso maior que um período inteiro: **salta**, não acumula dívida. Repor o prazo a
        // partir de `now` é o que impede o laço de entrar em corrida para recuperar — a mesma
        // regra do `Pacing::Absolute` do player (quadro superado é descartado).
        if now >= deadline {
            let atrasados = (now - deadline) / period + 1;
            skipped += atrasados;
            deadline = now + period;
        }

        if cfg.exit_on_finish && rt.state() == State::Finished {
            break ExitReason::ReachedEnd;
        }
    };

    finish(rt, pacer, journal, ticks, skipped, reason)
}

/// Encerramento **limpo**: regista o estado final, o motivo, e faz *flush*.
fn finish<P: Pacer, W: Write>(
    rt: &ShowRuntime,
    pacer: &P,
    journal: &mut Journal<W>,
    ticks: u64,
    skipped: u64,
    reason: ExitReason,
) -> Outcome {
    let t = pacer.now_ms();
    journal.line(&state_to_json(t, rt.state(), rt.position_ms()));
    journal.line(&notice_to_json(
        t,
        "shutdown",
        &format!("{reason:?} · ticks={ticks} · skipped={skipped}"),
    ));
    // Sem este flush, um encerramento por `--max-ticks` podia perder as últimas linhas.
    journal.flush();
    Outcome {
        ticks,
        skipped,
        final_state: rt.state(),
        final_position_ms: rt.position_ms(),
        reason,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GS3 — o laço com plano de controlo
// ─────────────────────────────────────────────────────────────────────────────

/// Campos de resposta ao cliente, ou a recusa.
#[cfg(unix)]
pub type IpcReply = Result<Vec<(&'static str, String)>, crate::proto::ProtoError>;

/// O que aplicar um comando de IPC produz: a resposta **e** os eventos a difundir.
#[cfg(unix)]
pub type IpcOutcome = (IpcReply, Vec<led_daemon::Event>);

/// Aplica um comando vindo do IPC. **Só o laço chama isto** — é o aplicador único.
#[cfg(unix)]
fn apply_ipc(
    rt: &mut ShowRuntime,
    cmd: &crate::proto::Cmd,
    now: u64,
    cfg: &Config,
    stage: &mut Option<Stage>,
    presence: &dyn DevicePresence,
) -> IpcOutcome {
    use crate::proto::Cmd;
    use crate::server::{load_error, runtime_result_to_reply};
    use led_daemon::{Command, ShowId};

    // `load` é o único que traduz argumentos externos em estado: lê o ficheiro e, se a
    // integridade for AFIRMADA, arma logo a seguir. Sem essa afirmação fica em `Loaded` e o
    // `play` recusa com `not_armed` — o gate do ADR-0023 fica **visível no fio**, em vez de
    // ser escondido por um arm implícito.
    if let Cmd::Load { path, assume_integrity } = cmd {
        let desc = match crate::loader::descriptor_from_path(path, ShowId(1)) {
            Ok(d) => d,
            Err(e) => return (Err(load_error(e.to_string())), Vec::new()),
        };
        let mut eventos = match rt.apply(Command::Load(desc), now) {
            Ok(evs) => evs,
            Err(rej) => {
                return (runtime_result_to_reply(Err(rej), rt.state(), rt.position_ms()), Vec::new())
            }
        };
        // O palco é **do show**: um `load` novo dimensiona a saída pelo artefato que chegou.
        // Reabrir aqui é o que impede um show de 720 px de sair por uma saída de 300.
        if cfg.output.is_some() {
            match abrir_palco_ipc(cfg, path) {
                Ok(s) => *stage = s,
                Err(e) => return (Err(load_error(e)), eventos),
            }
        }
        if *assume_integrity {
            let guard = guarda_para(cfg);
            let report = preflight(
                Integrity::AssumedByOperator,
                stage.as_ref().map(|s| s.output().config()),
                &*guard,
                presence,
            )
            .report;
            match rt.apply(Command::Arm(report), now) {
                Ok(evs) => eventos.extend(evs),
                Err(rej) => {
                    return (
                        runtime_result_to_reply(Err(rej), rt.state(), rt.position_ms()),
                        eventos,
                    )
                }
            }
        }
        let reply = runtime_result_to_reply(Ok(eventos.clone()), rt.state(), rt.position_ms());
        return (reply, eventos);
    }

    let core = match cmd {
        Cmd::Unload => Command::Unload,
        Cmd::Play => Command::Play,
        Cmd::Pause => Command::Pause,
        Cmd::Stop => Command::Stop,
        Cmd::Seek { to_ms } => Command::Seek { to_ms: *to_ms },
        outro => {
            return (
                Err(crate::proto::ProtoError::new(
                    crate::proto::code::UNKNOWN_COMMAND,
                    outro.name(),
                )),
                Vec::new(),
            )
        }
    };
    let r = rt.apply(core, now);
    let eventos = r.clone().unwrap_or_default();
    (runtime_result_to_reply(r, rt.state(), rt.position_ms()), eventos)
}

/// O laço, com plano de controlo ligado.
///
/// `inicial` é opcional: com IPC o show pode chegar por `load`, e o daemon arranca em `Idle`
/// à espera. Sem plano de controlo isso seria um daemon que nunca faz nada — com ele, é o
/// modo normal de operação.
#[cfg(unix)]
pub fn run_with_control<P: Pacer, W: Write>(
    rt: &mut ShowRuntime,
    inicial: Option<(String, ShowDescriptor)>,
    cfg: &Config,
    pacer: &mut P,
    journal: &mut Journal<W>,
    shutdown: &AtomicBool,
    cp: &crate::server::ControlPlane,
) -> Outcome {
    use led_daemon::Command;

    journal.line(&notice_to_json(pacer.now_ms(), "mode", &modo(cfg, true)));

    let presence = ArtPollPresence;
    let mut stage: Option<Stage> = None;
    let mut duration_ms = 0;
    if let Some((path, desc)) = inicial {
        duration_ms = desc.duration_ms;
        if let Ok(evs) = rt.apply(Command::Load(desc), pacer.now_ms()) {
            for e in &evs {
                journal.line(&event_to_json(pacer.now_ms(), e));
            }
        }
        stage = match abrir_palco(cfg, &path, pacer, journal) {
            Ok(s) => s,
            Err(()) => return finish(rt, pacer, journal, 0, 0, ExitReason::NeverStarted),
        };
        if cfg.autoplay && cfg.integrity == Integrity::AssumedByOperator {
            let report =
                preflight_e_registar(cfg, stage.as_ref(), pacer, journal, &presence);
            let _ = rt.apply(Command::Arm(report), pacer.now_ms());
            let _ = rt.apply(Command::Play, pacer.now_ms());
        }
    }

    let mut ticks: u64 = 0;
    let mut skipped: u64 = 0;
    let mut ja_avisou = false;
    let period = cfg.tick_ms.max(1);
    let mut deadline = pacer.now_ms() + period;

    let reason = loop {
        if shutdown.load(Ordering::Relaxed) {
            break ExitReason::ShutdownRequested;
        }
        if let Some(max) = cfg.max_ticks {
            if ticks >= max {
                break ExitReason::MaxTicks;
            }
        }

        // ── Comandos do IPC, aplicados NO LIMITE DO TICK ─────────────────────
        for job in cp.drain_jobs() {
            let now = pacer.now_ms();
            let (reply, eventos) = apply_ipc(rt, &job.cmd, now, cfg, &mut stage, &presence);
            for e in &eventos {
                let linha = event_to_json(now, e);
                journal.line(&linha);
                cp.broadcast(&linha);
            }
            if let Some(d) = rt.show().map(|s| s.duration_ms) {
                duration_ms = d;
            }
            let _ = job.reply.send(reply); // cliente desligou: não é erro do daemon
        }

        pacer.sleep_until(deadline);
        let now = pacer.now_ms();
        for e in rt.apply(Command::Tick, now).unwrap_or_default() {
            let linha = event_to_json(now, &e);
            journal.line(&linha);
            cp.broadcast(&linha);
        }
        tick_do_palco(&mut stage, rt, now, pacer, journal, &mut ja_avisou);
        ticks += 1;

        // Snapshot para o `status` — publicado, não bloqueado atrás do runtime.
        *cp.snapshot.lock().expect("snapshot") = crate::server::Snapshot {
            state: rt.state().as_str().to_string(),
            position_ms: rt.position_ms(),
            show_id: rt.show().map(|s| s.id.0),
            duration_ms,
            ticks,
        };

        deadline += period;
        if now >= deadline {
            skipped += (now - deadline) / period + 1;
            deadline = now + period;
        }

        if cfg.exit_on_finish && rt.state() == State::Finished {
            break ExitReason::ReachedEnd;
        }
    };

    finish(rt, pacer, journal, ticks, skipped, reason)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::pacer::test_pacer::VirtualPacer;
    use led_daemon::ShowId;

    fn desc(duration_ms: u64) -> ShowDescriptor {
        ShowDescriptor { id: ShowId(1), frame_count: 100, pixel_count: 720, duration_ms }
    }

    fn cfg(max_ticks: Option<u64>) -> Config {
        Config {
            tick_ms: 20,
            max_ticks,
            autoplay: true,
            exit_on_finish: true,
            integrity: Integrity::AssumedByOperator,
            output: None,
            profile: None,
        }
    }

    /// Corre e devolve `(resultado, esperas do pacer, linhas do journal)`.
    fn corre(cfg: Config, duration_ms: u64, stop_after: Option<u64>) -> (Outcome, Vec<u64>, String) {
        let mut rt = ShowRuntime::new();
        let mut p = VirtualPacer::new();
        let mut buf = Vec::new();
        let flag = AtomicBool::new(false);
        let out = {
            let mut j = Journal::new(&mut buf);
            if let Some(_n) = stop_after {
                flag.store(true, Ordering::Relaxed); // pede encerramento antes do 1.º tick
            }
            run(&mut rt, "", desc(duration_ms), &cfg, &mut p, &mut j, &flag)
        };
        (out, p.sleeps, String::from_utf8(buf).unwrap())
    }

    #[test]
    fn carrega_arma_toca_e_encerra_no_fim() {
        let (out, _, log) = corre(cfg(None), 200, None);
        assert_eq!(out.reason, ExitReason::ReachedEnd);
        assert_eq!(out.final_state, State::Finished);
        assert_eq!(out.final_position_ms, 200);
        assert!(log.contains(r#""event":"show_loaded""#));
        assert!(log.contains(r#""from":"ready","to":"playing""#));
        assert!(log.contains(r#""event":"reached_end""#));
        assert!(log.contains(r#""notice":"shutdown""#));
    }

    /// **Sem busy-loop, provado sem relógio de parede.** Uma espera por tick, sempre.
    #[test]
    fn cada_iteracao_espera_antes_de_trabalhar() {
        let (out, sleeps, _) = corre(cfg(Some(5)), 10_000, None);
        assert_eq!(out.ticks, 5);
        assert_eq!(sleeps.len(), 5, "uma espera por tick — menos que isso é busy-loop");
        // Prazos absolutos e equiespaçados: o período não deriva.
        assert_eq!(sleeps, vec![20, 40, 60, 80, 100]);
    }

    #[test]
    fn periodo_zero_nao_vira_busy_loop() {
        let mut c = cfg(Some(3));
        c.tick_ms = 0;
        let (out, sleeps, _) = corre(c, 10_000, None);
        assert_eq!(out.ticks, 3);
        assert_eq!(sleeps.len(), 3, "mesmo com tick_ms=0 há uma espera por iteração");
    }

    /// Atraso maior que um período **salta** ticks em vez de os acumular.
    #[test]
    fn tick_superado_e_saltado_nao_acumulado() {
        let mut rt = ShowRuntime::new();
        let mut p = VirtualPacer::with_lag(100); // 5 períodos de atraso por espera
        let mut buf = Vec::new();
        let flag = AtomicBool::new(false);
        let out = {
            let mut j = Journal::new(&mut buf);
            run(&mut rt, "", desc(100_000), &cfg(Some(4)), &mut p, &mut j, &flag)
        };
        assert_eq!(out.ticks, 4);
        assert!(out.skipped > 0, "atraso de 5 períodos tem de contar como saltos");
    }

    #[test]
    fn encerramento_pedido_e_limpo() {
        let (out, _, log) = corre(cfg(None), 10_000, Some(0));
        assert_eq!(out.reason, ExitReason::ShutdownRequested);
        assert_eq!(out.ticks, 0, "nem chega a ticar");
        assert!(log.contains(r#""notice":"state""#), "o estado final é registado");
        assert!(log.trim_end().ends_with('}'), "a última linha está completa (houve flush)");
    }

    /// Sem integridade afirmada, o pré-voo **reprova** e o daemon não toca. É o gate a
    /// funcionar: nada de tocar um artefato sobre o qual não se sabe nada.
    #[test]
    fn sem_integridade_afirmada_o_preflight_reprova() {
        let mut c = cfg(Some(3));
        c.integrity = Integrity::NotVerified;
        let (out, _, log) = corre(c, 1_000, None);
        assert_eq!(out.reason, ExitReason::NeverStarted);
        assert_eq!(out.final_state, State::Loaded, "carregou, mas não armou");
        assert!(log.contains(r#""notice":"arm_refused","detail":"preflight_failed""#), "{log}");
    }

    #[test]
    fn a_vacuidade_do_preflight_esta_registada_no_journal() {
        let (_, _, log) = corre(cfg(Some(1)), 10_000, None);
        assert!(log.contains(r#""notice":"preflight_vacuous""#), "a vacuidade tem de ser dita");
        assert!(log.contains(r#""notice":"integrity_assumed""#), "e a afirmação também");
        assert!(log.contains("no-output"), "e o modo sem saída também");
    }

    #[test]
    fn sem_autoplay_fica_carregado_e_nao_toca() {
        let mut c = cfg(Some(3));
        c.autoplay = false;
        let (out, sleeps, _) = corre(c, 10_000, None);
        assert_eq!(out.final_state, State::Loaded);
        assert_eq!(out.final_position_ms, 0, "Tick fora de Playing é inerte (F1)");
        assert_eq!(sleeps.len(), 3, "mas o laço continua a respeitar a cadência");
    }
}
