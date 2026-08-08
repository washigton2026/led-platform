//! GS4.2 (E2E) — **`.lumyx` → daemon → OutputManager → UDP**, pelo caminho oficial.
//!
//! A diferença para `pipeline.rs`: ali as peças foram ligadas à mão para provar que encaixam;
//! aqui quem as liga é o **laço do daemon**, com `run()` a fazer o que faz em produção —
//! carregar, pré-voo, armar, tocar, ticar. Se alguém desligar a saída do laço, é este
//! ficheiro que fica vermelho, e nenhum teste de unidade repararia.

use led_core::PixelColor;
use led_daemon::{ShowId, ShowRuntime, State};
use led_daemon_bin::{
    descriptor_from_path, run, Config, ExitReason, Integrity, Journal, Pacer,
};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Pacer virtual: o laço não dorme, e o teste não depende do relógio da máquina.
struct VPacer {
    now: u64,
}
impl Pacer for VPacer {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn sleep_until(&mut self, deadline_ms: u64) {
        self.now = self.now.max(deadline_ms);
    }
}

fn escrever(nome: &str, frames: u32, passo: u64, px: u32) -> String {
    let path = std::env::temp_dir().join(nome);
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, px).unwrap();
    for i in 0..frames {
        w.write_frame(&ShowRecord {
            timestamp_ms: i as u64 * passo,
            pixels: vec![PixelColor { r: 10 + i as u8, g: 20, b: 30 }; px as usize],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

fn preset_de(proto: &str) -> &'static str {
    match proto {
        "ddp" => "esp32-poe-wled-ddp",
        "artnet" => "esp32-devkit-wled-artnet",
        "sacn" => "falcon-f16v3-sacn",
        outro => panic!("sem preset para {outro}"),
    }
}

fn cfg(output: Option<&str>, profile: Option<&str>) -> Config {
    Config {
        // 25 ms = 40 Hz. **Não é cosmético**: os presets declaram 40–44 Hz, e os 20 ms
        // (50 Hz) que aqui estavam eram uma cadência acima da capacidade declarada — o
        // ADR-0025 passou a recusá-la, e foi assim que este ficheiro a revelou.
        tick_ms: 25,
        max_ticks: None,
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: output.map(String::from),
        profile: profile.map(String::from),
    }
}

/// Corre o daemon até ao fim do show e devolve `(datagramas recebidos, journal)`.
fn correr(path: &str, cfg: Config, sock: &UdpSocket) -> (usize, String, ExitReason, State) {
    let desc = descriptor_from_path(path, ShowId(1)).expect("carregar");
    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let out = {
        let mut j = Journal::new(&mut buf);
        run(&mut rt, path, desc, &cfg, &mut p, &mut j, &flag)
    };
    let mut n = 0;
    let mut b = [0u8; 4096];
    while sock.recv(&mut b).is_ok() {
        n += 1;
    }
    (n, String::from_utf8(buf).unwrap(), out.reason, out.final_state)
}

fn socket() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
    s
}

/// **O caminho oficial põe bytes no fio, nos três protocolos.**
#[test]
fn o_daemon_envia_frames_reais_em_ddp_artnet_e_sacn() {
    for proto in ["ddp", "artnet", "sacn"] {
        let path = escrever(&format!("e2e_{proto}.lumyx"), 8, 25, 6);
        let sock = socket();
        let spec = sock.local_addr().unwrap().to_string();
        let (n, log, reason, estado) =
            correr(&path, cfg(Some(&spec), Some(preset_de(proto))), &sock);

        assert_eq!(reason, ExitReason::ReachedEnd, "{proto}: {log}");
        assert_eq!(estado, State::Finished, "{proto}");
        assert!(n > 0, "{proto}: NENHUM datagrama saiu do daemon — a saída não está ligada");
        assert!(log.contains(r#""notice":"output_open""#), "{proto}: {log}");
        assert!(log.contains(r#""notice":"profile""#), "{proto}: o preset tem de ficar no journal");
        assert!(
            !log.contains("nenhum frame deixa este processo"),
            "{proto}: a frase do GS2 deixou de ser verdadeira e não pode continuar no journal"
        );
        assert!(!log.contains(r#""notice":"output_error""#), "{proto}: {log}");
        let _ = std::fs::remove_file(path);
    }
}

/// **Controle negativo.** Sem `--output` nada sai — se este teste apanhasse datagramas, o
/// teste de cima estaria a medir outra coisa qualquer no socket.
#[test]
fn sem_output_o_daemon_continua_a_nao_enviar_nada() {
    let path = escrever("e2e_mudo.lumyx", 8, 25, 6);
    let sock = socket();
    let (n, log, reason, _) = correr(&path, cfg(None, None), &sock);
    assert_eq!(reason, ExitReason::ReachedEnd);
    assert_eq!(n, 0, "sem saída configurada, o fio tem de ficar em silêncio");
    assert!(log.contains(r#""notice":"preflight_vacuous""#), "e a vacuidade continua dita");
    let _ = std::fs::remove_file(path);
}

/// **A vacuidade do pré-voo acabou quando há saída.** O journal passa a trazer o veredito de
/// cada sonda, e nunca mais a frase de vacuidade.
#[test]
fn com_output_o_preflight_deixa_de_ser_vacuoso() {
    let path = escrever("e2e_preflight.lumyx", 4, 25, 4);
    let sock = socket();
    let spec = sock.local_addr().unwrap().to_string();
    let (_, log, _, _) = correr(&path, cfg(Some(&spec), Some("esp32-poe-wled-ddp")), &sock);

    assert!(
        !log.contains(r#""notice":"preflight_vacuous""#),
        "com saída, nada pode continuar a ser declarado vacuoso: {log}"
    );
    assert!(
        log.contains(r#""notice":"network_local""#),
        "num alvo de loopback a rede tem de ser declarada local, não `ok` por omissão: {log}"
    );
    // O alvo é loopback: a sonda ArtPoll recusa-se a fingir que descobriu um rig ali.
    assert!(
        log.contains(r#""notice":"devices_unverified""#),
        "num alvo de loopback a sonda tem de dizer que NÃO verificou: {log}"
    );
    assert!(
        !log.contains(r#""notice":"devices_checked""#),
        "e nunca pode afirmar que verificou: {log}"
    );
    let _ = std::fs::remove_file(path);
}

/// Saída inválida **não arranca** — em vez de tocar um show para lado nenhum.
#[test]
fn saida_impossivel_impede_o_arranque() {
    let path = escrever("e2e_ruim.lumyx", 4, 25, 4);
    let mut rt = ShowRuntime::new();
    let desc = descriptor_from_path(&path, ShowId(1)).unwrap();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let out = {
        let mut j = Journal::new(&mut buf);
        run(
            &mut rt,
            &path,
            desc,
            &cfg(Some("nao-e-um-ip"), Some("esp32-poe-wled-ddp")),
            &mut p,
            &mut j,
            &flag,
        )
    };
    let log = String::from_utf8(buf).unwrap();
    assert_eq!(out.reason, ExitReason::NeverStarted);
    assert!(log.contains(r#""notice":"output_failed""#), "{log}");
    let _ = std::fs::remove_file(path);
}
