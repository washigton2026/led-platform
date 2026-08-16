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

/// O preset do catálogo, com a **calibração neutralizada**.
///
/// Estes testes isolam fragmentação, universos e ordem de canais — e afirmam valores de byte
/// exatos. Os presets declaram gamma 2.2, que desde a Emenda 1 do ADR-0019 chega ao fio: sem
/// esta neutralização, `[200,100,50]` sairia `[149,33,7]` e o teste mediria **duas** coisas ao
/// mesmo tempo, sem provar bem nenhuma. A calibração tem o seu próprio ficheiro
/// (`calibration_path.rs`), onde é a variável e não o ruído.
fn perfil(nome: &str) -> HardwareProfile {
    let mut p =
        HardwareRegistry::with_builtin().profile(nome).unwrap_or_else(|| panic!("preset {nome}"));
    p.calibration = led_hardware_profile::Calibration { gamma: 1.0, brightness: 1.0 };
    p
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

        let previsto = cfg.datagrams_per_frame();
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
    assert_eq!(cfg.datagrams_per_frame(), 5);
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
fn resolver_um_endereco_nao_e_um_segundo_caminho() {
    let p = perfil("esp32-poe-wled-ddp");
    let addr = "127.0.0.1:4048".parse().unwrap();
    // `0` e não `1`: este preset é DDP, que endereça por byte e ignora universos — o
    // `resolve` devolve 0 para eles desde o ADR-0029 §7, e comparar contra 1 mediria a
    // diferença de universo em vez da equivalência das três escritas do endereço.
    let direto = OutputConfig::from_profile(&p, addr, 720, 0).unwrap();

    // As três formas de escrever o mesmo endereço têm de dar exatamente a mesma configuração.
    for spec in ["127.0.0.1", "127.0.0.1:4048", "ddp://127.0.0.1:4048"] {
        let resolvido = OutputConfig::resolve(&p, spec, 720).unwrap();
        assert_eq!(direto, resolvido, "`{spec}` divergiu — `resolve` deixou de delegar");
    }
}

/// **O esquema não é uma segunda fonte de protocolo.** Escrevê-lo em desacordo com o preset é
/// erro; sem este teste, `artnet://` sobre um preset DDP passaria em silêncio e o operador
/// julgaria ter escolhido o protocolo.
#[test]
fn o_esquema_escrito_tem_de_concordar_com_o_profile() {
    let ddp = perfil("esp32-poe-wled-ddp");
    let erro = OutputConfig::resolve(&ddp, "artnet://127.0.0.1", 720).unwrap_err();
    assert!(erro.contains("contradiz o profile"), "{erro}");
    assert!(OutputConfig::resolve(&ddp, "ddp://127.0.0.1", 720).is_ok(), "concordar é aceite");

    let artnet = perfil("esp32-devkit-wled-artnet");
    assert!(
        OutputConfig::resolve(&artnet, "ddp://127.0.0.1@0", 720).is_err(),
        "e a recusa vale nos dois sentidos"
    );
}

// ── GS4.4: cor e MTU medidos no fio, não deduzidos ──────────────────────────

/// **RGBW põe quatro bytes por pixel no fio, e o branco é o do ADR-0020.**
///
/// O preset RGBW declara `WhiteMode::MinSubtract`: o neutro sai pelo die branco e é
/// **subtraído** do RGB. Um teste que só contasse canais passaria com o modo aditivo antigo —
/// que consumia 4× mais corrente no branco pleno.
#[test]
fn rgbw_poe_quatro_canais_no_fio_com_o_branco_subtraido() {
    let p = perfil("esp32-poe-wled-rgbw-ddp");
    assert!(matches!(p.capabilities.color, ColorFormat::Rgbw(..)), "o preset tem de ser RGBW");
    let canais = p.capabilities.color.channels();
    assert_eq!(canais, 4);

    let sock = socket();
    let addr = sock.local_addr().unwrap();
    let px = 6usize;
    let cfg = OutputConfig::resolve(&p, &addr.to_string(), px).unwrap();
    // Cor com neutro embutido: min = 50.
    let dg = um_frame(cfg, &sock, vec![PixelColor { r: 200, g: 100, b: 50 }; px]);

    assert_eq!(dg.len(), 1);
    let payload = &dg[0][DDP_HEADER..];
    assert_eq!(payload.len(), px * canais, "4 bytes por pixel, sem padding");

    // O que o `led-core` produz para este formato é a única fonte da verdade do branco.
    let mut esperado = [0u8; 4];
    p.capabilities.color.write(PixelColor { r: 200, g: 100, b: 50 }, &mut esperado);
    assert_eq!(&payload[..4], &esperado, "os bytes do fio têm de ser os do ColorFormat");
    assert_eq!(esperado[3], 50, "W = min(r,g,b)");
    assert!(
        esperado[..3].iter().all(|&c| c < 200),
        "MinSubtract: o neutro foi retirado do RGB, não somado ({esperado:?})"
    );
}

/// **Um MTU menor fragmenta mais, e nenhum datagrama passa do MTU.**
///
/// A previsão vem do `Transport`; a verificação vem do socket. Um MTU declarado que o fio não
/// respeitasse seria pior que não o declarar.
#[test]
fn mtus_diferentes_produzem_fragmentacoes_diferentes_e_nenhum_datagrama_excede_o_mtu() {
    let px = 720usize;
    // MTU maior ⇒ MENOS datagramas. Começa no infinito para o primeiro passo ser válido.
    let mut anterior = u32::MAX;

    for mtu in [576u16, 1_000, 1_500] {
        let mut p = perfil("esp32-poe-wled-ddp");
        p.transport = Transport { mtu_bytes: mtu, ..p.transport };

        let previsto = p.transport.datagrams_for(
            px as u32,
            p.capabilities.protocol,
            p.capabilities.color,
            p.limits.pixels_per_universe,
        );

        let sock = socket();
        let addr = sock.local_addr().unwrap();
        let cfg = OutputConfig::resolve(&p, &addr.to_string(), px).unwrap();
        let dg = um_frame(cfg, &sock, branco(px));

        assert_eq!(dg.len() as u32, previsto, "MTU {mtu}: previsto {previsto}, no fio {}", dg.len());
        for d in &dg {
            assert!(
                d.len() + Transport::IP_UDP_OVERHEAD <= mtu as usize,
                "MTU {mtu}: um datagrama de {} bytes não cabe",
                d.len()
            );
        }
        assert!(previsto < anterior, "MTU {mtu} devia fragmentar MENOS que o MTU anterior");
        anterior = previsto;
    }
}

/// **O primeiro universo é da instância, não do tipo** (ADR-0018) — e o fio obedece.
///
/// Desde o ADR-0029 §7 ele é **declarado na própria especificação** (`IP@N`), e este teste
/// passou a exercitar esse caminho: antes chegava por um parâmetro que a CLI nunca expunha.
#[test]
fn o_primeiro_universo_e_respeitado_seja_qual_for() {
    let p = perfil("esp32-devkit-wled-artnet");
    for primeiro in [0u16, 1, 7, 100] {
        let sock = socket();
        let addr = sock.local_addr().unwrap();
        let cfg = OutputConfig::resolve(&p, &format!("{addr}@{primeiro}"), 400).unwrap();
        let dg = um_frame(cfg, &sock, branco(400));

        let mut universos: Vec<u16> = dg
            .iter()
            .map(|d| u16::from_le_bytes([d[ARTDMX_UNIVERSE_OFF], d[ARTDMX_UNIVERSE_OFF + 1]]))
            .collect();
        universos.sort_unstable();
        let esperado: Vec<u16> = (primeiro..primeiro + universos.len() as u16).collect();
        assert_eq!(universos, esperado, "first_universe={primeiro}");
    }
}

/// **As três ordens de canais produzem três resultados distintos.** Sem isto, um `RgbOrder`
/// ignorado passaria despercebido em qualquer teste que só olhasse para um preset.
#[test]
fn cada_ordem_de_canais_produz_bytes_proprios() {
    let cor = PixelColor { r: 200, g: 100, b: 50 };
    let mut vistos: Vec<(RgbOrder, [u8; 3])> = Vec::new();

    for ordem in [RgbOrder::Rgb, RgbOrder::Grb, RgbOrder::Bgr] {
        let mut p = perfil("esp32-poe-wled-ddp");
        p.capabilities.color = ColorFormat::Rgb(ordem);
        let sock = socket();
        let addr = sock.local_addr().unwrap();
        let cfg = OutputConfig::resolve(&p, &addr.to_string(), 4).unwrap();
        let dg = um_frame(cfg, &sock, vec![cor; 4]);
        let b: [u8; 3] = dg[0][DDP_HEADER..DDP_HEADER + 3].try_into().unwrap();
        vistos.push((ordem, b));
    }

    assert_eq!(vistos[0].1, [200, 100, 50], "RGB");
    assert_eq!(vistos[1].1, [100, 200, 50], "GRB — verde primeiro");
    assert_eq!(vistos[2].1, [50, 100, 200], "BGR");
    for i in 0..vistos.len() {
        for j in (i + 1)..vistos.len() {
            assert_ne!(
                vistos[i].1, vistos[j].1,
                "{:?} e {:?} produziram os mesmos bytes",
                vistos[i].0, vistos[j].0
            );
        }
    }
}

/// **PASSO 6, como gate e não como afirmação.** Um `grep` feito uma vez prova o estado de um
/// instante; este teste prova-o em cada `cargo test`.
///
/// A regra: no caminho da saída do daemon não pode aparecer um valor **físico** literal —
/// ordem de canais, pixels por universo, pixels por datagrama ou MTU. Todos vêm do
/// `HardwareProfile`. As portas dos protocolos são a exceção declarada: são identidade do
/// protocolo (IANA), não propriedade do nó, e o protocolo já vem do profile.
#[test]
fn nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida() {
    const FONTES: &[(&str, &str)] = &[
        ("output.rs", include_str!("../src/output.rs")),
        ("stage.rs", include_str!("../src/stage.rs")),
        ("run.rs", include_str!("../src/run.rs")),
    ];
    // Literais que só podem vir do profile. `170`/`487`/`1462`/`1500` são os que a fatia
    // anterior deixou soltos; `RgbOrder::` construído é o defeito de cor original.
    const PROIBIDOS: &[&str] =
        &["RgbOrder::Rgb)", "RgbOrder::Grb)", "RgbOrder::Bgr)", "170", "487", "1462", "1500"];

    for (nome, fonte) in FONTES {
        // **Só produção.** O `mod tests` e tudo o que vem depois ficam de fora, pela mesma
        // razão que o TD-015 fixou no `surface_gate` do `led-console-bin`: um gate não pode
        // reprovar por causa de um teste que usa o número **para provar a regra**. O
        // `repartir` do ADR-0029 é exercitado com `max_pixels` literais — são entradas de uma
        // função pura, não valores físicos a escapar ao profile no caminho da saída.
        //
        // O corte é por `mod tests`, o mesmo que o `main.rs` já faz contra si próprio — e
        // não por `#[cfg(test)]`, que não apanharia um `#[cfg(all(test, unix))]`.
        let producao = fonte.split("mod tests").next().unwrap_or(fonte);
        for linha in producao.lines() {
            let t = linha.trim_start();
            // Comentários e doc-comments podem (e devem) citar os números ao explicá-los.
            if t.starts_with("//") {
                continue;
            }
            for proibido in PROIBIDOS {
                assert!(
                    !linha.contains(proibido),
                    "{nome}: valor físico `{proibido}` escrito à mão fora de comentário.\n\
                     Tem de vir do HardwareProfile.\n  {linha}"
                );
            }
        }
    }
}
