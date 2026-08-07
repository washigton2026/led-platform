//! GS4.3 — descoberta ArtPoll **sobre sockets UDP reais**, sem mocks.
//!
//! Um controlador de mentira responde na loopback com um `ArtPollReply` construído pelo
//! próprio `led-protocols`, e o lado que descobre analisa os bytes que chegaram pelo fio. O
//! que fica provado é o **formato e a lógica de presença**; o que não fica provado é que um
//! WLED real responde — isso é a etapa 3 do runbook e precisa do ESP32-POE.

use led_protocols::{build_art_poll, build_art_poll_reply, parse_art_poll_reply, presence};
use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

/// Um controlador falso: recebe um ArtPoll e responde como o hardware responderia.
fn responder(sock: &UdpSocket, ip: Ipv4Addr, nome: &str) -> std::io::Result<()> {
    let mut buf = [0u8; 1024];
    let (n, origem) = sock.recv_from(&mut buf)?;
    // Um ArtPoll tem de ser reconhecível pelo cabeçalho `Art-Net\0`.
    assert!(n >= 12 && &buf[..8] == b"Art-Net\0", "não era um ArtPoll: {:?}", &buf[..n.min(12)]);
    let mut resp = [0u8; 239];
    build_art_poll_reply(&mut resp, ip, &[0], nome);
    sock.send_to(&resp, origem)?;
    Ok(())
}

/// **Ida e volta reais**: ArtPoll sai, ArtPollReply volta, e a presença é decidida sobre os
/// bytes recebidos.
#[test]
fn um_artpoll_no_fio_traz_de_volta_um_no_presente() {
    let controlador = UdpSocket::bind("127.0.0.1:0").unwrap();
    controlador.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let alvo = controlador.local_addr().unwrap();
    let ip_declarado: Ipv4Addr = "192.168.2.156".parse().unwrap();

    let t = std::thread::spawn(move || responder(&controlador, ip_declarado, "wled-robo-1"));

    let descobridor = UdpSocket::bind("127.0.0.1:0").unwrap();
    descobridor.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut poll = [0u8; led_protocols::ART_POLL_LEN];
    build_art_poll(&mut poll);
    descobridor.send_to(&poll, alvo).unwrap();

    let mut buf = [0u8; 1024];
    let n = descobridor.recv(&mut buf).expect("o controlador tem de responder");
    let reply = parse_art_poll_reply(&buf[..n]).expect("ArtPollReply válido no fio");
    assert_eq!(reply.ip, ip_declarado, "o reply tem de trazer o IP que o nó declara");

    let r = presence(&[ip_declarado], &[reply]);
    assert_eq!(r.responded, vec![ip_declarado]);
    assert!(r.missing.is_empty());
    t.join().unwrap().unwrap();
}

/// **Controle negativo: silêncio é ausência.** Sem responder ninguém, o nó tem de aparecer
/// como `missing` — se aparecesse como presente, o gate do palco escuro não valeria nada.
#[test]
fn um_no_que_nao_responde_fica_ausente() {
    let esperado: Ipv4Addr = "192.168.2.157".parse().unwrap();
    let r = presence(&[esperado], &[]);
    assert!(r.responded.is_empty());
    assert_eq!(r.missing, vec![esperado], "silêncio = ausente, sempre");
}

/// **Uma resposta de outro nó não pode tapar um ausente.** É o cenário real de um rig
/// parcialmente ligado: o nó 1 responde, o nó 2 está sem corrente, e um contador ingénuo
/// diria "recebi resposta, está tudo bem".
#[test]
fn a_resposta_de_um_no_nao_mascara_o_silencio_de_outro() {
    let controlador = UdpSocket::bind("127.0.0.1:0").unwrap();
    controlador.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let alvo = controlador.local_addr().unwrap();
    let vivo: Ipv4Addr = "192.168.2.156".parse().unwrap();
    let morto: Ipv4Addr = "192.168.2.157".parse().unwrap();

    let t = std::thread::spawn(move || responder(&controlador, vivo, "wled-robo-1"));

    let d = UdpSocket::bind("127.0.0.1:0").unwrap();
    d.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut poll = [0u8; led_protocols::ART_POLL_LEN];
    build_art_poll(&mut poll);
    d.send_to(&poll, alvo).unwrap();
    let mut buf = [0u8; 1024];
    let n = d.recv(&mut buf).unwrap();
    let reply = parse_art_poll_reply(&buf[..n]).unwrap();

    let r = presence(&[vivo, morto], &[reply]);
    assert_eq!(r.responded, vec![vivo]);
    assert_eq!(r.missing, vec![morto], "o nó 2 continua ausente apesar da resposta do nó 1");
    t.join().unwrap().unwrap();
}

/// Lixo no fio **não vira um nó descoberto**. Um datagrama qualquer na porta não pode ser
/// lido como um controlador presente.
#[test]
fn lixo_no_fio_nao_e_um_controlador() {
    for lixo in [&b""[..], &b"ola"[..], &[0xFFu8; 239][..], &b"Art-Net\0mas o resto e mentira"[..]] {
        assert!(parse_art_poll_reply(lixo).is_none(), "aceitou lixo: {lixo:?}");
    }
}
