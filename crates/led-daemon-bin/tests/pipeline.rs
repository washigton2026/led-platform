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
use led_daemon_bin::{
    descriptor_from_path, profile_by_name, FrameSource, OutputConfig, OutputManager,
};
use led_daemon::ShowId;
use led_show_recorder::{ShowRecord, ShowWriter};
use std::net::UdpSocket;

/// O preset do catálogo que fala cada protocolo. **Nenhum teste escolhe protocolo à mão** —
/// escolhe hardware, e o protocolo vem com ele.
fn perfil_de(proto: &str) -> led_hardware_profile::HardwareProfile {
    profile_by_name(match proto {
        "ddp" => "esp32-poe-wled-ddp",
        "artnet" => "esp32-devkit-wled-artnet",
        "sacn" => "falcon-f16v3-sacn",
        outro => panic!("sem preset para {outro}"),
    })
    .unwrap()
}

fn escrever(nome: &str, frames: &[(u64, u8)], px: u32) -> String {
    // **O pid no nome não é decoração.** Com um nome fixo, duas execuções concorrentes de
    // `cargo test --workspace` são dois processos a escrever o MESMO ficheiro em
    // `temp_dir()`: uma trunca o que a outra está a ler, e o desfecho é `UnexpectedEof` a
    // meio do `.lumyx` — uma falha que parece regressão do loader e não é. Aconteceu, e
    // está registado na FASE C.
    //
    // O discriminante certo é o **processo**, porque é essa a unidade que colide: dois
    // `cargo test` são dois pids. Dentro de uma execução os chamadores já usam nomes
    // distintos entre si, portanto o pid é suficiente e não esconde nada.
    //
    // Reutiliza o padrão que o repositório já usa para o mesmo fim — `sse_reconnect.rs:108`,
    // `estado_por_alvo.rs:32`, `e2e_output.rs:418` — em vez de acrescentar `tempfile`, que
    // seria uma segunda maneira de resolver um problema já resolvido.
    let path = std::env::temp_dir().join(format!("{}-{nome}", std::process::id()));
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

        // Art-Net e sACN exigem `@UNIVERSO`; o DDP recusa-o (ADR-0029 §7). O `0` é o valor
        // que a bancada de 2026-07-23 confirmou alinhado com o `dmx.uni` do WLED.
        let perfil = perfil_de(proto);
        // Art-Net aceita `0`; o sACN **não** — o E1.31 começa em 1 (ADR-0029 §7.1).
        let spec = match proto {
            "ddp" => addr.to_string(),
            "sacn" => format!("{addr}@1"),
            _ => format!("{addr}@0"),
        };
        let om = OutputManager::open(
            OutputConfig::resolve(&perfil, &spec, px as usize).unwrap(),
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
        OutputConfig::resolve(
            &perfil_de("ddp"),
            &sock.local_addr().unwrap().to_string(),
            px as usize,
        )
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
