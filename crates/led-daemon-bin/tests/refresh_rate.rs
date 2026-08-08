//! ADR-0025 — **`refresh_hz` é um limite, e o daemon recusa ultrapassá-lo.**
//!
//! Escritos antes da implementação, como o ADR exige. A comparação corre **uma vez**, na
//! abertura do palco — não há relógio novo e o `Pacer` não muda.

use led_core::PixelColor;
use led_daemon::{ShowId, ShowRuntime, State};
use led_daemon_bin::{
    cadencia_cabe_no_profile, descriptor_from_path, profile_by_name, run, Config, ExitReason,
    Integrity, Journal, Pacer,
};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;

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

fn escrever(nome: &str) -> String {
    let path = std::env::temp_dir().join(nome);
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, 4).unwrap();
    for i in 0..4u32 {
        w.write_frame(&ShowRecord {
            timestamp_ms: i as u64 * 25,
            pixels: vec![PixelColor { r: 10, g: 20, b: 30 }; 4],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

/// Um profile com o teto que o caso precisa. **Não inventa hardware**: parte de um preset real
/// e muda só o campo em teste, para que nada mais possa explicar o veredito.
fn com_refresh(hz: u16) -> led_hardware_profile::HardwareProfile {
    let mut p = profile_by_name("esp32-poe-wled-ddp").unwrap();
    p.limits.refresh_hz = hz;
    p
}

// ── Casos 1–3 · a política do ADR-0025 ──────────────────────────────────────

/// **Caso 1 — dentro da capacidade.** 40 Hz declarados, 30 Hz pedidos (tick 33 ms).
#[test]
fn dentro_da_capacidade_e_permitido() {
    assert!(cadencia_cabe_no_profile(&com_refresh(40), 33).is_ok(), "1000/33 = 30,3 Hz ≤ 40");
}

/// **Caso 2 — exatamente no limite.** É o teste que distingue `>` de `>=`, e por isso o mais
/// fácil de escrever errado: o limite é **alcançável**, não proibido.
#[test]
fn exatamente_no_limite_e_permitido() {
    assert!(
        cadencia_cabe_no_profile(&com_refresh(40), 25).is_ok(),
        "1000/25 = 40 Hz exatos: o teto é alcançável"
    );
    assert!(cadencia_cabe_no_profile(&com_refresh(50), 20).is_ok(), "1000/20 = 50 Hz exatos");
    assert!(cadencia_cabe_no_profile(&com_refresh(100), 10).is_ok(), "1000/10 = 100 Hz exatos");
}

/// **Caso 3 — acima da capacidade.** Recusa, e o erro nomeia os dois números.
#[test]
fn acima_da_capacidade_e_recusado() {
    let e = cadencia_cabe_no_profile(&com_refresh(40), 20).unwrap_err();
    assert!(e.contains("50") && e.contains("40"), "o erro tem de dar os dois números: {e}");

    // O caso que motivou a auditoria: preset a 40 Hz, daemon a 100 Hz.
    assert!(cadencia_cabe_no_profile(&com_refresh(40), 10).is_err(), "100 Hz > 40 Hz");
    // E o extremo do sweep histórico, que no daemon não tem lugar (ADR-0025 §C).
    assert!(cadencia_cabe_no_profile(&com_refresh(44), 1).is_err(), "1000 Hz > 44 Hz");
}

/// **A fronteira é uma linha, não uma zona.** Um passo de cada lado do teto muda o veredito —
/// se não mudasse, o limite seria decorativo.
#[test]
fn um_passo_de_cada_lado_do_teto_muda_o_veredito() {
    let p = com_refresh(40);
    assert!(cadencia_cabe_no_profile(&p, 25).is_ok(), "40 Hz: permitido");
    assert!(cadencia_cabe_no_profile(&p, 24).is_err(), "41,7 Hz: recusado");
}

// ── Caso 5 · zero nunca é capacidade infinita ───────────────────────────────

/// **`refresh_hz = 0` não pode virar "ilimitado".**
///
/// Já é recusado pelo validador (`ZeroLimit`, ADR-0018) e desde o ADR-0024 essa recusa impede
/// a saída de abrir. Este teste **prova a composição das duas regras** em vez de a assumir —
/// era exatamente aqui que um "zero significa sem limite" poderia entrar sem ninguém reparar.
#[test]
fn refresh_zero_nunca_e_capacidade_infinita() {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap().to_string();
    let p = com_refresh(0);
    assert!(
        led_daemon_bin::OutputConfig::resolve(&p, &addr, 4, 1).is_err(),
        "refresh_hz=0 tem de impedir a saída (ZeroLimit + ADR-0024)"
    );
    // E a política de cadência também não o lê como ilimitado.
    assert!(cadencia_cabe_no_profile(&p, 1).is_err(), "0 Hz declarado não autoriza 1000 Hz");
}

// ── A interação com o pacer, ponta a ponta ──────────────────────────────────

/// **O laço recusa arrancar, e o `tick_ms` pedido nunca é alterado.**
#[test]
fn o_daemon_recusa_arrancar_acima_do_teto_e_nao_clampa() {
    let path = escrever("a3_acima.lumyx");
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let cfg = Config {
        tick_ms: 5, // 200 Hz contra um preset de 44 Hz
        max_ticks: Some(5),
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: Some(sock.local_addr().unwrap().to_string()),
        profile: Some("esp32-poe-wled-ddp".to_string()),
    };
    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let desc = descriptor_from_path(&path, ShowId(1)).unwrap();
    let out = {
        let mut j = Journal::new(&mut buf);
        run(&mut rt, &path, desc, &cfg, &mut p, &mut j, &flag)
    };
    let log = String::from_utf8(buf).unwrap();

    assert_eq!(out.reason, ExitReason::NeverStarted, "{log}");
    assert_ne!(out.final_state, State::Playing, "nunca toca acima do teto");
    assert!(log.contains(r#""notice":"output_failed""#), "{log}");
    assert_eq!(cfg.tick_ms, 5, "o daemon NÃO altera a cadência pedida (nunca clampa)");
    let _ = std::fs::remove_file(path);
}

/// **Controle negativo do teste acima**: com um `tick_ms` dentro do teto, o mesmo preset e o
/// mesmo caminho tocam até ao fim. Sem isto, o teste de recusa podia estar a apanhar
/// qualquer outra falha e passar por bom.
#[test]
fn dentro_do_teto_o_mesmo_caminho_toca_ate_ao_fim() {
    let path = escrever("a3_dentro.lumyx");
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let cfg = Config {
        tick_ms: 25, // 40 Hz ≤ 44 Hz do preset
        max_ticks: None,
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: Some(sock.local_addr().unwrap().to_string()),
        profile: Some("esp32-poe-wled-ddp".to_string()),
    };
    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let desc = descriptor_from_path(&path, ShowId(1)).unwrap();
    let out = {
        let mut j = Journal::new(&mut buf);
        run(&mut rt, &path, desc, &cfg, &mut p, &mut j, &flag)
    };
    let log = String::from_utf8(buf).unwrap();
    assert_eq!(out.reason, ExitReason::ReachedEnd, "{log}");
    assert_eq!(out.final_state, State::Finished);
    let _ = std::fs::remove_file(path);
}

/// **Sem `--output` não há verificação** — não há nó para proteger. É o mesmo raciocínio da
/// vacuidade do pré-voo do GS2, e sem este teste a regra poderia alastrar para o modo mudo.
#[test]
fn sem_saida_a_cadencia_nao_e_limitada() {
    let path = escrever("a3_mudo.lumyx");
    let cfg = Config {
        tick_ms: 1, // 1000 Hz — absurdo para qualquer nó, mas não há nó
        max_ticks: Some(3),
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: None,
        profile: None,
    };
    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let desc = descriptor_from_path(&path, ShowId(1)).unwrap();
    let out = {
        let mut j = Journal::new(&mut buf);
        run(&mut rt, &path, desc, &cfg, &mut p, &mut j, &flag)
    };
    assert_eq!(out.reason, ExitReason::MaxTicks, "sem saída, ninguém limita a cadência");
    assert_eq!(out.ticks, 3);
    let _ = std::fs::remove_file(path);
}

/// **`heartbeat_ms` e `refresh_hz` medem coisas diferentes e não se contaminam.** O keep-alive
/// é o intervalo *máximo* sem frame; o refresh é a taxa *máxima* de frames. Confundi-los daria
/// um daemon que recusa 40 Hz por causa de um heartbeat de 800 ms.
#[test]
fn o_heartbeat_e_o_refresh_nao_se_confundem() {
    let p = com_refresh(40);
    assert_eq!(p.transport.heartbeat_ms, 800, "1,25 Hz de keep-alive");
    // 40 Hz é 32× mais rápido que o heartbeat, e continua a ser permitido.
    assert!(cadencia_cabe_no_profile(&p, 25).is_ok());
}
