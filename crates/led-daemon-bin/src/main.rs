//! Ponto de entrada do daemon. Só CLI e cablagem — a lógica está na biblioteca, para que o
//! laço seja testável sem processo, sem relógio e sem sinais.

use led_daemon::{ShowId, ShowRuntime};
use led_daemon_bin::{
    descriptor_from_path, run, Config, ExitReason, Integrity, Journal, SystemPacer,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const USAGE: &str = "\
led-daemon — processo de transporte do LUMYX (GS2)

USO:
    led-daemon <SHOW.lumyx> [OPÇÕES]

OPÇÕES:
    --tick-ms N           Período do tick em ms (padrão: 20)
    --max-ticks N         Encerra após N ticks (padrão: corre até pedirem)
    --log CAMINHO         Acrescenta o journal a um ficheiro (além do stdout)
    --assume-integrity    O operador AFIRMA que o artefato está íntegro.
                          NÃO é verificação — nenhum hash é recomputado, e o
                          journal regista que foi afirmado. Sem isto o pré-voo
                          reprova e o daemon não toca.
    --output SPEC         Saída Ethernet: ddp://IP[:4048], artnet://IP[:6454]
                          ou sacn://IP[:5568]. SEM esta opção nenhum frame
                          deixa o processo, e o pré-voo de rede/dispositivos
                          é VACUOSO (fica dito no journal).
    --socket CAMINHO      Abre o socket de controlo (UDS, owner-only 0600).
                          Com --socket, o <SHOW.lumyx> é OPCIONAL: o show pode
                          chegar por `load` do ledctl.
    --no-autoplay         Carrega mas não arma nem toca
    --keep-running        Não encerra ao chegar ao fim do show
    -h, --help            Esta ajuda

ENCERRAMENTO:
    Escreva `shutdown` no stdin, ou use --max-ticks.
    NÃO há tratamento de SIGINT/SIGTERM: exigiria uma dependência de sinais, e o
    `shutdown` por IPC é entrega do GS3. Ctrl-C termina o processo, mas de forma
    ABRUPTA — sem a linha final de estado nem o flush do journal.

PRÉ-VOO:
    COM --output o pré-voo é REAL: WifiBlockGuard (ADR-0005, WiFi proibido ao
    vivo) e descoberta ArtPoll dos controladores. Uma sonda que não consegue
    medir deixa prosseguir COM AVISO — e o journal nunca diz `verificado`
    quando não verificou.

HEARTBEAT:
    Pause/Stop/Finished NÃO apagam o palco: o último frame válido é reenviado
    a cada 800 ms. Nenhum caminho deste processo envia zeros.
";

#[derive(Debug)]
struct Args {
    show: Option<String>,
    socket: Option<String>,
    cfg: Config,
    log: Option<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut show: Option<String> = None;
    let mut socket: Option<String> = None;
    let mut cfg = Config { integrity: Integrity::NotVerified, ..Config::default() };
    let mut log = None;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let mut valor = |nome: &str| -> Result<String, String> {
            i += 1;
            argv.get(i).cloned().ok_or_else(|| format!("{nome} exige um valor"))
        };
        match a {
            "-h" | "--help" => return Err(String::new()),
            "--tick-ms" => {
                cfg.tick_ms = valor("--tick-ms")?.parse().map_err(|_| "--tick-ms inválido")?
            }
            "--max-ticks" => {
                cfg.max_ticks =
                    Some(valor("--max-ticks")?.parse().map_err(|_| "--max-ticks inválido")?)
            }
            "--log" => log = Some(valor("--log")?),
            "--output" => cfg.output = Some(valor("--output")?),
            "--socket" => socket = Some(valor("--socket")?),
            "--assume-integrity" => cfg.integrity = Integrity::AssumedByOperator,
            "--no-autoplay" => cfg.autoplay = false,
            "--keep-running" => cfg.exit_on_finish = false,
            outro if outro.starts_with('-') => return Err(format!("opção desconhecida: {outro}")),
            caminho => {
                if show.is_some() {
                    return Err("apenas um ficheiro de show".into());
                }
                show = Some(caminho.to_string());
            }
        }
        i += 1;
    }

    if show.is_none() && socket.is_none() {
        return Err("falta o ficheiro <SHOW.lumyx> (ou use --socket para carregar por IPC)".into());
    }
    Ok(Args { show, socket, cfg, log })
}

/// Lê o stdin numa thread própria e sinaliza o encerramento na linha `shutdown`.
///
/// **EOF NÃO encerra**, de propósito: um daemon supervisionado corre frequentemente com o
/// stdin fechado, e fazer o EOF encerrar mataria o processo à nascença quando o stdin fosse
/// `/dev/null`.
fn spawn_stdin_shutdown(flag: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for linha in stdin.lock().lines() {
            match linha {
                Ok(l) if l.trim().eq_ignore_ascii_case("shutdown") => {
                    flag.store(true, Ordering::Relaxed);
                    return;
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
        // EOF: a thread termina; o daemon continua.
    });
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(msg) => {
            if msg.is_empty() {
                print!("{USAGE}");
                std::process::exit(0);
            }
            eprintln!("erro: {msg}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let desc = match &args.show {
        Some(p) => match descriptor_from_path(p, ShowId(1)) {
            Ok(d) => Some((p.clone(), d)),
            Err(e) => {
                eprintln!("erro ao carregar {p}: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let stdout = std::io::stdout();
    let mut journal = Journal::new(stdout.lock());
    if let Some(path) = &args.log {
        match Journal::new(std::io::stdout()).with_file(path) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("erro: não consigo abrir o log {path}: {e}");
                std::process::exit(1);
            }
        }
        // Reabre já com o ficheiro ligado (a verificação acima falha cedo com erro claro).
        journal = Journal::new(stdout.lock()).with_file(path).expect("verificado acima");
    }

    let flag = Arc::new(AtomicBool::new(false));
    spawn_stdin_shutdown(Arc::clone(&flag));

    let mut rt = ShowRuntime::new();
    let mut pacer = SystemPacer::new();

    let outcome = match &args.socket {
        Some(path) => {
            let cp = led_daemon_bin::ControlPlane::new(Arc::clone(&flag));
            let srv = match led_daemon_bin::Server::bind(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("erro: não consigo abrir o socket {path}: {e}");
                    std::process::exit(1);
                }
            };
            srv.spawn(Arc::clone(&cp));
            led_daemon_bin::run::run_with_control(
                &mut rt, desc, &args.cfg, &mut pacer, &mut journal, &flag, &cp,
            )
        }
        None => {
            let (path, d) = desc.expect("sem --socket o show é obrigatório (verificado no parse)");
            run(
            &mut rt,
            &path,
            d,
            &args.cfg,
            &mut pacer,
            &mut journal,
            &flag,
        )
        }
    };

    std::process::exit(match outcome.reason {
        ExitReason::NeverStarted => 1,
        _ => 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parse_minimo() {
        let a = args(&["show.lumyx"]).unwrap();
        assert_eq!(a.show.as_deref(), Some("show.lumyx"));
        assert_eq!(a.cfg.tick_ms, 20);
        assert_eq!(a.cfg.integrity, Integrity::NotVerified, "integridade NÃO é o padrão");
        assert_eq!(a.cfg.output, None, "sem --output o daemon continua sem saída");
    }

    #[test]
    fn parse_opcoes() {
        let a = args(&[
            "s.lumyx",
            "--tick-ms",
            "40",
            "--max-ticks",
            "10",
            "--log",
            "/tmp/j.jsonl",
            "--output",
            "ddp://192.168.2.156",
            "--assume-integrity",
            "--no-autoplay",
            "--keep-running",
        ])
        .unwrap();
        assert_eq!(a.cfg.tick_ms, 40);
        assert_eq!(a.cfg.max_ticks, Some(10));
        assert_eq!(a.log.as_deref(), Some("/tmp/j.jsonl"));
        assert_eq!(a.cfg.output.as_deref(), Some("ddp://192.168.2.156"));
        assert_eq!(a.cfg.integrity, Integrity::AssumedByOperator);
        assert!(!a.cfg.autoplay);
        assert!(!a.cfg.exit_on_finish);
    }

    #[test]
    fn erros_de_cli_sao_explicitos() {
        assert!(args(&[]).is_err(), "sem ficheiro nem socket tem de falhar");
        assert!(args(&["--socket", "/tmp/s.sock"]).is_ok(), "com socket o show é opcional");
        assert!(args(&["a.lumyx", "b.lumyx"]).is_err(), "dois ficheiros");
        assert!(args(&["a.lumyx", "--tick-ms"]).is_err(), "opção sem valor");
        assert!(args(&["a.lumyx", "--tick-ms", "abc"]).is_err(), "valor não numérico");
        assert!(args(&["a.lumyx", "--nao-existe"]).is_err(), "opção desconhecida");
    }

    #[test]
    fn ajuda_e_sinalizada_por_erro_vazio() {
        assert_eq!(args(&["--help"]).unwrap_err(), "");
        assert_eq!(args(&["-h"]).unwrap_err(), "");
    }
}
