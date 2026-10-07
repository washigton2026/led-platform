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
use led_daemon_bin::run::{run_com, run_with_control_com};
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

struct SondaOk;
impl NetworkGuard for SondaOk {
    fn check(&self) -> Result<(), NetworkPolicyError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "sonda-ok"
    }
}

static FALHADA: SondaFalhada = SondaFalhada;
static OK: SondaOk = SondaOk;
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
    // O efeito, não só a resposta: um `play` a seguir também tem de ser recusado (MIPC1).
    let play = pedir(&mut s, &mut r, r#"{"v":1,"id":3,"cmd":"play"}"#);
    // E DEPOIS do play recusado: o laço não arma nem toca em silêncio (MIPC4), nem envia (MIPC5).
    nada_saiu_pelo_fio(&d);
    let j = d.parar();
    assert!(!armou_ou_tocou(&j), "depois do load recusado o daemon não pode tocar:\n{j}");

    assert!(resp.contains("preflight_failed"), "sonda falhada sem override bloqueia: {resp}");
    assert!(play.contains(r#""ok":false"#) && play.contains("not_armed"),
            "depois do load recusado o play é recusado POR NÃO ESTAR ARMADO (não por já tocar): {play}");
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
/// Espera (causal, com prazo) que o laço publique pelo menos `n` ticks — o `status` só conta
/// ticks que o laço correu. Assim o oráculo vê também o que o laço faz DEPOIS do arranque
/// (falsificador ronda 2, MG1: armar e tocar no 1.º tick escapava a quem parava logo).
fn esperar_ticks(d: &Daemon, n: u64) {
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let fim = Instant::now() + Duration::from_secs(5);
    loop {
        let st = pedir(&mut s, &mut r, r#"{"v":1,"id":2,"cmd":"status"}"#);
        let ticks = st.split(r#""ticks":"#).nth(1).and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
        if ticks >= n {
            return;
        }
        assert!(Instant::now() < fim, "o laço não chegou a {n} ticks: {st}");
        std::thread::yield_now();
    }
}

/// Depois de um `load` recusado: deixa o laço correr 40 ticks e afirma, pelo `status`, que NENHUM
/// quadro saiu (MIPC5: quadros enviados contornando o runtime) — o fio, não só o estado.
fn nada_saiu_pelo_fio(d: &Daemon) {
    esperar_ticks(d, 40);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let st = pedir(&mut s, &mut r, r#"{"v":1,"id":2,"cmd":"status"}"#);
    assert!(st.contains(r#""frames":"#), "premissa: o palco está aberto e o status conta quadros: {st}");
    for parte in st.split(r#""frames":"#).skip(1) {
        let n: u64 = parte.split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap();
        assert_eq!(n, 0, "um load recusado não pode pôr quadros no fio: {st}");
    }
}

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
    esperar_ticks(&d, 40);
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
    esperar_ticks(&d, 40);
    let j = d.parar();
    assert!(j.contains(r#""to":"loaded""#), "premissa: o show carregou:\n{j}");
    assert!(!armou_ou_tocou(&j), "WiFi ativo + flag: o arranque não pode armar nem tocar:\n{j}");
    assert_eq!(contar(&j, "network_refused"), 1, "{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0, "a flag não cobre WiFi ativo:\n{j}");

    let d = Daemon::subir_com("wifi-ipc", cfg(true, false), None, &WIFI);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 2));
    let play = pedir(&mut s, &mut r, r#"{"v":1,"id":3,"cmd":"play"}"#);
    nada_saiu_pelo_fio(&d);
    let j = d.parar();
    assert!(!armou_ou_tocou(&j), "WiFi + flag: depois do load recusado o daemon não pode tocar:\n{j}");
    assert!(resp.contains("preflight_failed"), "WiFi ativo + flag: o load tem de ser recusado: {resp}\n{j}");
    assert!(play.contains(r#""ok":false"#) && play.contains("not_armed"), "WiFi + flag: o play não toca: {play}");
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
    // D4 «zero tipos novos»: TODA a linha de notice do journal tem um nome que já existia
    // (falsificador ronda 2, MC5). A linha final `state` tem outro formato (state_to_json).
    const CONHECIDAS: &[&str] = &[
        "mode", "profile", "output_open", "output_error", "output_failed", "load_refused",
        "arm_refused", "play_refused", "integrity_assumed", "started", "shutdown", "log_write_failed",
        "network_checked", "network_local", "network_refused", "network_unverified",
        "network_probe_failed", "network_assumed_by_operator", "network_override_unused",
        "devices_checked", "devices_missing", "devices_unverified", "preflight_vacuous",
    ];
    for linha in j.lines() {
        assert!(
            linha.starts_with(r#"{"t_ms":"#) && (linha.contains(r#","notice":""#) || linha.contains(r#","event":""#)),
            "linha do JSONL que não é notice nem event (D4, MC6): {linha}"
        );
    }
    const EVENTOS: &[&str] = &["transitioned", "position_changed", "show_loaded", "show_unloaded",
                                "reached_end", "faulted", "fault_cleared"];
    for linha in j.lines().filter(|l| l.contains(r#","event":""#)) {
        let tipo = linha.split(r#","event":""#).nth(1).and_then(|r| r.split('"').next()).unwrap_or("");
        assert!(EVENTOS.contains(&tipo), "tipo de evento novo no journal (MC6e): {linha}");
        // Chaves EXATAS do event_to_json, por tipo (MC6f: um campo novo nos eventos passava).
        let extra: &[&str] = match tipo {
            "transitioned" => &["from", "to"],
            "show_loaded" | "show_unloaded" => &["show_id"],
            "position_changed" => &["ms", "cause"],
            "faulted" => &["code"],
            _ => &[],
        };
        let mut esperadas: Vec<&str> = vec!["t_ms", "event"];
        esperadas.extend_from_slice(extra);
        let chaves: Vec<&str> = linha
            .match_indices("\":")
            .filter_map(|(i, _)| linha[..i].rsplit('"').next())
            .collect();
        assert_eq!(chaves, esperadas, "chaves do evento {tipo} diferentes do event_to_json: {linha}");
    }
    for linha in j.lines().filter(|l| l.contains(r#""notice":"#) && !l.contains(r#""notice":"state""#)) {
        let nome = notice_exata(linha).unwrap_or_else(|| panic!("formato diferente do notice_to_json: {linha}"));
        assert!(CONHECIDAS.contains(&nome), "tipo de notice novo no journal: {linha}");
    }
}

/// **A flag só existe na CLI.** Nenhuma fonte do daemon lê variáveis de ambiente (MB1): o
/// override tem de ser escrito por quem arranca o processo, em cada execução.
#[test]
fn a_flag_nao_vem_do_ambiente() {
    // Todos os ficheiros do src do crate (falsificador ronda 2: a leitura mudada para o
    // loader.rs, ou um alias `use std::env as e`, fugiam a uma lista de 3 ficheiros).
    // Os únicos usos de `std::env` permitidos são `args()` e `temp_dir()`.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut vistos = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        vistos += 1;
        let fonte = std::fs::read_to_string(&p).unwrap();
        let nome = p.file_name().unwrap().to_string_lossy().to_string();
        for proibido in ["use std::env", "std::env as"] {
            assert!(!fonte.contains(proibido), "{nome} lê o ambiente ({proibido})");
        }
        // `var(` só como identificador inteiro (`reprovar(` não conta).
        for f in ["var(", "var_os(", "vars(", "vars_os("] {
            for (i, _) in fonte.match_indices(f) {
                let antes = fonte[..i].chars().next_back();
                assert!(
                    antes.is_some_and(|c| c.is_alphanumeric() || c == '_'),
                    "{nome} lê o ambiente ({f}): {}",
                    &fonte[i.saturating_sub(20)..(i + 20).min(fonte.len())]
                );
            }
        }
        for (i, _) in fonte.match_indices("std::env") {
            let resto = &fonte[i + "std::env".len()..];
            assert!(
                resto.starts_with("::args()") || resto.starts_with("::temp_dir()"),
                "{nome}: uso de std::env que não é args()/temp_dir(): {}",
                &fonte[i..(i + 40).min(fonte.len())]
            );
        }
    }
    assert!(vistos >= 5, "premissa: o teste tem de ler os ficheiros do src ({vistos})");
}

/// **Modo CLI (`run`, sem `--socket`), sem a flag e com a sonda falhada: não arma.** O `run` e o
/// `run_with_control` são os dois chamadores do pré-voo (accept R5, item 3); até à ronda 2 do
/// falsificador só o segundo tinha teste (MA1R: forçar o override aqui deixava a suite verde).
fn correr_cli(nome: &str, assume_no_wifi: bool, guarda: &dyn NetworkGuard) -> String {
    correr_cli_com_outcome(nome, assume_no_wifi, guarda).0
}

fn correr_cli_com_outcome(
    nome: &str,
    assume_no_wifi: bool,
    guarda: &dyn NetworkGuard,
) -> (String, led_daemon_bin::Outcome) {
    let show = escrever(nome);
    let d = descriptor_from_path(&show, led_daemon::ShowId(1)).expect("show");
    let mut c = cfg(assume_no_wifi, true);
    c.max_ticks = Some(3);
    let buf = Buf::default();
    let mut jn = Journal::new(buf.clone());
    let mut rt = ShowRuntime::new();
    let mut pacer = SystemPacer::new();
    let parar = AtomicBool::new(false);
    let o = run_com(&mut rt, &show, d, &c, &mut pacer, &mut jn, &parar, guarda, &Presente);
    (buf.texto(), o)
}

/// O efeito medido no `Outcome`, não no texto (MCLI3: escrever `arm_refused`/`NeverStarted` e
/// tocar em silêncio passava o oráculo textual).
fn nao_armou_outcome(o: &led_daemon_bin::Outcome) {
    assert_eq!(o.final_state, led_daemon::State::Loaded, "{o:?}");
    assert_eq!(o.ticks, 0, "bloqueado no arranque, o laço não tica: {o:?}");
    assert_eq!(o.reason, led_daemon_bin::ExitReason::NeverStarted, "{o:?}");
}

/// O EFEITO no modo CLI, não só a linha `arm_refused` (MCLI1): nenhuma transição para ready ou
/// playing, e o processo termina como `NeverStarted`.
fn nao_armou_cli(j: &str) {
    assert!(!j.contains(r#""to":"ready""#) && !j.contains(r#""to":"playing""#), "armou ou tocou:\n{j}");
    assert!(j.contains("NeverStarted"), "o modo CLI bloqueado termina como NeverStarted:\n{j}");
}

#[test]
fn modo_cli_sem_override_com_sonda_falhada_nao_arma() {
    let (j, o) = correr_cli_com_outcome("td029-cli-sem.lumyx", false, &FALHADA);
    nao_armou_outcome(&o);
    assert_eq!(contar(&j, "network_probe_failed"), 1, "{j}");
    assert_eq!(contar(&j, "arm_refused"), 1, "sem a flag o modo CLI não arma:\n{j}");
    nao_armou_cli(&j);
    assert_eq!(contar(&j, "network_assumed_by_operator"), 0, "{j}");
    // Controlo positivo do mesmo oráculo: com a flag, arma e corre os 3 ticks.
    let j = correr_cli("td029-cli-com.lumyx", true, &FALHADA);
    assert_eq!(contar(&j, "arm_refused"), 0, "com a flag e a sonda falhada o modo CLI arma:\n{j}");
    assert_eq!(contar(&j, "network_assumed_by_operator"), 1, "{j}");
}

/// **Modo CLI: WiFi ativo bloqueia SEMPRE, com a flag** (MD2R).
#[test]
fn modo_cli_wifi_ativo_com_flag_nao_arma() {
    let (j, o) = correr_cli_com_outcome("td029-cli-wifi.lumyx", true, &WIFI);
    nao_armou_outcome(&o);
    assert_eq!(contar(&j, "network_refused"), 1, "{j}");
    assert_eq!(contar(&j, "arm_refused"), 1, "WiFi ativo + flag: o modo CLI não pode armar:\n{j}");
    nao_armou_cli(&j);
}

/// **O aviso em stderr (accept item 4), no binário real** — e só com a flag (MH2).
#[test]
fn o_binario_avisa_em_stderr_so_com_a_flag() {
    let show = escrever("td029-bin.lumyx");
    let correr = |extra: &[&str]| {
        let mut args = vec![show.as_str(), "--max-ticks", "1", "--tick-ms", "25"];
        args.extend_from_slice(extra);
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_led-daemon"))
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("led-daemon");
        String::from_utf8_lossy(&o.stderr).to_string()
    };
    let com = correr(&["--assume-no-wifi"]);
    assert!(com.contains("AVISO: --assume-no-wifi"), "a flag tem de avisar em stderr: {com}");
    // Também com saída — o caso de produção (MH5: avisar só sem --output passava).
    let com_saida = correr(&["--assume-no-wifi", "--output", ALVO, "--profile", PRESET]);
    assert!(com_saida.contains("AVISO: --assume-no-wifi"), "com --output também avisa: {com_saida}");
    // E no modo --socket (MH6).
    let sock = std::env::temp_dir().join(format!("lumyx-td029-aviso-{}.sock", std::process::id()));
    let com_socket = correr(&["--assume-no-wifi", "--socket", sock.to_str().unwrap()]);
    assert!(com_socket.contains("AVISO: --assume-no-wifi"), "com --socket também avisa: {com_socket}");
    // No ARRANQUE (MH3): num processo que fica a correr, o aviso chega antes de o mandarmos parar.
    {
        use std::io::Read;
        let mut filho = std::process::Command::new(env!("CARGO_BIN_EXE_led-daemon"))
            .args([show.as_str(), "--assume-no-wifi", "--assume-integrity", "--keep-running", "--tick-ms", "25"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("led-daemon");
        let mut err = filho.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut b = [0u8; 256];
            let mut acc = String::new();
            while let Ok(n) = err.read(&mut b) {
                if n == 0 { break; }
                acc.push_str(&String::from_utf8_lossy(&b[..n]));
                if acc.contains("AVISO: --assume-no-wifi") { let _ = tx.send(()); break; }
            }
        });
        let a_tempo = rx.recv_timeout(Duration::from_secs(10)).is_ok();
        // Premissa: o processo ainda está vivo (senão «antes do fim» não prova nada).
        let vivo = filho.try_wait().expect("try_wait").is_none();
        writeln!(filho.stdin.as_mut().unwrap(), "shutdown").ok();
        let _ = filho.wait();
        assert!(vivo, "premissa: com --keep-running o daemon tem de continuar vivo");
        assert!(a_tempo, "o aviso tem de sair no arranque, com o processo ainda a correr");
    }
    let sem = correr(&[]);
    assert!(!sem.contains("--assume-no-wifi"), "sem a flag não há aviso: {sem}");
}

/// **A guarda REAL é a que o binário usa com `--output`, nos dois modos** (MR1/MR2: passar a
/// permissiva ao `run_com`, ou escolhê-la sempre no `guarda_para`, deixava a suite verde — todos os
/// outros testes com saída injetam a guarda ou usam loopback). Portável: não importa o que a guarda
/// real decide nesta máquina (WiFi ativo, inativo ou sonda falhada), importa que NÃO é a permissiva.
#[test]
fn o_binario_com_saida_consulta_a_guarda_real_nos_dois_modos() {
    let show = escrever("td029-guarda-real.lumyx");
    let sock = std::env::temp_dir().join(format!("lumyx-td029-gr-{}.sock", std::process::id()));
    let sock = sock.to_str().unwrap().to_string();
    for extra in [vec![], vec!["--socket", sock.as_str()]] {
        let mut args = vec![show.as_str(), "--output", ALVO, "--profile", PRESET, "--assume-integrity",
                            "--max-ticks", "2", "--tick-ms", "25"];
        args.extend(extra.iter().copied());
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_led-daemon"))
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("led-daemon");
        let j = String::from_utf8_lossy(&o.stdout);
        let rede = ["network_checked", "network_refused", "network_probe_failed", "network_unverified"]
            .iter()
            .map(|n| contar(&j, n))
            .sum::<usize>();
        assert_eq!(rede, 1, "premissa: o pré-voo de rede correu uma vez ({extra:?}):\n{j}");
        assert!(!j.contains("PermissiveGuard"), "com --output a guarda tem de ser a real ({extra:?}):\n{j}");
        // Sem a flag, o override NUNCA aparece (MR4/MR5: o `run`/`run_with_control` a ligá-lo sozinhos).
        // Discrimina onde a guarda real não reprova por WiFi (CI sem WiFi → `override_unused`).
        assert_eq!(contar(&j, "network_override_unused") + contar(&j, "network_assumed_by_operator"), 0,
                   "sem --assume-no-wifi não pode haver override ({extra:?}):\n{j}");
    }
}

/// **D2(b) pelo LAÇO, nos dois caminhos** (MC8: filtrar `network_override_unused` no laço
/// passava — só a função pura o provava): flag + sonda que verifica ⇒ o aviso «sem efeito» fica
/// no journal no modo CLI e em cada `load` IPC.
#[test]
fn flag_com_sonda_ok_deixa_o_aviso_sem_efeito_no_journal_nos_dois_caminhos() {
    let j = correr_cli("td029-cli-ok.lumyx", true, &OK);
    assert_eq!(contar(&j, "network_override_unused"), 1, "modo CLI:\n{j}");
    let show = escrever("td029-ipc-ok.lumyx");
    let d = Daemon::subir_com("ipc-ok", cfg(true, false), None, &OK);
    let (mut s, mut r) = d.cliente();
    pedir(&mut s, &mut r, r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 2));
    assert!(resp.contains(r#""ok":true"#), "{resp}");
    pedir(&mut s, &mut r, r#"{"v":1,"id":3,"cmd":"unload"}"#);
    let resp = pedir(&mut s, &mut r, &load(&show, 4));
    assert!(resp.contains(r#""ok":true"#), "{resp}");
    let j = d.parar();
    assert_eq!(contar(&j, "network_override_unused"), 2, "um por load IPC (MC8c):\n{j}");
    // O terceiro caminho: o arranque do modo --socket (MC8b).
    let show = escrever("td029-arranque-ok.lumyx");
    let d = Daemon::subir_com("arranque-ok", cfg(true, true), Some(show), &OK);
    let j = d.parar();
    assert_eq!(contar(&j, "network_override_unused"), 1, "arranque com --socket:\n{j}");
}
