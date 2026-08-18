//! GS4.2 (E2E) — **`.lumyx` → daemon → OutputManager → UDP**, pelo caminho oficial.
//!
//! A diferença para `pipeline.rs`: ali as peças foram ligadas à mão para provar que encaixam;
//! aqui quem as liga é o **laço do daemon**, com `run()` a fazer o que faz em produção —
//! carregar, pré-voo, armar, tocar, ticar. Se alguém desligar a saída do laço, é este
//! ficheiro que fica vermelho, e nenhum teste de unidade repararia.

use led_core::PixelColor;
use led_daemon::{ShowId, ShowRuntime, State};
use led_daemon_bin::{
    descriptor_from_path, run, Config, ExitReason, Integrity, Journal, Pacer, SystemPacer,
};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(unix)]
use led_daemon_bin::{run::run_with_control, server::Server, ControlPlane};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::sync::Arc;

/// Cabeçalho DDP: 10 bytes, com o offset em `[4..8]` big-endian.
#[cfg(unix)]
const DDP_HEADER_LEN: usize = 10;

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
        // `Vec` desde o ADR-0029: a saída passou a poder ter N nós. Este helper
        // continua a exprimir um só — as asserções deste ficheiro não mudaram.
        output: output.map(String::from).into_iter().collect(),
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
        // Art-Net e sACN exigem `@UNIVERSO` desde o ADR-0029 §7; o DDP recusa-o. Sem isto o
        // palco não abre — e foi assim que este teste apanhou a mudança, o que é o seu papel.
        let spec = match proto {
            "ddp" => sock.local_addr().unwrap().to_string(),
            // O E1.31 não define o universo 0 — Art-Net define (ADR-0029 §7.1).
            "sacn" => format!("{}@1", sock.local_addr().unwrap()),
            _ => format!("{}@0", sock.local_addr().unwrap()),
        };
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

/// **O laço liga o produtor ao instantâneo** (ADR-0029 §8).
///
/// O `estado_por_alvo.rs` prova que a atribuição por nó sobrevive até ao fio — mas constrói o
/// `Snapshot` à mão. Se o laço nunca chamasse `por_alvo()`, aquele ficheiro passaria na mesma:
/// o campo existiria, tipado e vazio para sempre, que é a forma mais silenciosa de um produtor
/// não ter consumidor. **Este teste não existia, e a falsificação foi quem o pediu**: mutar a
/// linha de `run.rs` para `Vec::new()` não punha nada vermelho no repositório inteiro.
///
/// Dois nós, porque um só não distingue "a lista veio do produtor" de "a lista tem uma entrada".
#[cfg(unix)]
#[test]
fn o_laco_publica_a_contabilidade_de_cada_no() {
    // **3000 px, e o número não é decorativo.** A repartição é DERIVADA do `max_pixels` do
    // preset (1500 aqui): com um show pequeno, `repartir` **recusa** dois endereços porque o
    // segundo nó ficaria com zero píxeis — e a primeira versão deste teste apanhou exactamente
    // essa recusa, com o daemon a não arrancar. Dois nós só existem quando o show os exige.
    let path = escrever("e2e_por_alvo.lumyx", 4, 25, 3000);
    let n1 = socket();
    let n2 = socket();
    let enderecos =
        vec![n1.local_addr().unwrap().to_string(), n2.local_addr().unwrap().to_string()];

    let desc = descriptor_from_path(&path, ShowId(1)).expect("carregar");
    let mut c = cfg(None, Some(preset_de("ddp")));
    c.output = enderecos.clone();

    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0 };
    let mut buf = Vec::new();
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    {
        let mut j = Journal::new(&mut buf);
        run_with_control(&mut rt, Some((path, desc)), &c, &mut p, &mut j, &flag, &cp);
    }

    let snap = cp.snapshot.lock().expect("snapshot").clone();
    assert_eq!(
        snap.outputs.len(),
        2,
        "dois nós configurados, duas contabilidades no instantâneo.\
         \n  Vazio significa que o laço NAO chama `por_alvo()` — o campo existe e nunca é\
         \n  preenchido, e o operador continua sem saber qual nó falhou.\
         \n  journal: {}",
        String::from_utf8_lossy(&buf)
    );

    for (i, esperado) in enderecos.iter().enumerate() {
        let (addr, frames, erros) = snap.outputs[i];
        assert_eq!(&addr.to_string(), esperado, "a entrada {i} tem de nomear o seu nó");
        assert!(
            frames > 0,
            "o nó {i} ({addr}) tem de ter enviado: um loopback nao falha, portanto \
             frames=0 aqui significa que a contabilidade nao veio do OutputManager"
        );
        assert_eq!(erros, 0, "loopback nao pode ter erros: {:?}", snap.outputs);
    }
}

/// **C — um nó morre e o laço NÃO cai: a perda é reportada uma vez e atribuída** (ADR-0029 §5).
///
/// ## O que este teste cobre e nenhum outro cobria
///
/// O `output.rs` prova o isolamento ao nível do `OutputManager`, e o
/// `o_laco_publica_a_contabilidade_de_cada_no` prova que o laço lê `por_alvo()` — mas **com os
/// dois nós vivos**. O caminho de FALHA através do laço não tinha teste nenhum: provado por
/// mutação, apagar `journal.line(… "output_error" …)` de `run.rs` deixava a suíte inteira do
/// crate verde. Por consequência, a de-duplicação (`ja_avisou`) também não estava coberta —
/// um teste que afirmasse "aparece uma vez" teria reprovado com o aviso ausente.
///
/// ## Porque o pacer é o REAL e não o virtual
///
/// O nó morto é `127.0.0.1:1`, e o erro só chega quando o ICMP port-unreachable volta — isso é
/// tempo de **relógio**. Com o `VPacer` os ticks executam em microssegundos e o laço acabaria o
/// show inteiro antes de o ICMP chegar: o teste passaria **sem exercitar nada**, que é o
/// falso-verde que este ficheiro existe para impedir.
///
/// Com o pacer do sistema a 25 ms por tick a margem é ~300× sobre os 76 µs medidos no C0. Não
/// é um `sleep`: é a cadência real do daemon, que é precisamente o que está sob teste.
#[cfg(unix)]
#[test]
fn um_no_morto_nao_derruba_o_laco_e_a_perda_e_reportada_uma_vez() {
    // O nó morto portátil — medido em Ubuntu e macOS no C0 (`probe/no-morto-portatil`).
    // Terceira cópia da constante, e é deliberado: extraí-la poria infraestrutura de teste na
    // superfície pública do crate, que é troca pior que três literais com o mesmo comentário.
    const ALVO_MORTO: &str = "127.0.0.1:1";

    // 3000 px porque a repartição é DERIVADA do `max_pixels` (1500): é o que exige dois nós.
    // 40 quadros a 25 ms ≈ 1 s de show, tempo de sobra para o ICMP voltar.
    let path = escrever("e2e_falha_parcial.lumyx", 40, 25, 3000);
    let vivo = socket();
    let enderecos = vec![vivo.local_addr().unwrap().to_string(), ALVO_MORTO.to_string()];

    let desc = descriptor_from_path(&path, ShowId(1)).expect("carregar");
    let mut c = cfg(None, Some(preset_de("ddp")));
    c.output = enderecos.clone();

    let mut rt = ShowRuntime::new();
    let mut p = SystemPacer::new();
    let mut buf = Vec::new();
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    let out = {
        let mut j = Journal::new(&mut buf);
        run_with_control(&mut rt, Some((path, desc)), &c, &mut p, &mut j, &flag, &cp)
    };
    let log = String::from_utf8(buf).expect("journal utf-8");
    let snap = cp.snapshot.lock().expect("snapshot").clone();

    // ── 1 · A premissa: o nó tem mesmo de morrer ─────────────────────────────
    //
    // Se ninguém morreu, este teste não exercitou o §5 — e passar seria pior que reprovar.
    let avisos = log.matches(r#""notice":"output_error""#).count();
    assert!(
        avisos > 0,
        "a condição de nó morto NÃO foi estabelecida: nenhum `output_error` no journal.\
         \n  Ou há um ouvinte em {ALVO_MORTO}, ou o laço deixou de reportar a perda.\
         \n  journal: {log}"
    );

    // ── 2 · Reportada UMA vez, não a 40 Hz ───────────────────────────────────
    //
    // Um erro de rede por tick encheria o journal e esconderia tudo o resto. É o `ja_avisou`
    // do `tick_do_palco`, que até agora não tinha teste nenhum.
    assert_eq!(
        avisos, 1,
        "a perda tem de ser registada UMA vez; vieram {avisos} avisos em {} ticks.\
         \n  Um erro por tick afoga o journal e esconde tudo o resto.\n  journal: {log}",
        out.ticks
    );

    // ── 3 · O laço NÃO caiu ──────────────────────────────────────────────────
    //
    // É o coração do §5: um nó perdido não pode virar falha global. O show tem de chegar ao fim.
    assert_eq!(
        out.reason,
        ExitReason::ReachedEnd,
        "um nó morto NÃO pode derrubar o laço — o show tinha de chegar ao fim, veio {:?} \
         com {} ticks.\n  journal: {log}",
        out.reason,
        out.ticks
    );
    assert_eq!(out.final_state, State::Finished, "estado final: {:?}", out.final_state);

    // ── 4 · O nó vivo continuou a acender, E COM A SUA FATIA ─────────────────
    //
    // Receber "alguma coisa" não chega: se o fan-out se enganasse e mandasse ao nó 0 a fatia
    // do nó 1, este socket receberia bytes na mesma e a asserção passaria. O que distingue é
    // o **offset** do cabeçalho DDP (bytes 4..8, big-endian), porque o helper `escrever`
    // produz quadros UNIFORMES — o conteúdo dos píxeis é idêntico e não separa nada.
    //
    // O nó 0 cobre os píxeis 0..1500 ⇒ offset em bytes 0. O nó 1 começa em 1500 ⇒ 4500.
    let mut offsets = Vec::new();
    let mut b = [0u8; 4096];
    while let Ok(n) = vivo.recv(&mut b) {
        if n >= DDP_HEADER_LEN {
            offsets.push(u32::from_be_bytes([b[4], b[5], b[6], b[7]]));
        }
    }
    assert!(
        !offsets.is_empty(),
        "o nó vivo tem de ter recebido bytes apesar do vizinho morto"
    );
    assert!(
        offsets.contains(&0),
        "o nó vivo tem de receber o INÍCIO do show (offset 0) — é a sua fatia.\
         \n  offsets vistos: {offsets:?}"
    );
    let fronteira = 1500u32 * 3; // onde começa a fatia do nó 1
    assert!(
        offsets.iter().all(|o| *o < fronteira),
        "nenhum datagrama do nó vivo pode cair na fatia do nó 1 (offset ≥ {fronteira}).\
         \n  Se cair, o fan-out atribuiu a fatia errada e o palco acenderia trocado.\
         \n  offsets vistos: {offsets:?}"
    );

    // ── 5 · A perda é ATRIBUÍDA, e o vivo fica limpo ─────────────────────────
    //
    // É isto que separa "um nó falhou" de "o sistema falhou". Um agregado diria que houve
    // erros e não diria de quem — e com cinco robôs isso manda procurar em cinco sítios.
    assert_eq!(snap.outputs.len(), 2, "dois nós, duas contabilidades: {:?}", snap.outputs);
    let (addr_vivo, frames_vivo, erros_vivo) = snap.outputs[0];
    let (addr_morto, _frames_morto, erros_morto) = snap.outputs[1];
    assert_eq!(&addr_vivo.to_string(), &enderecos[0], "a entrada 0 nomeia o nó vivo");
    assert_eq!(&addr_morto.to_string(), ALVO_MORTO, "a entrada 1 nomeia o nó morto");
    assert!(
        frames_vivo > 0 && erros_vivo == 0,
        "o nó VIVO tinha de enviar sem erros; veio (frames={frames_vivo}, erros={erros_vivo}).\
         \n  Erros no nó vivo significam que a falha do vizinho contaminou este.\
         \n  {:?}",
        snap.outputs
    );
    assert!(
        erros_morto > 0,
        "o nó MORTO tinha de acusar erro; veio erros={erros_morto}. Se for 0, a perda \
         desapareceu da contabilidade.\n  {:?}",
        snap.outputs
    );

    // ── 6 · A verdade chega ao FIO, não fica no instantâneo em memória ───────
    //
    // O `estado_por_alvo.rs` prova `Snapshot → Cmd::Status → fio`, mas com um `Snapshot`
    // construído à mão. Este teste prova `laço → Snapshot` com falha real. Compor os dois
    // por argumento seria repetir o erro do §8: lá, **cada metade parecia bem** e o elo do
    // meio não existia. Por isso o fio é interrogado aqui, sobre o instantâneo que o laço
    // REALMENTE produziu.
    let sock_path =
        std::env::temp_dir().join(format!("lumyx-c-falha-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock_path);
    let srv = Server::bind(&sock_path).expect("bind do socket de controlo");
    srv.spawn(Arc::clone(&cp));

    let s = UnixStream::connect(&sock_path).expect("ligar ao daemon");
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut w = s;
    let mut linha = String::new();
    writeln!(w, r#"{{"v":1,"id":1,"cmd":"hello","client":"teste-c"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();
    linha.clear();
    writeln!(w, r#"{{"v":1,"id":2,"cmd":"status"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();
    let _ = std::fs::remove_file(&sock_path);

    // O nó morto tem de aparecer no fio, nomeado e com erro. Um agregado — ou um estado
    // global inventado — falharia aqui.
    assert!(
        linha.contains(&format!(r#""addr":"{ALVO_MORTO}""#)),
        "o nó morto tem de ser NOMEADO no fio; sem nome, cinco robôs mandam procurar em \
         cinco sítios.\n  no fio: {linha}"
    );
    assert!(
        linha.contains(&format!(r#""addr":"{}""#, enderecos[0])),
        "e o nó vivo também.\n  no fio: {linha}"
    );
    assert!(
        linha.contains(r#""errors":0"#),
        "o nó vivo tem de aparecer com `errors:0` no fio — se todos tiverem erro, a falha \
         de um contaminou o relatório do outro.\n  no fio: {linha}"
    );
    // E o estado global continua a ser o real: nada de `Error` inventado por causa de um nó.
    assert!(
        linha.contains(r#""state":"finished""#),
        "o estado global tem de continuar `finished` — um nó perdido NÃO pode virar falha \
         global.\n  no fio: {linha}"
    );
}
