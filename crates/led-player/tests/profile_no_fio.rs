//! **O critério C do TD-019, do lado do `led-player`.**
//!
//! Que falsidade este teste impede? Esta: *«o `led-player` honra o `HardwareProfile`»* —
//! afirmada hoje só por leitura de código, e por mais nada. A medição que motivou este
//! ficheiro: mutar o **dono** do endereçamento (`led_hardware_profile::compile_layout_de`)
//! reprova o `led-daemon-bin`, e deixa o `led-player` em **18 passed · 0 failed**. O ramo
//! `--profile` vive no `main.rs`, que é um alvo binário com **zero** testes — e um alvo com
//! zero testes não pode reprovar. Alguém podia trocar `main.rs` de volta para o
//! `linear_assignments` — reintroduzindo o TD-019 inteiro — e o workspace ficava verde.
//!
//! Este teste corre o **binário**, não uma função: é a única forma de cobrir um ramo de
//! `main.rs` sem o refactorizar. Produção fica intocada.
//!
//! ## Porque é o caminho `--artnet`, e não o `--ddp`
//!
//! Medido, não escolhido por gosto: o arm DDP do `main.rs` usa **só** `profile_color` e
//! **não consome o `CompiledLayout`** — o DDP contorna o HAL por desenho (ADR-0003) e
//! endereça por byte, sem universos. Logo o *endereçamento* do critério C só é observável
//! onde o layout é consumido: `Hal::new(layout, …)`, no arm Art-Net.
//!
//! ## O discriminante, e porque não é o `pixels_per_universe`
//!
//! **Todos** os presets Art-Net do catálogo declaram `pixels_per_universe: 170`, que é
//! exactamente a constante escrita à mão no ramo de fallback. Nesse eixo, honrar o profile e
//! ignorá-lo são **indistinguíveis** — e uma asserção sobre ele seria falso-verde. O que
//! discrimina é o resto do que o profile declara:
//!
//! | pixel lógico `(200,100,50)` | byte 0 | byte 1 | byte 2 |
//! |---|---|---|---|
//! | profile honrado — GRB + gamma 2.2 | `lut(100)` | `lut(200)` | `lut(50)` |
//! | fallback — RGB, calibração identidade | `200` | `100` | `50` |
//!
//! Os três bytes diferem. A cor lógica tem os três canais **distintos** de propósito: com
//! `(255,255,255)` a ordem seria invisível e o teste passaria sem exercitar nada.

use led_core::PixelColor;
use led_show_recorder::{finalise_seekable, ShowRecord, ShowWriter};
use std::net::UdpSocket;
use std::process::Command;
use std::time::Duration;

/// O nó de bancada real do rig (ESP32 DevKit V1 + WLED), que declara `ArtNet` — portanto a
/// configuração é **coerente**: não se está a forçar um preset sobre um protocolo que ele
/// não declara só para obter um número mais bonito.
const PRESET: &str = "esp32-devkit-wled-artnet";

/// 300 px a 170/universo dão **dois** universos, com o segundo parcial. Não discrimina o
/// `ppu` (ver acima), mas fixa a forma do endereçamento: se alguém partir a aritmética de
/// universo no dono, os comprimentos deixam de bater.
const PX: usize = 300;
const PRIMEIRO_UNIVERSO: u16 = 1;

/// Três canais distintos — é isso que torna a ORDEM observável.
const LOGICA: PixelColor = PixelColor { r: 200, g: 100, b: 50 };

/// `lut[i] = ((i/255)^gamma · brightness · 255 + 0.5) as u8`, com o `gamma: 2.2` e o
/// `brightness: 1.0` que o preset declara (`calibration.rs:57-58`).
///
/// Os valores são **previstos**, não colhidos de uma execução: `lut(100) = 33`,
/// `lut(200) = 149`, `lut(50) = 7`. Em ordem **GRB** o byte 0 leva o verde.
const ESPERADO: [u8; 3] = [33, 149, 7];

fn escrever_show(caminho: &std::path::Path) {
    let f = std::fs::File::create(caminho).expect("criar .lumyx");
    let mut w = ShowWriter::new(f, PX as u32).expect("cabeçalho .lumyx");
    for t in 0..2u64 {
        w.write_frame(&ShowRecord {
            timestamp_ms: t * 25,
            pixels: vec![LOGICA; PX],
            audio: None,
        })
        .expect("escrever quadro");
    }
    finalise_seekable(&mut w).expect("finalizar .lumyx");
}

#[test]
fn o_player_com_profile_poe_no_fio_o_formato_do_profile_e_nao_o_do_fallback() {
    let caminho =
        std::env::temp_dir().join(format!("lumyx-td019-c-{}.lumyx", std::process::id()));
    escrever_show(&caminho);

    let sock = UdpSocket::bind("127.0.0.1:0").expect("socket de captura");
    sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let porta = sock.local_addr().unwrap().port();

    let saida = Command::new(env!("CARGO_BIN_EXE_led-player"))
        .arg(&caminho)
        .arg("--profile")
        .arg(PRESET)
        .arg("--artnet")
        .arg(format!("127.0.0.1:{porta}"))
        .arg("--first-universe")
        .arg(PRIMEIRO_UNIVERSO.to_string())
        .arg("--speed")
        .arg("max")
        .output()
        .expect("executar led-player");

    let _ = std::fs::remove_file(&caminho);

    // A premissa tem de reprovar em voz alta. Um binário que não arranca produz zero
    // datagramas, e zero datagramas não podem ser lidos como «nada a verificar» (KB-012).
    assert!(
        saida.status.success(),
        "o led-player não completou — este teste não exercitou nada.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&saida.stdout),
        String::from_utf8_lossy(&saida.stderr)
    );

    let mut dg: Vec<Vec<u8>> = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(n) = sock.recv(&mut buf) {
        dg.push(buf[..n].to_vec());
    }
    assert!(
        !dg.is_empty(),
        "nenhum datagrama Art-Net chegou ao socket — o teste não mediu o fio.\nstdout:\n{}",
        String::from_utf8_lossy(&saida.stdout)
    );

    // Reutiliza o parser de produção em vez de escrever um segundo: um parser próprio no
    // teste podia divergir do que o `ArtNetDevice` emite e ninguém reparava.
    let mut vistos: Vec<u16> = Vec::new();
    let mut universos_com_pixel_zero = 0usize;

    for (i, d) in dg.iter().enumerate() {
        let (universo, _seq, dados) =
            led_protocols::parse_art_dmx(d).unwrap_or_else(|| panic!("datagrama {i} não é ArtDmx"));

        if !vistos.contains(&universo) {
            vistos.push(universo);
        }

        // O comprimento NÃO discrimina, e foi medido antes de ser escrito: o universo vai
        // sempre **preenchido** a 512 canais, tal como o sACN. Uma asserção de comprimento
        // é cega a tudo o que interessa aqui; fica como premissa, não como prova.
        assert_eq!(dados.len(), 512, "datagrama {i}, universo {universo}: universo preenchido");

        // O que discrimina é o PERÍODO do padrão e a FRONTEIRA do preenchimento. 300 px a
        // 170/universo: 170 pixels no primeiro universo, 130 no segundo — e o resto a zero.
        // Se a aritmética de universo do dono se partir, a fronteira muda e isto reprova.
        let px_neste = if universo == PRIMEIRO_UNIVERSO { 170 } else { PX - 170 };

        for p in 0..px_neste {
            let o = p * 3;
            assert_eq!(
                &dados[o..o + 3],
                &ESPERADO[..],
                "datagrama {i}, universo {universo}, pixel {p}: esperava {ESPERADO:?} \
                 (GRB do profile + gamma 2.2), veio {:?}. Em RGB sem calibração sairia \
                 [200, 100, 50] — o fallback escrito à mão, que é o TD-019.",
                &dados[o..o + 3]
            );
        }
        assert!(
            dados[px_neste * 3..].iter().all(|&b| b == 0),
            "datagrama {i}, universo {universo}: o preenchimento depois do pixel {} devia \
             ser zero — a fronteira do universo mudou",
            px_neste - 1
        );

        if universo == PRIMEIRO_UNIVERSO {
            universos_com_pixel_zero += 1;
        }
    }

    vistos.sort_unstable();
    assert_eq!(
        vistos,
        vec![PRIMEIRO_UNIVERSO, PRIMEIRO_UNIVERSO + 1],
        "300 px a 170/universo são dois universos consecutivos a partir de {PRIMEIRO_UNIVERSO}"
    );

    // Sem isto, um universo de arranque que nunca chegasse deixaria a asserção do
    // discriminante por executar, e o teste passava sem medir a cor.
    assert!(
        universos_com_pixel_zero > 0,
        "nenhum datagrama do universo {PRIMEIRO_UNIVERSO} chegou: a cor não foi medida"
    );
}
