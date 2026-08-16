//! ADR-0029 §8 — **o estado por alvo chega ao operador pelo `status`.**
//!
//! `por_alvo()` existia e não tinha consumidor: a contabilidade de cada nó morria dentro do
//! processo, e o operador via um agregado que **não distingue** cinco nós a funcionar de
//! quatro a funcionar e um morto. Este ficheiro prova o elo que faltava, no fio.
//!
//! **Três alvos, de propósito.** Com **um** alvo, "por nó" e "agregado" são indistinguíveis —
//! foi exactamente assim que duas mutações do passo 1 do ADR-0029 (`all`→`any` e "sondar só o
//! primeiro") não apanharam nada. Um teste que não distingue os dois mundos não prova nenhum
//! (KB-012). Aqui o nó do meio **falha**, e os outros dois não: um relatório que repetisse o
//! agregado daria `frames=2, errors=1` nos três e reprova.

#![cfg(unix)]

use led_daemon_bin::json::{parse, Json};
use led_daemon_bin::output::Alvo;
use led_daemon_bin::server::{ControlPlane, Server, Snapshot};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_core::{LogicalFrame, PixelColor};
use std::io::{BufRead, BufReader, Write};
use std::net::UdpSocket;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Cliente mínimo do IPC v1.
///
/// É a mesma forma que o `ipc.rs` usa, e **não** foi extraída para um módulo comum de
/// propósito: fazê-lo obrigaria a editar o `ipc.rs`, que guarda o GS3, numa fatia que é sobre
/// o §8. Vinte linhas duplicadas num teste custam menos que tocar num ficheiro fora do âmbito.
fn status_no_fio(cp: &Arc<ControlPlane>, nome: &str) -> String {
    let path = std::env::temp_dir().join(format!("lumyx-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let srv = Server::bind(&path).expect("bind");
    srv.spawn(Arc::clone(cp));

    let s = UnixStream::connect(&path).expect("ligar");
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut w = s;
    let mut linha = String::new();

    writeln!(w, r#"{{"v":1,"id":1,"cmd":"hello","client":"teste-por-alvo"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();

    linha.clear();
    writeln!(w, r#"{{"v":1,"id":2,"cmd":"status"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();
    linha.trim().to_string()
}

/// **A perda é atribuída ao nó certo, e sobrevive até ao fio.**
#[test]
fn o_status_distingue_o_no_morto_dos_que_acenderam() {
    let vivo1 = UdpSocket::bind("127.0.0.1:0").unwrap();
    let vivo2 = UdpSocket::bind("127.0.0.1:0").unwrap();

    let perfil = profile_by_name("esp32-poe-wled-ddp").expect("preset");
    let mut cfg = OutputConfig::resolve(&perfil, "127.0.0.1", 24).expect("resolver");
    cfg.alvos = vec![
        Alvo {
            addr: vivo1.local_addr().unwrap(),
            first_universe: 1,
            pixel_offset: 0,
            pixel_count: 8,
        },
        // Porta 1 num endereço de broadcast: o `connect` passa e o `send` devolve EACCES.
        // É a mesma técnica que o `output.rs` já usa para produzir uma falha real de nó.
        Alvo {
            addr: "255.255.255.255:1".parse().unwrap(),
            first_universe: 1,
            pixel_offset: 8,
            pixel_count: 8,
        },
        Alvo {
            addr: vivo2.local_addr().unwrap(),
            first_universe: 1,
            pixel_offset: 16,
            pixel_count: 8,
        },
    ];

    let mgr = OutputManager::open(cfg).expect(
        "a abertura tem de passar: o connect a um destino de broadcast nao falha, so o send. \
         Se falhou, este teste deixou de exercitar o que afirma",
    );
    let _ = mgr.send(&LogicalFrame::new(vec![PixelColor { r: 9, g: 0, b: 0 }; 24], 0));

    let por_alvo = mgr.por_alvo();
    assert_eq!(por_alvo.len(), 3, "tres alvos, tres contabilidades: {por_alvo:?}");

    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    *cp.snapshot.lock().unwrap() = Snapshot { outputs: por_alvo.clone(), ..Default::default() };

    let linha = status_no_fio(&cp, "por-alvo");
    let j = parse(&linha).unwrap_or_else(|e| panic!("o status tem de ser JSON valido: {e}\n{linha}"));

    let Some(Json::Arr(saidas)) = j.get("outputs") else {
        panic!("o `status` nao traz `outputs` como array — o §8 nao chegou ao fio.\n{linha}");
    };
    assert_eq!(saidas.len(), 3, "tres nos, tres entradas: {linha}");

    // **A asserção que mata o agregado repetido.** Somado, daria (2, 1) nos três.
    for (i, esperado) in [(0usize, (1u64, 0u64)), (1, (0, 1)), (2, (1, 0))] {
        let e = &saidas[i];
        let addr = e.get("addr").and_then(Json::as_str).unwrap_or("<sem addr>");
        let frames = e.get("frames").and_then(Json::as_u64);
        let errors = e.get("errors").and_then(Json::as_u64);
        assert_eq!(
            addr,
            por_alvo[i].0.to_string(),
            "a entrada {i} tem de nomear o SEU endereço — sem nome, cinco robôs mandam \
             procurar em cinco sítios.\n{linha}"
        );
        assert_eq!(
            (frames, errors),
            (Some(esperado.0), Some(esperado.1)),
            "no {i} ({addr}): esperava (frames, errors)={esperado:?}.\
             \n  Se vierem (2, 1) nos três, o relatório está a repetir o AGREGADO por nó — \
             que é exactamente o que o §8 existe para impedir.\
             \n  Se o do meio contou um frame, este SO nao recusou o broadcast e o teste \
             deixou de exercitar a falha: troque o alvo, nao a asserção.\
             \n  no fio: {linha}"
        );
    }
}

/// **Sem saída, a lista é vazia — nunca zeros fabricados.**
///
/// A alternativa D que o ADR-0029 §8 rejeita é reportar um total somado; a que este teste
/// fecha é a irmã dela — inventar uma entrada com `frames: 0` para um nó que não existe.
/// Ausência de saída e uma saída parada são factos diferentes.
#[test]
fn sem_saida_o_status_nao_inventa_nos() {
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));

    let linha = status_no_fio(&cp, "sem-saida");
    let j = parse(&linha).expect("JSON valido");

    let Some(Json::Arr(saidas)) = j.get("outputs") else {
        panic!("`outputs` tem de existir sempre, mesmo vazio — um campo ausente e um campo \
                vazio dizem coisas diferentes ao frontend.\n{linha}");
    };
    assert!(saidas.is_empty(), "sem palco aberto nao ha nos: {linha}");
}
