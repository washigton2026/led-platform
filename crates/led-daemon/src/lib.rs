//! # led-daemon — a superfície de transporte do engine (ADR-0023)
//!
//! O estado de show **em runtime**: o que o console vai comandar e o que o `led-player`
//! nunca teve. Até aqui o player tocava um `.lumyx` linearmente do início ao fim e não havia
//! noção de posição em runtime — a lacuna que `docs/architecture/control-protocol.md`
//! mediu e que bloqueava toda a FASE D.
//!
//! ## As três propriedades que este módulo garante
//!
//! 1. **Estados inválidos são irrepresentáveis.** [`State`] é um enum fechado e a única forma
//!    de mudar de estado é [`ShowRuntime::apply`]. Não há flags booleanas que possam entrar em
//!    contradição (`playing && paused` não existe porque não é escrevível).
//! 2. **Transições são determinísticas.** O tempo é **injetado** (`apply(cmd, now_ms)`); a
//!    máquina nunca lê o relógio. A mesma sequência de `(comando, now_ms)` dá sempre o mesmo
//!    estado — e nenhum teste precisa dormir.
//! 3. **Recusa nunca altera o estado.** Um comando inadmissível devolve [`Rejected`] com
//!    motivo tipado e a máquina fica **exatamente** onde estava.
//!
//! ## ⚠️ Transporte não é saída — `Stop` e `Pause` NÃO apagam o palco
//!
//! Esta é a invariante que mais importa aqui, e é deliberada (ADR-0023 §3).
//!
//! Em `Paused`, `Stopped` e `Finished` o **heartbeat continua a reenviar o último frame
//! válido** e o rig **continua aceso**. Parar o transporte é parar o *avanço do tempo*, não
//! a saída.
//!
//! Apagar o palco é **blackout**, é uma máscara de saída noutra camada, e está **bloqueado
//! pelo [ADR-0017]**. Esta máquina não tem — e não pode ganhar — nenhum comando que zere
//! saída. Se um dia parecer que precisa, isso é sinal de que o blackout está a ser
//! implementado no sítio errado.
//!
//! [ADR-0017]: https://example.invalid/adr-0017
//!
//! ## Exemplo
//!
//! ```
//! use led_daemon::*;
//!
//! let mut rt = ShowRuntime::new();
//! assert_eq!(rt.state(), State::Idle);
//!
//! let show = ShowDescriptor { id: ShowId(7), frame_count: 100, pixel_count: 720, duration_ms: 4_000 };
//! rt.apply(Command::Load(show), 0).unwrap();
//! assert_eq!(rt.state(), State::Loaded);
//!
//! // Carregar não é estar pronto: o pré-voo é um veredito INJETADO.
//! rt.apply(Command::Arm(PreflightReport::all_clear()), 0).unwrap();
//! assert_eq!(rt.state(), State::Ready);
//!
//! rt.apply(Command::Play, 1_000).unwrap();
//! rt.apply(Command::Tick, 3_500).unwrap();
//! assert_eq!(rt.position_ms(), 2_500);
//!
//! // Pausar preserva a posição — e NÃO apaga o rig.
//! rt.apply(Command::Pause, 3_500).unwrap();
//! assert_eq!(rt.position_ms(), 2_500);
//! ```

#![forbid(unsafe_code)]

use std::fmt;

// ─────────────────────────────────────────────────────────────────────────────
// Estado
// ─────────────────────────────────────────────────────────────────────────────

/// O ciclo de vida do show em runtime. **Fechado por construção** — nenhum outro estado é
/// representável, e é isso que torna "sem estados inválidos" uma propriedade do tipo em vez
/// de uma promessa da documentação.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// Nenhum show carregado.
    Idle,
    /// Artefato carregado; **pré-condições ainda não verificadas**.
    Loaded,
    /// Pré-voo aprovado — **armado**, seguro para tocar.
    Ready,
    /// Transporte a correr.
    Playing,
    /// Transporte parado, **posição preservada**. O rig continua aceso.
    Paused,
    /// Transporte parado, **posição em zero**. O rig continua aceso.
    Stopped,
    /// Chegou ao fim naturalmente. O rig continua aceso.
    Finished,
    /// Falha de runtime registada.
    Error,
}

impl State {
    /// Todos os estados — a base da matriz exaustiva de transições.
    pub const ALL: [State; 8] = [
        State::Idle,
        State::Loaded,
        State::Ready,
        State::Playing,
        State::Paused,
        State::Stopped,
        State::Finished,
        State::Error,
    ];

    /// Nome estável para JSON e diagnóstico. **Não** derivado de `Debug`: `Debug` pode mudar
    /// com um refactor e isto é superfície observável pelo control-plane.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Loaded => "loaded",
            State::Ready => "ready",
            State::Playing => "playing",
            State::Paused => "paused",
            State::Stopped => "stopped",
            State::Finished => "finished",
            State::Error => "error",
        }
    }

    /// Há um show carregado neste estado?
    pub fn has_show(self) -> bool {
        !matches!(self, State::Idle)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Identificador opaco do show. Opaco de propósito: o runtime não interpreta, só compara.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShowId(pub u64);

/// O que o runtime precisa saber sobre um show — e **só** isso.
///
/// Não há caminho de arquivo, nem frames, nem dispositivos: quem carrega o `.lumyx` é o
/// daemon, e passa o descritor como dado (ADR-0023 §7). É o que mantém este crate leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowDescriptor {
    pub id: ShowId,
    pub frame_count: u64,
    pub pixel_count: u32,
    pub duration_ms: u64,
}

/// Veredito dos gates de pré-voo. **Recebido**, nunca calculado aqui.
///
/// Os três campos espelham gates que já existem no repo: integridade é o `--verify <hash>`
/// do player, rede é `Hal::check_network()` (regra WiFi do ADR-0005) e presença é o
/// discovery pré-show (`--require-all`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PreflightReport {
    pub integrity_verified: bool,
    pub network_ok: bool,
    pub devices_present: bool,
}

impl PreflightReport {
    /// Todos os gates aprovados.
    pub fn all_clear() -> Self {
        Self { integrity_verified: true, network_ok: true, devices_present: true }
    }
    /// Nenhuma reprovação.
    pub fn is_clear(&self) -> bool {
        self.integrity_verified && self.network_ok && self.devices_present
    }
    /// Nomes dos gates reprovados, na ordem em que são declarados.
    pub fn failures(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if !self.integrity_verified {
            v.push("integrity");
        }
        if !self.network_ok {
            v.push("network");
        }
        if !self.devices_present {
            v.push("devices");
        }
        v
    }
}

/// Classe da falha de runtime. Enumerada, nunca string livre — mesma regra do modelo de erro
/// do `control-protocol.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultCode {
    /// Perda de ligação a um controlador.
    DeviceLost,
    /// A fonte de frames falhou (leitura, decodificação).
    SourceFailed,
    /// Violação de política detetada em runtime (ex.: WiFi ficou ativo — ADR-0005).
    PolicyViolation,
    /// O motor não conseguiu manter a cadência.
    OutputStalled,
}

impl FaultCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FaultCode::DeviceLost => "device_lost",
            FaultCode::SourceFailed => "source_failed",
            FaultCode::PolicyViolation => "policy_violation",
            FaultCode::OutputStalled => "output_stalled",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Comandos
// ─────────────────────────────────────────────────────────────────────────────

/// O que se pode pedir ao runtime.
///
/// **Não existe comando de blackout, nem de intensidade.** Isso é saída, não transporte, e
/// está bloqueado pelo ADR-0017.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Carrega um descritor. Só a partir de `Idle`.
    Load(ShowDescriptor),
    /// Descarrega e volta a `Idle`. **Recusado durante `Playing`** — parar primeiro.
    Unload,
    /// Arma com o veredito de pré-voo. Reprovado ⇒ recusa, e o estado **não muda**.
    Arm(PreflightReport),
    /// Toca. A partir de `Paused`, **retoma** (ADR-0023: não há comando `Resume` próprio).
    Play,
    /// Pausa preservando a posição. **Não apaga o rig.**
    Pause,
    /// Para e põe a posição em **zero**. **Não apaga o rig.**
    Stop,
    /// Salta para um instante absoluto.
    Seek { to_ms: u64 },
    /// Avanço de tempo. Só tem efeito em `Playing`; noutros estados é aceite e inócuo,
    /// porque o daemon tica em cadência fixa e não deve ter de saber o estado para o fazer.
    Tick,
    /// O motor reporta uma falha de runtime.
    Fault(FaultCode),
    /// Limpa a falha e volta ao estado carregado.
    ClearFault,
}

impl Command {
    /// Nome estável, para diagnóstico e para a matriz de transições.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Load(_) => "load",
            Command::Unload => "unload",
            Command::Arm(_) => "arm",
            Command::Play => "play",
            Command::Pause => "pause",
            Command::Stop => "stop",
            Command::Seek { .. } => "seek",
            Command::Tick => "tick",
            Command::Fault(_) => "fault",
            Command::ClearFault => "clear_fault",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recusas e eventos
// ─────────────────────────────────────────────────────────────────────────────

/// Por que um comando foi recusado. **Uma recusa nunca altera o estado.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// O comando exige um show carregado.
    NoShowLoaded,
    /// Já há um show carregado — descarregar primeiro.
    ShowAlreadyLoaded(ShowId),
    /// O pré-voo reprovou; os gates falhados vêm nomeados.
    PreflightFailed(PreflightReport),
    /// Tocar exige estar armado (`Ready`) — passar por `Arm` primeiro.
    NotArmed,
    /// `Seek` para fora de `[0, duration_ms]`.
    SeekOutOfRange { to_ms: u64, duration_ms: u64 },
    /// A máquina está em `Error`; só `ClearFault`/`Unload` são aceites.
    InErrorState(FaultCode),
    /// O comando não se aplica a este estado.
    NotApplicable { state: State, command: &'static str },
}

impl Rejected {
    /// Código enumerado, para o control-plane (nunca string livre).
    pub fn code(&self) -> &'static str {
        match self {
            Rejected::NoShowLoaded => "no_show_loaded",
            Rejected::ShowAlreadyLoaded(_) => "show_already_loaded",
            Rejected::PreflightFailed(_) => "preflight_failed",
            Rejected::NotArmed => "not_armed",
            Rejected::SeekOutOfRange { .. } => "seek_out_of_range",
            Rejected::InErrorState(_) => "in_error_state",
            Rejected::NotApplicable { .. } => "not_applicable",
        }
    }
}

/// O que aconteceu. Emitido **só** quando o comando é aceite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Transitioned { from: State, to: State },
    ShowLoaded(ShowId),
    ShowUnloaded(ShowId),
    PositionChanged { ms: u64 },
    ReachedEnd,
    Faulted(FaultCode),
    FaultCleared,
}

// ─────────────────────────────────────────────────────────────────────────────
// A máquina
// ─────────────────────────────────────────────────────────────────────────────

/// O estado de show em runtime.
///
/// Instância única por engine — ver o critério de reversão do ADR-0023 (dois shows a tocar
/// ao mesmo tempo obrigariam a um modelo por-*deck*, que não existe hoje).
#[derive(Clone, Debug)]
pub struct ShowRuntime {
    state: State,
    show: Option<ShowDescriptor>,
    /// Posição do transporte, em ms desde o início do show.
    position_ms: u64,
    /// Instante injetado em que o `Play` corrente começou, e a posição nesse instante.
    /// `None` fora de `Playing`.
    play_started_at: Option<u64>,
    play_started_from: u64,
    fault: Option<FaultCode>,
    /// Último `now_ms` visto — guarda de monotonicidade (precedente do `SharedClock`).
    last_now_ms: u64,
}

impl Default for ShowRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ShowRuntime {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            show: None,
            position_ms: 0,
            play_started_at: None,
            play_started_from: 0,
            fault: None,
            last_now_ms: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn position_ms(&self) -> u64 {
        self.position_ms
    }
    pub fn show(&self) -> Option<&ShowDescriptor> {
        self.show.as_ref()
    }
    pub fn fault(&self) -> Option<FaultCode> {
        self.fault
    }

    /// O transporte está a avançar? **Não** diz se o rig está aceso — o rig está sempre
    /// aceso enquanto o heartbeat correr. Ver a nota de módulo.
    pub fn is_advancing(&self) -> bool {
        self.state == State::Playing
    }

    /// Snapshot para o read-model. JSON à mão, sem `serde` — convenção do workspace.
    pub fn to_json(&self) -> String {
        let (id, frames, px, dur) = match self.show {
            Some(s) => (s.id.0 as i64, s.frame_count as i64, s.pixel_count as i64, s.duration_ms as i64),
            None => (-1, -1, -1, -1),
        };
        let fault = match self.fault {
            Some(f) => format!("\"{}\"", f.as_str()),
            None => "null".to_string(),
        };
        format!(
            "{{\"state\":\"{}\",\"position_ms\":{},\"advancing\":{},\"show\":{{\"id\":{},\"frame_count\":{},\"pixel_count\":{},\"duration_ms\":{}}},\"fault\":{}}}",
            self.state.as_str(),
            self.position_ms,
            self.is_advancing(),
            id,
            frames,
            px,
            dur,
            fault
        )
    }

    /// Aplica um comando num instante **injetado**.
    ///
    /// `now_ms` é tempo absoluto de uma fonte monotónica do daemon. A máquina nunca lê o
    /// relógio: é isso que torna as transições determinísticas e os testes livres de espera.
    ///
    /// Devolve os eventos em caso de aceitação, ou [`Rejected`] — e **numa recusa o estado
    /// fica exatamente como estava**.
    pub fn apply(&mut self, cmd: Command, now_ms: u64) -> Result<Vec<Event>, Rejected> {
        // Guarda de monotonicidade: um relógio que anda para trás é clampado, não pânico.
        // Mesmo tratamento que o `SharedClock` já dá — num sistema ao vivo, entrar em pânico
        // por um salto de relógio é pior que continuar com o último valor bom.
        let now_ms = now_ms.max(self.last_now_ms);
        self.last_now_ms = now_ms;

        // `Error` é absorvente: só se sai por `ClearFault` ou `Unload`.
        if self.state == State::Error
            && !matches!(cmd, Command::ClearFault | Command::Unload)
        {
            return Err(Rejected::InErrorState(
                self.fault.expect("estado Error sempre tem falha registada"),
            ));
        }

        match cmd {
            Command::Load(desc) => self.cmd_load(desc),
            Command::Unload => self.cmd_unload(),
            Command::Arm(report) => self.cmd_arm(report),
            Command::Play => self.cmd_play(now_ms),
            Command::Pause => self.cmd_pause(now_ms),
            Command::Stop => self.cmd_stop(),
            Command::Seek { to_ms } => self.cmd_seek(to_ms, now_ms),
            Command::Tick => Ok(self.cmd_tick(now_ms)),
            Command::Fault(code) => self.cmd_fault(code, now_ms),
            Command::ClearFault => self.cmd_clear_fault(),
        }
    }

    // ── handlers ─────────────────────────────────────────────────────────────

    fn transition(&mut self, to: State) -> Event {
        let from = self.state;
        self.state = to;
        Event::Transitioned { from, to }
    }

    fn cmd_load(&mut self, desc: ShowDescriptor) -> Result<Vec<Event>, Rejected> {
        if self.state != State::Idle {
            return Err(match self.show {
                Some(s) => Rejected::ShowAlreadyLoaded(s.id),
                None => Rejected::NotApplicable { state: self.state, command: "load" },
            });
        }
        self.show = Some(desc);
        self.position_ms = 0;
        Ok(vec![self.transition(State::Loaded), Event::ShowLoaded(desc.id)])
    }

    fn cmd_unload(&mut self) -> Result<Vec<Event>, Rejected> {
        match self.state {
            State::Idle => Err(Rejected::NoShowLoaded),
            // Descarregar com o transporte a correr é a classe de erro que apaga um palco a
            // meio do número. Parar primeiro é explícito e barato.
            State::Playing => Err(Rejected::NotApplicable { state: self.state, command: "unload" }),
            _ => {
                let id = self.show.expect("estado com show tem descritor").id;
                self.show = None;
                self.position_ms = 0;
                self.play_started_at = None;
                self.play_started_from = 0;
                self.fault = None;
                Ok(vec![self.transition(State::Idle), Event::ShowUnloaded(id)])
            }
        }
    }

    fn cmd_arm(&mut self, report: PreflightReport) -> Result<Vec<Event>, Rejected> {
        match self.state {
            State::Idle => Err(Rejected::NoShowLoaded),
            State::Loaded | State::Stopped | State::Finished | State::Ready => {
                if !report.is_clear() {
                    // Recusa NÃO muda o estado: o operador corrige a rede e re-arma.
                    return Err(Rejected::PreflightFailed(report));
                }
                Ok(vec![self.transition(State::Ready)])
            }
            _ => Err(Rejected::NotApplicable { state: self.state, command: "arm" }),
        }
    }

    fn cmd_play(&mut self, now_ms: u64) -> Result<Vec<Event>, Rejected> {
        match self.state {
            State::Idle => Err(Rejected::NoShowLoaded),
            State::Loaded => Err(Rejected::NotArmed),
            State::Ready | State::Paused | State::Stopped => {
                self.play_started_at = Some(now_ms);
                self.play_started_from = self.position_ms;
                Ok(vec![self.transition(State::Playing)])
            }
            State::Playing => Err(Rejected::NotApplicable { state: self.state, command: "play" }),
            // ADR-0023 §4: rebobinar implicitamente faria um show recomeçar no palco com um
            // toque acidental. O caminho é `Stop`/`Seek` e depois `Play`.
            State::Finished => Err(Rejected::NotApplicable { state: self.state, command: "play" }),
            State::Error => unreachable!("Error é filtrado em apply()"),
        }
    }

    fn cmd_pause(&mut self, now_ms: u64) -> Result<Vec<Event>, Rejected> {
        if self.state != State::Playing {
            return Err(Rejected::NotApplicable { state: self.state, command: "pause" });
        }
        self.advance_position(now_ms);
        self.play_started_at = None;
        Ok(vec![self.transition(State::Paused), Event::PositionChanged { ms: self.position_ms }])
    }

    fn cmd_stop(&mut self) -> Result<Vec<Event>, Rejected> {
        match self.state {
            State::Playing | State::Paused | State::Finished => {
                self.position_ms = 0; // ADR-0023 §5 — como o xLights
                self.play_started_at = None;
                self.play_started_from = 0;
                Ok(vec![self.transition(State::Stopped), Event::PositionChanged { ms: 0 }])
            }
            State::Idle => Err(Rejected::NoShowLoaded),
            _ => Err(Rejected::NotApplicable { state: self.state, command: "stop" }),
        }
    }

    fn cmd_seek(&mut self, to_ms: u64, now_ms: u64) -> Result<Vec<Event>, Rejected> {
        let Some(show) = self.show else {
            return Err(Rejected::NoShowLoaded);
        };
        if !matches!(
            self.state,
            State::Loaded | State::Ready | State::Playing | State::Paused | State::Stopped | State::Finished
        ) {
            return Err(Rejected::NotApplicable { state: self.state, command: "seek" });
        }
        if to_ms > show.duration_ms {
            return Err(Rejected::SeekOutOfRange { to_ms, duration_ms: show.duration_ms });
        }
        self.position_ms = to_ms;
        if self.state == State::Playing {
            // Re-baseia a origem para que o próximo `Tick` conte a partir daqui.
            self.play_started_at = Some(now_ms);
            self.play_started_from = to_ms;
        }
        Ok(vec![Event::PositionChanged { ms: to_ms }])
    }

    fn cmd_tick(&mut self, now_ms: u64) -> Vec<Event> {
        // Aceite em qualquer estado e inócuo fora de `Playing`: o daemon tica em cadência
        // fixa e não deve ter de conhecer o estado para o fazer.
        if self.state != State::Playing {
            return Vec::new();
        }
        self.advance_position(now_ms);
        let mut events = vec![Event::PositionChanged { ms: self.position_ms }];

        let duration = self.show.expect("Playing exige show").duration_ms;
        if self.position_ms >= duration {
            self.position_ms = duration;
            self.play_started_at = None;
            events.push(self.transition(State::Finished));
            events.push(Event::ReachedEnd);
        }
        events
    }

    fn cmd_fault(&mut self, code: FaultCode, now_ms: u64) -> Result<Vec<Event>, Rejected> {
        if self.state == State::Idle {
            return Err(Rejected::NoShowLoaded);
        }
        // Preserva a posição onde a falha aconteceu — é o que o operador precisa para saber
        // em que ponto do show o problema surgiu.
        if self.state == State::Playing {
            self.advance_position(now_ms);
        }
        self.play_started_at = None;
        self.fault = Some(code);
        Ok(vec![self.transition(State::Error), Event::Faulted(code)])
    }

    fn cmd_clear_fault(&mut self) -> Result<Vec<Event>, Rejected> {
        if self.state != State::Error {
            return Err(Rejected::NotApplicable { state: self.state, command: "clear_fault" });
        }
        self.fault = None;
        // Volta a `Loaded`, nunca a `Ready`: depois de uma falha o pré-voo tem de correr
        // outra vez. Voltar direto a armado seria confiar num veredito anterior à falha.
        Ok(vec![self.transition(State::Loaded), Event::FaultCleared])
    }

    /// Avança `position_ms` pelo tempo decorrido desde o início do `Play` corrente,
    /// saturando na duração do show.
    fn advance_position(&mut self, now_ms: u64) {
        let (Some(started_at), Some(show)) = (self.play_started_at, self.show) else {
            return;
        };
        let elapsed = now_ms.saturating_sub(started_at);
        self.position_ms = self
            .play_started_from
            .saturating_add(elapsed)
            .min(show.duration_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show() -> ShowDescriptor {
        ShowDescriptor { id: ShowId(1), frame_count: 240, pixel_count: 720, duration_ms: 10_000 }
    }

    /// Leva a máquina até `Playing` num caminho conhecido.
    fn playing() -> ShowRuntime {
        let mut rt = ShowRuntime::new();
        rt.apply(Command::Load(show()), 0).unwrap();
        rt.apply(Command::Arm(PreflightReport::all_clear()), 0).unwrap();
        rt.apply(Command::Play, 1_000).unwrap();
        rt
    }

    #[test]
    fn ciclo_de_vida_completo() {
        let mut rt = ShowRuntime::new();
        assert_eq!(rt.state(), State::Idle);
        rt.apply(Command::Load(show()), 0).unwrap();
        assert_eq!(rt.state(), State::Loaded);
        rt.apply(Command::Arm(PreflightReport::all_clear()), 0).unwrap();
        assert_eq!(rt.state(), State::Ready);
        rt.apply(Command::Play, 100).unwrap();
        assert_eq!(rt.state(), State::Playing);
        rt.apply(Command::Pause, 1_100).unwrap();
        assert_eq!(rt.state(), State::Paused);
        assert_eq!(rt.position_ms(), 1_000);
        rt.apply(Command::Stop, 1_200).unwrap();
        assert_eq!(rt.state(), State::Stopped);
        assert_eq!(rt.position_ms(), 0);
        rt.apply(Command::Unload, 1_300).unwrap();
        assert_eq!(rt.state(), State::Idle);
    }

    #[test]
    fn preflight_reprovado_nao_arma_e_nao_muda_estado() {
        let mut rt = ShowRuntime::new();
        rt.apply(Command::Load(show()), 0).unwrap();
        let bad = PreflightReport { integrity_verified: true, network_ok: false, devices_present: true };
        let err = rt.apply(Command::Arm(bad), 0).unwrap_err();
        assert_eq!(err, Rejected::PreflightFailed(bad));
        assert_eq!(rt.state(), State::Loaded, "recusa NÃO pode mudar o estado");
        assert_eq!(bad.failures(), vec!["network"]);
    }

    #[test]
    fn tocar_sem_armar_e_recusado() {
        let mut rt = ShowRuntime::new();
        rt.apply(Command::Load(show()), 0).unwrap();
        assert_eq!(rt.apply(Command::Play, 0).unwrap_err(), Rejected::NotArmed);
        assert_eq!(rt.state(), State::Loaded);
    }

    #[test]
    fn pausa_preserva_posicao_e_stop_zera() {
        let mut rt = playing();
        rt.apply(Command::Tick, 4_000).unwrap();
        assert_eq!(rt.position_ms(), 3_000);
        rt.apply(Command::Pause, 4_000).unwrap();
        assert_eq!(rt.position_ms(), 3_000, "pausa preserva");
        rt.apply(Command::Play, 5_000).unwrap();
        rt.apply(Command::Tick, 6_000).unwrap();
        assert_eq!(rt.position_ms(), 4_000, "retoma de onde parou");
        rt.apply(Command::Stop, 6_000).unwrap();
        assert_eq!(rt.position_ms(), 0, "stop zera");
    }

    #[test]
    fn chega_ao_fim_e_finished() {
        let mut rt = playing();
        let events = rt.apply(Command::Tick, 999_999).unwrap();
        assert_eq!(rt.state(), State::Finished);
        assert_eq!(rt.position_ms(), 10_000, "satura na duração, nunca a excede");
        assert!(events.contains(&Event::ReachedEnd));
    }

    /// ADR-0023 §4 — rebobinar implicitamente faria um show recomeçar no palco.
    #[test]
    fn play_a_partir_de_finished_e_recusado() {
        let mut rt = playing();
        rt.apply(Command::Tick, 999_999).unwrap();
        assert_eq!(rt.state(), State::Finished);
        assert!(rt.apply(Command::Play, 999_999).is_err());
        assert_eq!(rt.state(), State::Finished);
        // O caminho documentado funciona.
        rt.apply(Command::Stop, 999_999).unwrap();
        rt.apply(Command::Play, 999_999).unwrap();
        assert_eq!(rt.state(), State::Playing);
    }

    #[test]
    fn seek_fora_de_alcance_e_recusado() {
        let mut rt = playing();
        let err = rt.apply(Command::Seek { to_ms: 10_001 }, 1_000).unwrap_err();
        assert_eq!(err, Rejected::SeekOutOfRange { to_ms: 10_001, duration_ms: 10_000 });
        assert_eq!(rt.position_ms(), 0, "recusa não move a posição");
        rt.apply(Command::Seek { to_ms: 10_000 }, 1_000).unwrap();
        assert_eq!(rt.position_ms(), 10_000, "a fronteira exata é válida");
    }

    #[test]
    fn seek_durante_playing_rebaseia_a_origem() {
        let mut rt = playing();
        rt.apply(Command::Seek { to_ms: 5_000 }, 2_000).unwrap();
        rt.apply(Command::Tick, 3_000).unwrap();
        assert_eq!(rt.position_ms(), 6_000, "1 s após o seek para 5 s");
    }

    #[test]
    fn erro_e_absorvente_ate_clear_fault() {
        let mut rt = playing();
        rt.apply(Command::Tick, 3_000).unwrap();
        rt.apply(Command::Fault(FaultCode::DeviceLost), 3_000).unwrap();
        assert_eq!(rt.state(), State::Error);
        assert_eq!(rt.position_ms(), 2_000, "a posição da falha é preservada");
        assert_eq!(
            rt.apply(Command::Play, 4_000).unwrap_err(),
            Rejected::InErrorState(FaultCode::DeviceLost)
        );
        rt.apply(Command::ClearFault, 4_000).unwrap();
        assert_eq!(rt.state(), State::Loaded, "volta a Loaded — o pré-voo tem de correr outra vez");
        assert_eq!(rt.fault(), None);
    }

    #[test]
    fn unload_durante_playing_e_recusado() {
        let mut rt = playing();
        assert!(rt.apply(Command::Unload, 2_000).is_err());
        assert_eq!(rt.state(), State::Playing);
    }

    /// Determinismo: a mesma sequência de `(comando, now_ms)` produz o mesmo estado.
    #[test]
    fn transicoes_sao_deterministicas() {
        let seq = [
            (Command::Load(show()), 0u64),
            (Command::Arm(PreflightReport::all_clear()), 10),
            (Command::Play, 20),
            (Command::Tick, 1_020),
            (Command::Seek { to_ms: 500 }, 1_020),
            (Command::Tick, 2_020),
            (Command::Pause, 2_020),
        ];
        let run = || {
            let mut rt = ShowRuntime::new();
            for (cmd, t) in seq {
                let _ = rt.apply(cmd, t);
            }
            (rt.state(), rt.position_ms(), rt.to_json())
        };
        assert_eq!(run(), run());
    }

    /// Relógio a andar para trás é clampado — não entra em pânico nem retrocede a posição.
    #[test]
    fn relogio_retrogrado_e_clampado() {
        let mut rt = playing();
        rt.apply(Command::Tick, 5_000).unwrap();
        let p = rt.position_ms();
        rt.apply(Command::Tick, 10).unwrap(); // salto para trás
        assert!(rt.position_ms() >= p, "a posição nunca retrocede por salto de relógio");
    }

    #[test]
    fn json_do_snapshot_tem_a_forma_esperada() {
        let rt = playing();
        let j = rt.to_json();
        assert!(j.contains("\"state\":\"playing\""), "{j}");
        assert!(j.contains("\"advancing\":true"), "{j}");
        assert!(j.contains("\"fault\":null"), "{j}");
        assert!(ShowRuntime::new().to_json().contains("\"state\":\"idle\""));
    }
}
