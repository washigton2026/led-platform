//! GS4.2 — o pipeline completo: `.lumyx` → loader → source → OutputManager → **fio**.
//!
//! ## O que este teste prova, e o que NÃO prova
//!
//! **Prova:** que um quadro gravado num `.lumyx` real atravessa o daemon e sai como
//! datagrama UDP — bytes num socket, não um mock. É a resposta a *"o primeiro frame sai do
//! daemon"*.
//!
//! **Não prova:** que hardware recebeu, mostrou ou acendeu. Isso é GS4.3–GS4.7 e exige um
//! ESP32-POE na rede. Confundir as duas coisas seria exatamente o falso-verde que o KB-012
//! descreve — um teste verde que não exercita a propriedade que se afirma.

use led_core::PixelColor;
use led_daemon_bin::{descriptor_from_path, FrameSource, OutputConfig, OutputManager};
use led_daemon::ShowId;
use led_show_recorder::{ShowRecord, ShowWriter};
use std::net::UdpSocket;

fn escrever(nome: &str, frames: &[(u64, u8)], px: u32) -> String {
    let path = std::env::temp_dir().join(nome);
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, px).unwrap();
    for &(ts, v) in frames {
        w.write_frame(&ShowRecord {
            timestamp_ms: ts,
            pixels: vec![PixelColor { r: v, g: 0, b: 0 }; px as usize],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn do_lumyx_ate_ao_fio_nos_tres_protocolos() {
    let px = 8u32;
    let path = escrever("gs42.lumyx", &[(0, 11), (100, 22), (200, 33)], px);

    // O descritor sai do mesmo loader que o daemon usa.
    let desc = descriptor_from_path(&path, ShowId(1)).unwrap();
    assert_eq!(desc.frame_count, 3);
    assert_eq!(desc.duration_ms, 200);

    for proto in ["ddp", "artnet", "sacn"] {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let addr = sock.local_addr().unwrap();

        let om = OutputManager::open(
            OutputConfig::parse(&format!("{proto}://{addr}"), px as usize, 1).unwrap(),
        )
        .unwrap();
        let mut src = FrameSource::open(&path).unwrap();

        // Percorre as três posições do show, como o laço faria.
        for (pos, esperado) in [(0u64, 11u8), (150, 22), (999, 33)] {
            let f = src.frame_at(pos).unwrap().expect("o show tem quadros");
            assert_eq!(f.pixels[0].r, esperado, "{proto}: quadro errado em t={pos}");
            om.send(&f).unwrap_or_else(|e| panic!("{proto}: {e:?}"));

            let mut buf = [0u8; 2048];
            let n = sock.recv(&mut buf).unwrap_or_else(|e| panic!("{proto}: nada no fio: {e}"));
            assert!(n > 0, "{proto}: datagrama vazio");
        }
        assert_eq!(om.stats().frames(), 3, "{proto}");
        assert_eq!(om.stats().errors(), 0, "{proto}");
    }
    let _ = std::fs::remove_file(path);
}

/// Seek para trás atravessa o pipeline inteiro — reabrindo o ficheiro, sem quebrar a saída.
#[test]
fn seek_para_tras_atravessa_o_pipeline() {
    let px = 4u32;
    let path = escrever("gs42b.lumyx", &[(0, 1), (100, 2), (200, 3)], px);
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
    let om = OutputManager::open(
        OutputConfig::parse(&format!("ddp://{}", sock.local_addr().unwrap()), px as usize, 1)
            .unwrap(),
    )
    .unwrap();
    let mut src = FrameSource::open(&path).unwrap();

    for (pos, esperado) in [(200u64, 3u8), (0, 1), (100, 2)] {
        let f = src.frame_at(pos).unwrap().unwrap();
        assert_eq!(f.pixels[0].r, esperado, "t={pos}");
        om.send(&f).unwrap();
    }
    assert_eq!(om.stats().frames(), 3);
    let _ = std::fs::remove_file(path);
}
