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

struct WifiAtivo;
impl NetworkGuard for WifiAtivo {
    fn check(&self) -> Result<(), NetworkPolicyError> {
        Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] })
    }
    fn name(&self) -> &'static str {
        "wifi-ativo"
    }
}

static FALHADA: SondaFalhada = SondaFalhada;
static WIFI: WifiAtivo = WifiAtivo;

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
        Self::subir_com(nome, c, inicial, &FALHADA)
    }

    fn subir_com(
        nome: &str,
        c: Config,
        inicial: Option<String>,
        guarda: &'static (dyn NetworkGuard + Sync),
    ) -> Self {
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
            run_with_control_com(&mut rt, inicial, &c, &mut pacer, &mut jn, &f, &cp, guarda, &Presente);
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
    // Controlo positivo do oráculo `armou_ou_tocou`: com a flag o mesmo arranque arma e toca.
    assert!(armou_ou_tocou(&j), "com a flag e a sonda falhada o arranque toca:\n{j}");
}

/// O arranque (pré-voo + Arm + Play) corre inteiro ANTES do laço, que é onde o `parar` é visto;
/// o encerramento escreve SEMPRE a linha final `"notice":"state"`. Por isso, depois do `parar`,
/// o estado final diz se o arranque armou — sem esperar por relógio. Dois oráculos que NÃO servem:
/// o `status` (antes do 1.º tick publicado responde o instantâneo por omissão, `idle`) e os eventos
/// `transitioned` (neste caminho o Arm/Play do arranque não os escreve no journal).
fn armou_ou_tocou(j: &str) -> bool {
    let fim = j.lines().rev().find(|l| l.contains(r#""notice":"state""#)).expect("linha final de estado");
    !fim.contains(r#""state":"loaded""#)
}

/// **O arranque SEM a flag e com a sonda falhada não toca.** O par de
/// `arranque_com_override_regista_o_evento`: sem ele, ligar o override sempre no arranque
/// deixava a suite verde (falsificador R4.T, MA1) — e o show tocava sem a sonda verificar nada.
#[test]
fn arranque_sem_override_com_sonda_falhada_nao_toca() {
    let show = escrever("td029-arranque-sem.lumyx");
    let d = Daemon::subir("arranque-sem", cfg(false, true), Some(show));
    let j = d.parar();
    assert!(j.contains(r#""to":"loaded""#), "premissa: o show carregou:\n{j}");
    assert!(!armou_ou_tocou(&j), "sem a flag o arranque não arma nem toca:\n{j}");
    assert_eq!(contar(&j, "network_probe_failed"), 1, "a razão tem de ficar no journal:\n{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0, "sem a flag nunca há override:\n{j}");
}

/// **WiFi ativo bloqueia SEMPRE, com a flag, nos dois caminhos** (arranque e `load` por IPC).
/// A recusa estava provada só na função pura; forçar `network_ok` no laço passava (MD1/MD2).
#[test]
fn wifi_ativo_com_flag_bloqueia_no_arranque_e_no_ipc() {
    let show = escrever("td029-wifi.lumyx");
    let d = Daemon::subir_com("wifi-arranque", cfg(true, true), Some(show.clone()), &WIFI);
    let j = d.parar();
    assert!(j.contains(r#""to":"loaded""#), "premissa: o show carregou:\n{j}");
    assert!(!armou_ou_tocou(&j), "WiFi ativo + flag: o arranque não pode armar nem tocar:\n{j}");
    assert_eq!(contar(&j, "network_refused"), 1, "{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0, "a flag não cobre WiFi ativo:\n{j}");

    let d = Daemon::subir_com("wifi-ipc", cfg(true, false), None, &WIFI);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 2));
    let j = d.parar();
    assert!(resp.contains("preflight_failed"), "WiFi ativo + flag: o load tem de ser recusado: {resp}\n{j}");
    assert_eq!(contar(&j, "network_refused"), 1, "{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0, "{j}");
}

/// Uma linha de notice tem EXATAMENTE `{"t_ms":N,"notice":"X","detail":"…"}` — o formato do
/// `notice_to_json`, sem campos novos (D4). Devolve o nome da notice, ou `None` se a forma for outra.
fn notice_exata(linha: &str) -> Option<&str> {
    let resto = linha.strip_prefix(r#"{"t_ms":"#)?;
    let n = resto.find(|c: char| !c.is_ascii_digit())?;
    let resto = resto[n..].strip_prefix(r#","notice":""#)?;
    let fim_nome = resto.find('"')?;
    let (nome, resto) = resto.split_at(fim_nome);
    let detalhe = resto.strip_prefix(r#"","detail":""#)?.strip_suffix(r#""}"#)?;
    // Dentro do detalhe, toda a aspa tem de vir escapada: uma aspa crua é outro campo.
    let cruas = detalhe.match_indices('"').filter(|(i, _)| !detalhe[..*i].ends_with('\\')).count();
    (n > 0 && cruas == 0).then_some(nome)
}

/// **D4 — o caminho IPC escreve TODAS as notices do pré-voo, e no formato exato.** Não só as de
/// rede: `devices_checked` também tem de chegar (MC2). E nenhuma linha de notice do pré-voo pode
/// ganhar um campo (MC3, p.ex. `"via":"ipc"`).
#[test]
fn o_load_ipc_escreve_todas_as_notices_do_pre_voo_no_formato_exato() {
    let show = escrever("td029-ipc-formato.lumyx");
    let d = Daemon::subir("ipc-formato", cfg(true, false), None);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 2));
    assert!(resp.contains(r#""ok":true"#), "{resp}");
    let j = d.parar();

    assert_eq!(contar(&j, "devices_checked"), 1, "a notice de presença também vem do load IPC:\n{j}");
    let do_pre_voo = ["network_assumed_by_operator", "devices_checked"];
    let mut vistas = 0;
    for linha in j.lines().filter(|l| do_pre_voo.iter().any(|n| l.contains(&format!(r#""notice":"{n}""#)))) {
        let nome = notice_exata(linha).unwrap_or_else(|| panic!("formato diferente do notice_to_json: {linha}"));
        assert!(do_pre_voo.contains(&nome), "{linha}");
        vistas += 1;
    }
    assert_eq!(vistas, 2, "uma linha por notice do pré-voo:\n{j}");
}

/// **A flag só existe na CLI.** Nenhuma fonte do daemon lê variáveis de ambiente (MB1): o
/// override tem de ser escrito por quem arranca o processo, em cada execução.
#[test]
fn a_flag_nao_vem_do_ambiente() {
    for (nome, fonte) in [
        ("main.rs", include_str!("../src/main.rs")),
        ("run.rs", include_str!("../src/run.rs")),
        ("preflight.rs", include_str!("../src/preflight.rs")),
    ] {
        for proibido in ["env::var", "var_os(", "vars()"] {
            assert!(!fonte.contains(proibido), "{nome} lê o ambiente ({proibido})");
        }
    }
}
