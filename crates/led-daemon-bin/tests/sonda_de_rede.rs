//! TD-029 — sonda de rede FALHADA numa plataforma suportada, através do laço real.
//!
//! O `preflight.rs` prova a decisão como função pura. Este ficheiro prova que ela chega ao
//! **journal** pelos dois caminhos que correm o pré-voo, e que nenhum o perde:
//!
//! - o arranque (show inicial + autoplay → `preflight_e_registar`);
//! - cada `load` por IPC (`apply_ipc`), que até ao TD-029 deitava fora **todas** as notices
//!   do pré-voo — incluindo o `network_unverified` que dizia ao operador que não se verificou.
//!
//! As sondas são **injetadas** (`run_with_control_com`): uma guarda que devolve `ProbeFailed`
//! e uma presença que responde. Sem isso, provar «a sonda falhou» exigiria partir o `/sys` ou
//! o `networksetup` da máquina do teste. O alvo é 192.0.2.10 (TEST-NET-1, RFC 5737): não é
//! loopback, por isso a guarda é mesmo consultada, e nunca pertence a um rig real.
#![cfg(unix)]

use led_core::PixelColor;
use led_daemon::ShowRuntime;
use led_daemon_bin::preflight::{DevicePresence, Presence};
use led_daemon_bin::run::run_with_control_com;
use led_daemon_bin::{descriptor_from_path, Config, ControlPlane, Integrity, Journal, Server, SystemPacer};
use led_hal::{NetworkGuard, NetworkPolicyError};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::io::{BufRead, BufReader, Write};
use std::net::IpAddr;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ALVO: &str = "192.0.2.10";
const PRESET: &str = "esp32-poe-wled-ddp";

struct SondaFalhada;
impl NetworkGuard for SondaFalhada {
    fn check(&self) -> Result<(), NetworkPolicyError> {
        Err(NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "read_dir: permission denied".into(),
        })
    }
    fn name(&self) -> &'static str {
        "sonda-falhada"
    }
}

struct Presente;
impl DevicePresence for Presente {
    fn probe(&self, _: IpAddr) -> Presence {
        Presence::AllPresent
    }
    fn name(&self) -> &'static str {
        "presente"
    }
}

/// Um `Write` partilhado: o laço corre noutra thread e o teste lê o journal no fim.
#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Buf {
    fn texto(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn escrever(nome: &str) -> String {
    let path = std::env::temp_dir().join(format!("{}-{nome}", std::process::id()));
    let mut w = ShowWriter::new(std::fs::File::create(&path).unwrap(), 4).unwrap();
    for i in 0..4u64 {
        w.write_frame(&ShowRecord {
            timestamp_ms: i * 25,
            pixels: vec![PixelColor { r: 1, g: 2, b: 3 }; 4],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

fn cfg(assume_no_wifi: bool, autoplay: bool) -> Config {
    Config {
        tick_ms: 25,
        max_ticks: None,
        autoplay,
        exit_on_finish: false,
        integrity: Integrity::AssumedByOperator,
        output: vec![ALVO.to_string()],
        profile: Some(PRESET.to_string()),
        assume_no_wifi,
    }
}

fn contar(journal: &str, notice: &str) -> usize {
    journal.matches(&format!(r#""notice":"{notice}""#)).count()
}

/// Sobe o laço real com as sondas injetadas e um servidor UDS; devolve o que é preciso para
/// falar com ele e para o parar.
struct Daemon {
    sock: std::path::PathBuf,
    flag: Arc<AtomicBool>,
    journal: Buf,
    laco: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    fn subir(nome: &str, c: Config, inicial: Option<String>) -> Self {
        let sock = std::env::temp_dir().join(format!("lumyx-td029-{nome}-{}.sock", std::process::id()));
        let flag = Arc::new(AtomicBool::new(false));
        let cp = ControlPlane::new(Arc::clone(&flag));
        Server::bind(&sock).expect("bind").spawn(Arc::clone(&cp));
        let journal = Buf::default();
        let (j, f) = (journal.clone(), Arc::clone(&flag));
        let laco = std::thread::spawn(move || {
            let mut rt = ShowRuntime::new();
            let inicial = inicial.map(|p| {
                let d = descriptor_from_path(&p, led_daemon::ShowId(1)).expect("show");
                (p, d)
            });
            let mut pacer = SystemPacer::new();
            let mut jn = Journal::new(j);
            run_with_control_com(&mut rt, inicial, &c, &mut pacer, &mut jn, &f, &cp, &SondaFalhada, &Presente);
        });
        Daemon { sock, flag, journal, laco: Some(laco) }
    }

    fn cliente(&self) -> (UnixStream, BufReader<UnixStream>) {
        let fim = Instant::now() + Duration::from_secs(5);
        let s = loop {
            match UnixStream::connect(&self.sock) {
                Ok(s) => break s,
                Err(_) if Instant::now() < fim => std::thread::yield_now(),
                Err(e) => panic!("socket {}: {e}", self.sock.display()),
            }
        };
        let r = BufReader::new(s.try_clone().unwrap());
        (s, r)
    }

    fn parar(mut self) -> String {
        self.flag.store(true, Ordering::SeqCst);
        self.laco.take().unwrap().join().unwrap();
        self.journal.texto()
    }
}

fn pedir(s: &mut UnixStream, r: &mut BufReader<UnixStream>, linha: &str) -> String {
    writeln!(s, "{linha}").unwrap();
    s.flush().unwrap();
    // Eventos assíncronos não têm `id`: salta-os até chegar a resposta.
    loop {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        if l.contains(r#""id":"#) {
            return l.trim().to_string();
        }
    }
}

fn load(path: &str, id: u32) -> String {
    format!(r#"{{"v":1,"id":{id},"cmd":"load","args":{{"path":"{path}","assume_integrity":true}}}}"#)
}

/// **D2(a) + D4 — cada `load` por IPC com o override deixa o seu próprio evento.** Dois
/// `load` seguidos ⇒ dois `network_assumed_by_operator` no journal, cada um com a causa. Se
/// o caminho IPC voltar a deitar fora as notices, este teste fica vermelho.
#[test]
fn dois_loads_ipc_com_override_deixam_dois_eventos_no_journal() {
    let show = escrever("td029-ipc.lumyx");
    let d = Daemon::subir("ipc", cfg(true, false), None);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let r1 = pedir(&mut s, &mut r, &load(&show, 2));
    assert!(r1.contains(r#""ok":true"#), "com --assume-no-wifi o load arma: {r1}");
    let u = pedir(&mut s, &mut r, r#"{"v":1,"id":3,"cmd":"unload"}"#);
    assert!(u.contains(r#""ok":true"#), "unload: {u}");
    let r2 = pedir(&mut s, &mut r, &load(&show, 4));
    assert!(r2.contains(r#""ok":true"#), "segundo load: {r2}");
    let j = d.parar();

    assert_eq!(
        contar(&j, "network_assumed_by_operator"),
        2,
        "um evento POR pré-voo que usa o override, não um por processo.\n  journal:\n{j}"
    );
    for linha in j.lines().filter(|l| l.contains("network_assumed_by_operator")) {
        assert!(
            linha.contains("sysfs /sys/class/net") && linha.contains("permission denied"),
            "cada evento tem de registar a causa da falha da sonda: {linha}"
        );
    }
}

/// **D4 — sem a flag, o `load` por IPC é recusado E a razão chega ao journal.** Antes do
/// TD-029 a recusa chegava ao cliente (`preflight_failed`) mas o journal ficava sem a causa.
#[test]
fn sem_override_o_load_ipc_e_recusado_e_a_razao_fica_no_journal() {
    let show = escrever("td029-ipc-sem.lumyx");
    let d = Daemon::subir("ipc-sem", cfg(false, false), None);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 2));
    let j = d.parar();

    assert!(resp.contains("preflight_failed"), "sonda falhada sem override bloqueia: {resp}");
    assert_eq!(contar(&j, "network_probe_failed"), 1, "a razão tem de ficar no journal:\n{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0);
}

/// **O caminho do arranque (show inicial + autoplay) regista o mesmo evento.**
#[test]
fn arranque_com_override_regista_o_evento() {
    let show = escrever("td029-arranque.lumyx");
    let d = Daemon::subir("arranque", cfg(true, true), Some(show));
    // Espera causal: o evento do pré-voo do arranque é escrito antes do primeiro tick.
    let fim = Instant::now() + Duration::from_secs(5);
    while contar(&d.journal.texto(), "network_assumed_by_operator") == 0 && Instant::now() < fim {
        std::thread::yield_now();
    }
    let j = d.parar();
    assert_eq!(contar(&j, "network_assumed_by_operator"), 1, "journal:\n{j}");
}
