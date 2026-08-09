//! Servidor de controlo sobre **Unix Domain Socket**, owner-only (ADR-0014).
//!
//! ## O desenho que importa: **um só aplicador**
//!
//! As threads de ligação **nunca tocam** o `ShowRuntime`. Elas analisam, validam e
//! **enfileiram**; o laço principal aplica no **limite do tick** e responde. É o que o
//! `control-protocol.md` §"Isolamento do hot-path" exige, e tem duas consequências que valem
//! mais que a conformidade:
//!
//! - o runtime continua com **um único dono**, então o determinismo do ADR-0023 sobrevive à
//!   chegada da concorrência;
//! - `status` não passa pela fila — lê um **snapshot** publicado pelo laço. Consultar nunca
//!   compete com comandar.
//!
//! ## Segurança
//!
//! Socket em `0o600` (só o dono lê/escreve), aplicado **depois** do bind e verificado por
//! teste. Não há TCP: `0.0.0.0` não é sequer representável aqui, que é a forma mais forte de
//! cumprir a regra do ADR-0014.

#![cfg(unix)]

use crate::proto::{code, err_line, event_line, jstr, ok_line, Cmd, ProtoError, Request};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Tamanho máximo de uma linha de pedido. Sem isto, um cliente que nunca envie `\n` faz o
/// daemon crescer sem limite — negação de serviço com um byte por segundo.
pub const MAX_LINE: usize = 64 * 1024;

/// Quanto tempo uma ligação espera pela resposta do laço antes de desistir.
///
/// **Público desde o ADR-0026** para que o `led-console-bin` possa *derivar* dele o seu
/// timeout HTTP em vez de escrever um segundo número. Se os dois empatassem, o browser
/// receberia "falhou" enquanto o daemon ainda aplicaria o comando.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Snapshot que o laço publica e o `status` lê. Consultar não compete com comandar.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub state: String,
    pub position_ms: u64,
    pub show_id: Option<u64>,
    pub duration_ms: u64,
    pub ticks: u64,
}

/// Um comando à espera de ser aplicado pelo laço principal.
pub struct Job {
    pub cmd: Cmd,
    pub reply: mpsc::Sender<Result<Vec<(&'static str, String)>, ProtoError>>,
}

/// O que o servidor e o laço partilham.
pub struct ControlPlane {
    jobs_tx: mpsc::Sender<Job>,
    jobs_rx: Mutex<mpsc::Receiver<Job>>,
    pub snapshot: Mutex<Snapshot>,
    subs: Mutex<Vec<mpsc::Sender<String>>>,
    pub shutdown: Arc<AtomicBool>,
    pending_token: Mutex<Option<String>>,
    token_seq: AtomicU64,
}

impl ControlPlane {
    pub fn new(shutdown: Arc<AtomicBool>) -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        Arc::new(Self {
            jobs_tx: tx,
            jobs_rx: Mutex::new(rx),
            snapshot: Mutex::new(Snapshot::default()),
            subs: Mutex::new(Vec::new()),
            shutdown,
            pending_token: Mutex::new(None),
            token_seq: AtomicU64::new(0),
        })
    }

    /// Tira os trabalhos pendentes. Chamado pelo laço, **entre** ticks.
    pub fn drain_jobs(&self) -> Vec<Job> {
        let rx = self.jobs_rx.lock().expect("jobs_rx");
        rx.try_iter().collect()
    }

    /// Publica um evento para quem fez `subscribe`. Ligações mortas são **podadas** — sem
    /// isto, um cliente que morre deixa um `Sender` a acumular para sempre.
    pub fn broadcast(&self, payload: &str) {
        let linha = event_line(payload);
        let mut subs = self.subs.lock().expect("subs");
        subs.retain(|s| s.send(linha.clone()).is_ok());
    }

    pub fn subscribers(&self) -> usize {
        self.subs.lock().expect("subs").len()
    }

    fn new_token(&self) -> String {
        let n = self.token_seq.fetch_add(1, Ordering::Relaxed);
        // Token de uso único e curta validade. Não é segredo criptográfico: o socket já é
        // owner-only, e a confirmação existe contra o ENGANO, não contra um atacante que já
        // tem a credencial do dono.
        let t = format!("cfm-{n}-{:x}", std::process::id());
        *self.pending_token.lock().expect("token") = Some(t.clone());
        t
    }

    fn take_token(&self, given: &str) -> bool {
        let mut g = self.pending_token.lock().expect("token");
        match g.as_deref() {
            Some(t) if t == given => {
                *g = None; // uso único
                true
            }
            _ => false,
        }
    }
}

/// O servidor. `Drop` remove o ficheiro do socket.
pub struct Server {
    listener: UnixListener,
    path: std::path::PathBuf,
}

impl Server {
    /// Cria o socket em `path` com permissões **owner-only**.
    ///
    /// Um socket órfão de uma execução anterior é removido — sem isso, o daemon recusaria
    /// arrancar depois de qualquer paragem abrupta (e, sem tratamento de sinais, paragem
    /// abrupta é o caso comum).
    pub fn bind(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Aceita ligações numa thread própria. Cada ligação ganha a sua.
    pub fn spawn(self, cp: Arc<ControlPlane>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for stream in self.listener.incoming() {
                match stream {
                    Ok(s) => {
                        let cp = Arc::clone(&cp);
                        std::thread::spawn(move || {
                            // Uma ligação problemática não pode derrubar o daemon.
                            let _ = handle_connection(s, cp);
                        });
                    }
                    Err(_) => break, // listener fechado
                }
            }
            let _ = std::fs::remove_file(&self.path);
        })
    }
}

/// Trata uma ligação até ao EOF.
fn handle_connection(stream: UnixStream, cp: Arc<ControlPlane>) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    let mut hello_done = false;

    for linha in reader.lines() {
        let linha = match linha {
            Ok(l) => l,
            Err(_) => break,
        };
        if linha.len() > MAX_LINE {
            let e = ProtoError::new(code::BAD_REQUEST, "linha demasiado longa");
            writeln!(writer, "{}", err_line(None, &e))?;
            continue;
        }
        if linha.trim().is_empty() {
            continue;
        }

        let req = match Request::from_line(&linha) {
            Ok(r) => r,
            Err(e) => {
                writeln!(writer, "{}", err_line(e.id, &e.err))?;
                continue;
            }
        };

        // Handshake obrigatório: nada é aceite antes dele (control-protocol.md).
        if !hello_done && !matches!(req.cmd, Cmd::Hello { .. }) {
            let e = ProtoError::new(code::UNAUTHENTICATED, "envie `hello` primeiro");
            writeln!(writer, "{}", err_line(Some(req.id), &e))?;
            continue;
        }

        let resposta = match &req.cmd {
            Cmd::Hello { client } => {
                hello_done = true;
                Ok(vec![
                    ("engine", jstr(concat!("lumyx-daemon/", env!("CARGO_PKG_VERSION")))),
                    ("accepts", "[1]".to_string()),
                    ("client", jstr(client)),
                ])
            }
            Cmd::Ping => Ok(vec![("pong", "true".into())]),
            Cmd::Version => Ok(vec![
                ("protocol", crate::proto::PROTOCOL_V.to_string()),
                ("engine", jstr(env!("CARGO_PKG_VERSION"))),
            ]),
            Cmd::Status => {
                let s = cp.snapshot.lock().expect("snapshot").clone();
                Ok(vec![
                    ("state", jstr(&s.state)),
                    ("position_ms", s.position_ms.to_string()),
                    ("duration_ms", s.duration_ms.to_string()),
                    ("ticks", s.ticks.to_string()),
                    (
                        "show_id",
                        s.show_id.map(|i| i.to_string()).unwrap_or_else(|| "null".into()),
                    ),
                ])
            }
            Cmd::Subscribe => {
                let (tx, rx) = mpsc::channel::<String>();
                cp.subs.lock().expect("subs").push(tx);
                // Escritor próprio: esta ligação passa a ter dois sentidos, e o leitor não
                // pode ficar bloqueado à espera de eventos para continuar a aceitar comandos.
                let mut w = writer.try_clone()?;
                std::thread::spawn(move || {
                    for ev in rx {
                        if writeln!(w, "{ev}").is_err() {
                            break;
                        }
                    }
                });
                Ok(vec![("subscribed", "true".into())])
            }
            Cmd::Shutdown { confirm } => match confirm {
                Some(t) if cp.take_token(t) => {
                    cp.shutdown.store(true, Ordering::Relaxed);
                    Ok(vec![("shutting_down", "true".into())])
                }
                Some(_) => Err(ProtoError::new(
                    code::CONFIRMATION_REQUIRED,
                    "token inválido ou já usado; peça outro",
                )),
                None => {
                    let t = cp.new_token();
                    Err(ProtoError::new(
                        code::CONFIRMATION_REQUIRED,
                        format!("repita com \"confirm\":\"{t}\""),
                    ))
                }
            },
            outro if outro.touches_runtime() => {
                // **Enfileira** — não aplica. O laço é o único aplicador.
                let (tx, rx) = mpsc::channel();
                match cp.jobs_tx.send(Job { cmd: outro.clone(), reply: tx }) {
                    Ok(()) => rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
                        Err(ProtoError::new(code::ENGINE_BUSY, "o laço não respondeu a tempo"))
                    }),
                    Err(_) => Err(ProtoError::new(code::ENGINE_BUSY, "o daemon está a encerrar")),
                }
            }
            outro => Err(ProtoError::new(code::UNKNOWN_COMMAND, outro.name())),
        };

        match resposta {
            Ok(extra) => writeln!(writer, "{}", ok_line(req.id, &extra))?,
            Err(e) => writeln!(writer, "{}", err_line(Some(req.id), &e))?,
        }
        writer.flush()?;
    }
    Ok(())
}

/// Mapeia o resultado do runtime para a resposta do protocolo. Vive aqui para que `run.rs`
/// não precise de conhecer códigos de protocolo.
pub fn runtime_result_to_reply(
    r: Result<Vec<led_daemon::Event>, led_daemon::Rejected>,
    state: led_daemon::State,
    position_ms: u64,
) -> Result<Vec<(&'static str, String)>, ProtoError> {
    match r {
        Ok(evs) => Ok(vec![
            ("state", jstr(state.as_str())),
            ("position_ms", position_ms.to_string()),
            ("events", evs.len().to_string()),
        ]),
        // O código de recusa do ADR-0023 vai **inalterado** para o fio. Foi para isto que o
        // contrato foi congelado na GS1.6: `no_show_loaded` significa o mesmo dos dois lados.
        Err(rej) => Err(ProtoError::new(
            leak(rej.code()),
            format!("{rej:?}"),
        )),
    }
}

/// `Rejected::code()` já devolve `&'static str`; este passo existe só para o tornar explícito.
fn leak(s: &'static str) -> &'static str {
    s
}

/// Erros de política do daemon (pré-voo, carregamento) → códigos do protocolo.
pub fn policy_error(detail: impl Into<String>) -> ProtoError {
    ProtoError::new(code::REFUSED_BY_POLICY, detail)
}

pub fn load_error(detail: impl Into<String>) -> ProtoError {
    ProtoError::new(code::LOAD_FAILED, detail)
}

/// Clientes: HashMap reservado para GS4 (identidade por ligação). Declarado para que o tipo
/// exista quando for preciso, sem lógica a sustentar hoje.
pub type ClientRegistry = HashMap<u64, String>;
