//! Journal: os eventos do runtime como **JSON delimitado por linha**.
//!
//! ## Por que a serialização vive AQUI e não no `led-daemon`
//!
//! O formato do fio é contrato do **IPC (GS3/ADR-0014)**, e o contrato da máquina foi
//! congelado na GS1.6. Pôr `to_json` no `led-daemon` agora congelaria também o formato —
//! antes de existir um cliente para o discutir. Fica no processo, que é quem escreve.
//!
//! **Consequência boa e verificável:** este crate **não altera uma linha** do `led-daemon`.
//!
//! O enquadramento é uma mensagem por linha, como o `control-protocol.md` já especifica, e o
//! JSON é escrito à mão — convenção do workspace (`ReadModel::to_json`,
//! `MetricsEmitter::snapshot_json`), sem `serde`.

use led_daemon::{Event, PositionCause, State};
use std::io::Write;

/// Escapa uma string para JSON. Só o necessário: os valores aqui são identificadores
/// controlados por nós, não texto arbitrário do utilizador.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn cause_str(c: PositionCause) -> &'static str {
    c.as_str()
}

/// Um evento do runtime como uma linha de JSON, carimbada com o instante do daemon.
pub fn event_to_json(t_ms: u64, e: &Event) -> String {
    match e {
        Event::Transitioned { from, to } => format!(
            r#"{{"t_ms":{t_ms},"event":"transitioned","from":"{}","to":"{}"}}"#,
            from.as_str(),
            to.as_str()
        ),
        Event::ShowLoaded(id) => {
            format!(r#"{{"t_ms":{t_ms},"event":"show_loaded","show_id":{}}}"#, id.0)
        }
        Event::ShowUnloaded(id) => {
            format!(r#"{{"t_ms":{t_ms},"event":"show_unloaded","show_id":{}}}"#, id.0)
        }
        Event::PositionChanged { ms, cause } => format!(
            r#"{{"t_ms":{t_ms},"event":"position_changed","ms":{ms},"cause":"{}"}}"#,
            cause_str(*cause)
        ),
        Event::ReachedEnd => format!(r#"{{"t_ms":{t_ms},"event":"reached_end"}}"#),
        Event::Faulted(c) => {
            format!(r#"{{"t_ms":{t_ms},"event":"faulted","code":"{}"}}"#, c.as_str())
        }
        Event::FaultCleared => format!(r#"{{"t_ms":{t_ms},"event":"fault_cleared"}}"#),
    }
}

/// Linha de ciclo de vida do próprio daemon (não é evento do runtime).
pub fn notice_to_json(t_ms: u64, kind: &str, detail: &str) -> String {
    format!(
        r#"{{"t_ms":{t_ms},"notice":"{}","detail":"{}"}}"#,
        esc(kind),
        esc(detail)
    )
}

/// Estado corrente, para o arranque e o encerramento.
pub fn state_to_json(t_ms: u64, state: State, position_ms: u64) -> String {
    format!(
        r#"{{"t_ms":{t_ms},"notice":"state","state":"{}","position_ms":{position_ms}}}"#,
        state.as_str()
    )
}

/// Destino do journal: **sempre** stdout, e opcionalmente um ficheiro em modo *append*.
///
/// Persistir num ficheiro sem deixar de escrever em stdout é deliberado: sob um supervisor
/// (`launchd`, `systemd`) o stdout já é capturado, e duplicar é mais barato que perder.
pub struct Journal<W: Write> {
    out: W,
    file: Option<std::fs::File>,
}

impl<W: Write> Journal<W> {
    pub fn new(out: W) -> Self {
        Self { out, file: None }
    }

    /// Acrescenta um ficheiro de log (aberto em *append*; criado se não existir).
    pub fn with_file(mut self, path: &str) -> std::io::Result<Self> {
        self.file = Some(std::fs::OpenOptions::new().create(true).append(true).open(path)?);
        Ok(self)
    }

    /// Escreve uma linha nos dois destinos.
    ///
    /// **Uma falha ao escrever no ficheiro não derruba o daemon** — é reportada em stdout e o
    /// laço continua. Um disco cheio não pode parar um show; a mesma disciplina do
    /// `control-protocol.md` §"Degradação segura" (canal caído ⇒ o show continua).
    pub fn line(&mut self, s: &str) {
        let _ = writeln!(self.out, "{s}");
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{s}").is_err() {
                let _ = writeln!(self.out, "{}", notice_to_json(0, "log_write_failed", "journal file"));
                self.file = None; // não tenta outra vez a cada linha
            }
        }
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_daemon::{FaultCode, ShowId};

    #[test]
    fn cada_evento_tem_uma_linha_json_distinta() {
        let casos = vec![
            Event::Transitioned { from: State::Ready, to: State::Playing },
            Event::ShowLoaded(ShowId(7)),
            Event::ShowUnloaded(ShowId(7)),
            Event::PositionChanged { ms: 250, cause: PositionCause::Advanced },
            Event::PositionChanged { ms: 900, cause: PositionCause::Sought },
            Event::PositionChanged { ms: 0, cause: PositionCause::Reset },
            Event::ReachedEnd,
            Event::Faulted(FaultCode::DeviceLost),
            Event::FaultCleared,
        ];
        let linhas: Vec<String> = casos.iter().map(|e| event_to_json(42, e)).collect();

        for l in &linhas {
            assert!(l.starts_with('{') && l.ends_with('}'), "não é objeto JSON: {l}");
            assert!(!l.contains('\n'), "uma mensagem por LINHA: {l}");
            assert!(l.contains(r#""t_ms":42"#), "falta o carimbo: {l}");
        }

        // **As três causas produzem linhas diferentes** — se `cause` não fosse serializada,
        // este teste passaria a ver duplicados e falha. É o gate que prova que F2 chega ao fio.
        let mut ord = linhas.clone();
        ord.sort();
        ord.dedup();
        assert_eq!(ord.len(), linhas.len(), "duas variantes serializaram igual: {linhas:?}");
    }

    #[test]
    fn journal_escreve_no_buffer() {
        let mut buf = Vec::new();
        {
            let mut j = Journal::new(&mut buf);
            j.line(&notice_to_json(0, "started", "no-output"));
            j.line(&event_to_json(1, &Event::ReachedEnd));
            j.flush();
        }
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains(r#""notice":"started""#));
        assert!(s.contains(r#""event":"reached_end""#));
    }

    #[test]
    fn escape_nao_quebra_o_json() {
        let l = notice_to_json(0, "x", r#"caminho "com" aspas \ e barra"#);
        assert!(!l.contains(r#""com""#), "aspas cruas quebrariam o objeto: {l}");
        assert_eq!(l.matches('{').count(), 1);
    }
}
