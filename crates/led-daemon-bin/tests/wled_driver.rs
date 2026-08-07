//! GS4.3 — **driver WLED**: fragmentação, universos e ordem de pixels, medidos no fio.
//!
//! Cada afirmação aqui é feita contra **bytes recebidos num socket UDP**, e comparada com o
//! que o [`HardwareProfile`] prevê. Não é um mock: é o mesmo `OutputManager` que o daemon usa,
//! a falar com um socket de loopback que conta e disseca o que chega.
//!
//! # O que isto NÃO prova
//!
//! Que um WLED real aceita os bytes. Isso é a etapa 4 do runbook e precisa do ESP32-POE, que
//! **não está na rede**. O que fica provado é que aquilo que sai do daemon tem a forma que o
//! profile declara — que é a parte que se pode saber sem hardware, e que era exatamente onde
//! estava o defeito da ordem de canais encontrado nesta fatia.

use led_core::{ColorFormat, PixelColor, RgbOrder};
use led_daemon_bin::{OutputConfig, OutputManager};
use led_hardware_profile::{HardwareProfile, HardwareRegistry, Protocol, Transport};
use std::net::UdpSocket;
use std::time::Duration;

/// Cabeçalho DDP, em bytes — o payload começa logo a seguir.
const DDP_HEADER: usize = 10;
/// Cabeçalho ArtDmx até ao primeiro canal DMX.
const ARTDMX_HEADER: usize = 18;
/// Offset do campo `SubUni`/`Net` (port-address, little-endian) num ArtDmx.
const ARTDMX_UNIVERSE_OFF: usize = 14;

fn perfil(nome: &str) -> HardwareProfile {
    HardwareRegistry::with_builtin().profile(nome).unwrap_or_else(|| panic!("preset {nome}"))
}

fn socket() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
    s
}

/// Envia **um** frame e devolve todos os datagramas que chegaram.
fn um_frame(cfg: OutputConfig, sock: &UdpSocket, pixels: Vec<PixelColor>) -> Vec<Vec<u8>> {
    let om = OutputManager::open(cfg).expect("abrir saída");
    om.send(&led_core::LogicalFrame::new(pixels, 0)).expect("enviar");
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = sock.recv(&mut buf) {
        out.push(buf[..n].to_vec());
    }
    out
}

fn branco(n: usize) -> Vec<PixelColor> {
    vec![PixelColor { r: 200, g: 100, b: 50 }; n]
}

// ── Fragmentação ────────────────────────────────────────────────────────────

/// **O profile prevê a fragmentação, e o fio confirma-a.** Se algum dia divergirem, é aqui
/// que se sabe — em vez de num rig com metade da fita apagada.
#[test]
fn o_numero_de_datagramas_bate_com_o_que_o_profile_preve() {
    for (preset, px, esperado) in [
        ("esp32-poe-wled-ddp", 720usize, 2u32),   // 487 + 233
        ("esp32-poe-wled-ddp", 487, 1),           // exatamente um datagrama cheio
        ("esp32-poe-wled-ddp", 488, 2),           // um pixel a mais parte em dois
        ("esp32-devkit-wled-artnet", 720, 5),     // ceil(720 / 170)
        ("esp32-devkit-wled-artnet", 170, 1),
    ] {
        let p = perfil(preset);
        let sock = socket();
        let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), px, 1).unwrap();

        let previsto = cfg.datagrams_per_frame(&p.transport);
        assert_eq!(previsto, esperado, "{preset}/{px}px: previsão do profile");

        let recebidos = um_frame(cfg, &sock, branco(px));
        assert_eq!(
            recebidos.len() as u32, esperado,
            "{preset}/{px}px: o fio recebeu {} datagramas, o profile previa {esperado}",
            recebidos.len()
        );
    }
}

/// A fragmentação **não corta um pixel ao meio** — cada datagrama traz múltiplos inteiros de
/// canais. Um meio pixel no fio seria a fita a partir das cores a partir dali.
#[test]
fn nenhum_datagrama_parte_um_pixel_ao_meio() {
    let p = perfil("esp32-poe-wled-ddp");
    let sock = socket();
    let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), 720, 1).unwrap();
    let canais = cfg.color.channels();
    for (i, d) in um_frame(cfg, &sock, branco(720)).iter().enumerate() {
        let payload = d.len() - DDP_HEADER;
        assert_eq!(payload % canais, 0, "datagrama {i}: {payload} bytes não é múltiplo de {canais}");
    }
}

// ── Ordem dos pixels ────────────────────────────────────────────────────────

/// **A ordem de canais vem do profile.**
///
/// Este é o teste que apanha o defeito corrigido nesta fatia: antes do GS4.3 o
/// `OutputManager` construía o layout com `RgbOrder::Rgb` **fixo no código**, ignorando o
/// `ColorFormat::Rgb(Grb)` que os presets WLED declaram desde o ADR-0018. Vermelho puro saía
/// como vermelho no fio, e um nó GRB acendia-o **verde**.
#[test]
fn vermelho_puro_sai_na_ordem_que_o_no_declara() {
    let p = perfil("esp32-poe-wled-ddp");
    assert_eq!(p.capabilities.color, ColorFormat::Rgb(RgbOrder::Grb), "o preset é GRB");

    let sock = socket();
    let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), 4, 1).unwrap();
    assert_eq!(cfg.rgb_order(), RgbOrder::Grb, "a config tem de herdar a ordem do profile");

    let vermelho = vec![PixelColor { r: 255, g: 0, b: 0 }; 4];
    let d = um_frame(cfg, &sock, vermelho).remove(0);
    let px0 = &d[DDP_HEADER..DDP_HEADER + 3];
    assert_eq!(
        px0,
        &[0, 255, 0],
        "GRB: vermelho lógico tem de sair como (g=0, r=255, b=0); saiu {px0:?}"
    );
}

/// **Controle negativo da ordem.** Um profile que declare RGB tem de produzir bytes
/// diferentes do que declara GRB — senão o teste de cima passaria com a ordem ignorada.
#[test]
fn um_no_rgb_e_um_no_grb_nao_podem_produzir_os_mesmos_bytes() {
    let mut rgb = perfil("esp32-poe-wled-ddp");
    rgb.capabilities.color = ColorFormat::Rgb(RgbOrder::Rgb);
    let grb = perfil("esp32-poe-wled-ddp");

    let mut saidas = Vec::new();
    for p in [&rgb, &grb] {
        let sock = socket();
        let cfg = OutputConfig::from_profile(p, sock.local_addr().unwrap(), 4, 1).unwrap();
        let d = um_frame(cfg, &sock, vec![PixelColor { r: 255, g: 0, b: 0 }; 4]).remove(0);
        saidas.push(d[DDP_HEADER..DDP_HEADER + 3].to_vec());
    }
    assert_eq!(saidas[0], vec![255, 0, 0], "RGB");
    assert_eq!(saidas[1], vec![0, 255, 0], "GRB");
    assert_ne!(saidas[0], saidas[1], "se fossem iguais, a ordem não estaria a ser aplicada");
}

// ── Universos ───────────────────────────────────────────────────────────────

/// Os universos saem **numerados a partir do primeiro declarado pela instância**, sem saltos
/// e sem repetições — 720 px em 5 universos consecutivos.
#[test]
fn os_universos_do_artnet_sao_consecutivos_a_partir_do_primeiro() {
    let p = perfil("esp32-devkit-wled-artnet");
    let sock = socket();
    let primeiro = 3u16;
    let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), 720, primeiro).unwrap();

    let ds = um_frame(cfg, &sock, branco(720));
    assert_eq!(ds.len(), 5);
    let mut universos: Vec<u16> = ds
        .iter()
        .map(|d| u16::from_le_bytes([d[ARTDMX_UNIVERSE_OFF], d[ARTDMX_UNIVERSE_OFF + 1]]))
        .collect();
    universos.sort_unstable();
    assert_eq!(universos, vec![3, 4, 5, 6, 7], "consecutivos a partir de {primeiro}");
}

/// **Nenhum datagrama Art-Net excede um universo.** O MTU não é o que prende aqui — o
/// universo é — e é isso que o profile diz.
#[test]
fn nenhum_datagrama_artnet_leva_mais_do_que_um_universo() {
    let p = perfil("esp32-devkit-wled-artnet");
    let sock = socket();
    let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), 720, 1).unwrap();
    let max_canais = p.limits.pixels_per_universe as usize * cfg.color.channels();

    for (i, d) in um_frame(cfg, &sock, branco(720)).iter().enumerate() {
        let canais = d.len() - ARTDMX_HEADER;
        assert!(canais <= 512, "datagrama {i}: {canais} canais excede um universo DMX");
        assert!(canais <= max_canais.max(1) + 2, "datagrama {i}: {canais} canais");
    }
}

/// sACN parte no mesmo número de universos que Art-Net — é a mesma regra de 512 canais.
#[test]
fn sacn_usa_o_mesmo_numero_de_universos_que_artnet() {
    let mut p = perfil("esp32-devkit-wled-artnet");
    p.capabilities.protocol = Protocol::Sacn;
    let sock = socket();
    let cfg = OutputConfig::from_profile(&p, sock.local_addr().unwrap(), 720, 1).unwrap();
    assert_eq!(cfg.datagrams_per_frame(&p.transport), 5);
    assert_eq!(um_frame(cfg, &sock, branco(720)).len(), 5);
}

// ── Limites declarados ──────────────────────────────────────────────────────

/// Um show maior do que o nó declara suportar é **recusado na construção**, não descoberto
/// no palco com metade da fita apagada.
#[test]
fn um_show_maior_que_o_no_e_recusado() {
    let p = perfil("esp32-poe-wled-ddp");
    let excesso = p.limits.max_pixels as usize + 1;
    let e = OutputConfig::from_profile(&p, "127.0.0.1:1".parse().unwrap(), excesso, 1).unwrap_err();
    assert!(e.contains("máximo") || e.contains("maximo"), "{e}");
}

/// Um profile com heartbeat fora do teto do `LUMYX_GOSL` **não abre saída nenhuma**.
#[test]
fn um_heartbeat_inseguro_impede_a_saida() {
    let mut p = perfil("esp32-poe-wled-ddp");
    p.transport = Transport { mtu_bytes: 1_500, heartbeat_ms: 3_000 };
    let e = OutputConfig::from_profile(&p, "127.0.0.1:1".parse().unwrap(), 100, 1).unwrap_err();
    assert!(e.contains("2400"), "a mensagem tem de nomear o teto: {e}");
}

/// **Os dois construtores produzem a mesma coisa** onde os dados coincidem — a prova de que
/// `from_profile` não abriu um segundo caminho para o fio.
#[test]
fn from_profile_e_parse_produzem_a_mesma_saida_quando_os_dados_coincidem() {
    let mut p = perfil("esp32-poe-wled-ddp");
    p.capabilities.color = ColorFormat::Rgb(RgbOrder::Rgb); // igualar ao omisso do `parse`
    let addr = "127.0.0.1:4048".parse().unwrap();
    let a = OutputConfig::from_profile(&p, addr, 720, 1).unwrap();
    let b = OutputConfig::parse("ddp://127.0.0.1:4048", 720, 1).unwrap();
    assert_eq!(a, b, "mesmos dados ⇒ mesma configuração, seja qual for a porta de entrada");
}
