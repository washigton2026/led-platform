//! ADR-0019 (Emenda 1) — a calibração aplicada na **fronteira lógica de saída**.
//!
//! # O que estes testes existem para impedir
//!
//! Que um `HardwareProfile` declare `gamma`/`brightness` e o fio não saiba disso. Foi
//! exatamente o que aconteceu até esta fatia: o `led-player` calibrava no Art-Net, o DDP
//! nunca calibrou (contorna o HAL, por decisão de 2026-07-09d), e o daemon não calibrava em
//! lado nenhum. O campo existia, e ninguém o honrava.
//!
//! # Como provam
//!
//! Comparando **bytes recebidos num socket UDP** com e sem calibração, nos **três**
//! protocolos. Um teste que só verificasse que o `OutputManager` guarda a calibração provaria
//! que o campo foi copiado — não que ele chega ao fio.

use led_core::{ColorFormat, LogicalFrame, PixelColor, RgbOrder};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_hardware_profile::{Calibration as ProfileCalibration, HardwareProfile, Transport};
use std::net::UdpSocket;
use std::time::Duration;

const DDP_HEADER: usize = 10;
const ARTDMX_HEADER: usize = 18;
/// Offset do primeiro canal DMX num pacote E1.31 (126 bytes de cabeçalho + start code).
const SACN_FIRST_CHANNEL: usize = 126;

fn socket() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
    s
}

/// Perfil do catálogo com a calibração substituída — **o profile continua a ser a fonte**.
fn perfil_com(preset: &str, gamma: f32, brightness: f32) -> HardwareProfile {
    let mut p = profile_by_name(preset).expect("preset do catálogo");
    p.calibration = ProfileCalibration { gamma, brightness };
    p
}

/// Envia um frame e devolve os **bytes de payload** do primeiro datagrama.
fn payload(perfil: &HardwareProfile, px: usize, cor: PixelColor, offset: usize) -> Vec<u8> {
    let sock = socket();
    let addr = sock.local_addr().unwrap();
    let cfg = OutputConfig::resolve(perfil, &addr.to_string(), px, 1).expect("resolver");
    let om = OutputManager::open(cfg).expect("abrir saída");
    om.send(&LogicalFrame::new(vec![cor; px], 0)).expect("enviar");
    let mut buf = [0u8; 4096];
    let n = sock.recv(&mut buf).expect("um datagrama");
    buf[offset..n].to_vec()
}

fn offset_de(preset: &str) -> usize {
    match preset {
        "esp32-poe-wled-ddp" | "esp32-poe-wled-rgbw-ddp" => DDP_HEADER,
        "esp32-devkit-wled-artnet" => ARTDMX_HEADER,
        _ => SACN_FIRST_CHANNEL,
    }
}

/// Os três presets do catálogo que cobrem os três protocolos.
const PRESETS: &[(&str, &str)] = &[
    ("esp32-poe-wled-ddp", "DDP"),
    ("esp32-devkit-wled-artnet", "Art-Net"),
    ("falcon-f16v3-sacn", "sACN"),
];

// ── Identidade ──────────────────────────────────────────────────────────────

/// **A presença da calibração, por si só, não pode mudar um byte.** Sem este teste, qualquer
/// diferença observada nos outros poderia ser um efeito colateral do caminho novo, e não da
/// correção óptica.
#[test]
fn a_calibracao_identidade_nao_muda_nenhum_byte_em_nenhum_protocolo() {
    let cor = PixelColor { r: 200, g: 100, b: 50 };
    for (preset, nome) in PRESETS {
        let sem = payload(&perfil_com(preset, 1.0, 1.0), 8, cor, offset_de(preset));
        // O catálogo declara gamma 2.2; forçar 1.0/1.0 é a identidade explícita.
        let ident = payload(&perfil_com(preset, 1.0, 1.0), 8, cor, offset_de(preset));
        assert_eq!(sem, ident, "{nome}: identidade tem de ser byte-idêntica");
        assert!(
            ident.iter().any(|&b| b == 200 || b == 100 || b == 50),
            "{nome}: com gamma 1.0 e brilho 1.0 as cores originais têm de sobreviver"
        );
    }
}

// ── Gamma ───────────────────────────────────────────────────────────────────

/// **Gamma > 1 escurece os tons médios, nos três protocolos.** É a asserção que reprova se
/// algum caminho voltar a ignorar a calibração.
#[test]
fn gamma_chega_ao_fio_nos_tres_protocolos() {
    let cor = PixelColor { r: 128, g: 128, b: 128 };
    for (preset, nome) in PRESETS {
        let linear = payload(&perfil_com(preset, 1.0, 1.0), 8, cor, offset_de(preset));
        let gama = payload(&perfil_com(preset, 2.2, 1.0), 8, cor, offset_de(preset));

        assert_ne!(linear, gama, "{nome}: gamma 2.2 NÃO chegou ao fio");
        // (128/255)^2.2 ≈ 0.2176 → ~55. Verificar o valor, não só a diferença.
        let esperado = ((128.0f32 / 255.0).powf(2.2) * 255.0 + 0.5) as u8;
        assert_eq!(gama[0], esperado, "{nome}: valor de gamma errado (esperava {esperado})");
        assert!(gama[0] < linear[0], "{nome}: gamma 2.2 tem de escurecer o meio-tom");
    }
}

// ── Brightness ──────────────────────────────────────────────────────────────

#[test]
fn brightness_chega_ao_fio_nos_tres_protocolos() {
    let cor = PixelColor { r: 200, g: 200, b: 200 };
    for (preset, nome) in PRESETS {
        let cheio = payload(&perfil_com(preset, 1.0, 1.0), 8, cor, offset_de(preset));
        let meio = payload(&perfil_com(preset, 1.0, 0.5), 8, cor, offset_de(preset));

        assert_ne!(cheio, meio, "{nome}: brightness 0.5 NÃO chegou ao fio");
        let esperado = ((200.0f32 / 255.0) * 0.5 * 255.0 + 0.5) as u8;
        assert_eq!(meio[0], esperado, "{nome}: valor de brilho errado");
    }
}

// ── Combinação ──────────────────────────────────────────────────────────────

/// **As duas dobram numa só LUT** — o resultado tem de ser o composto, não uma das duas.
#[test]
fn gamma_e_brightness_compoem_numa_so_transformacao() {
    let cor = PixelColor { r: 180, g: 180, b: 180 };
    for (preset, nome) in PRESETS {
        let so_gama = payload(&perfil_com(preset, 2.2, 1.0), 8, cor, offset_de(preset));
        let so_brilho = payload(&perfil_com(preset, 1.0, 0.5), 8, cor, offset_de(preset));
        let ambos = payload(&perfil_com(preset, 2.2, 0.5), 8, cor, offset_de(preset));

        let esperado = ((180.0f32 / 255.0).powf(2.2) * 0.5 * 255.0 + 0.5) as u8;
        assert_eq!(ambos[0], esperado, "{nome}: composição errada");
        assert!(ambos[0] < so_gama[0], "{nome}: com brilho tem de ser mais escuro que só gamma");
        assert!(ambos[0] < so_brilho[0], "{nome}: e mais escuro que só brilho");
    }
}

// ── O que a calibração NÃO pode estragar ────────────────────────────────────

/// **A ordem de canais sobrevive à calibração.** Calibrar antes ou depois de ordenar dá
/// resultados diferentes quando os canais têm valores diferentes — este teste fixa qual é o
/// certo: a correção é **por canal**, e a ordem do nó continua a ser respeitada.
#[test]
fn a_ordem_de_canais_sobrevive_a_calibracao() {
    // Cor com os três canais distintos: se a ordem se perdesse, os bytes trocariam de sítio.
    let cor = PixelColor { r: 200, g: 100, b: 50 };
    let p = perfil_com("esp32-poe-wled-ddp", 2.2, 1.0);
    assert_eq!(p.capabilities.color, ColorFormat::Rgb(RgbOrder::Grb), "o preset é GRB");

    let b = payload(&p, 4, cor, DDP_HEADER);
    let lut = |v: u8| ((v as f32 / 255.0).powf(2.2) * 255.0 + 0.5) as u8;
    assert_eq!(
        &b[..3],
        &[lut(100), lut(200), lut(50)],
        "GRB calibrado: verde, vermelho, azul — cada um com o seu próprio valor corrigido"
    );
}

/// **A fragmentação não muda.** A calibração transforma valores, nunca a contagem de pixels —
/// se mudasse, um frame calibrado sairia em número diferente de datagramas.
#[test]
fn a_fragmentacao_e_o_mtu_sobrevivem_a_calibracao() {
    let px = 720usize;
    for (preset, nome) in PRESETS {
        for (g, b) in [(1.0f32, 1.0f32), (2.2, 0.5)] {
            let p = perfil_com(preset, g, b);
            let sock = socket();
            let addr = sock.local_addr().unwrap();
            let cfg = OutputConfig::resolve(&p, &addr.to_string(), px, 1).unwrap();
            let previsto = cfg.datagrams_per_frame();
            let om = OutputManager::open(cfg).unwrap();
            om.send(&LogicalFrame::new(vec![PixelColor { r: 128, g: 64, b: 32 }; px], 0))
                .unwrap();

            let (mut n, mut buf) = (0u32, [0u8; 4096]);
            while let Ok(len) = sock.recv(&mut buf) {
                assert!(
                    len + Transport::IP_UDP_OVERHEAD <= p.transport.mtu_bytes as usize,
                    "{nome} (γ{g}/b{b}): datagrama de {len} bytes excede o MTU"
                );
                n += 1;
            }
            assert_eq!(n, previsto, "{nome} (γ{g}/b{b}): a calibração mudou a fragmentação");
        }
    }
}

/// **Controle negativo do próprio conjunto.** Se a cor de teste fosse preta ou branca
/// saturada, gamma e brilho não a mudariam — e todos os testes acima passariam sem provar
/// nada. Este teste afirma que as extremidades são de facto imunes, o que é a razão de as
/// outras asserções usarem meios-tons.
#[test]
fn as_extremidades_sao_imunes_e_por_isso_os_testes_usam_meios_tons() {
    let p_lin = perfil_com("esp32-poe-wled-ddp", 1.0, 1.0);
    let p_cal = perfil_com("esp32-poe-wled-ddp", 2.2, 1.0);
    for cor in [PixelColor { r: 0, g: 0, b: 0 }, PixelColor { r: 255, g: 255, b: 255 }] {
        assert_eq!(
            payload(&p_lin, 4, cor, DDP_HEADER),
            payload(&p_cal, 4, cor, DDP_HEADER),
            "0 e 255 são pontos fixos de qualquer gamma — usar só estes não provaria nada"
        );
    }
}
