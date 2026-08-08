//! ADR-0024 — **um `HardwareProfile` inválido nunca abre saída, e por isso nunca chega a
//! `Ready`.**
//!
//! Estes testes descrevem a **regra arquitetural**, não o comportamento de hoje. Foram
//! escritos antes da implementação e reprovaram — é essa a razão de existirem.
//!
//! # Onde a fronteira fica
//!
//! A validação é **estática** e corre na construção da saída, que o laço já executa antes do
//! pré-voo e do `Arm`. O `PreflightReport` do ADR-0023 continua com três campos e **não foi
//! tocado**: um profile inválido não precisa de um quarto campo, precisa de não abrir palco.

use led_core::PixelColor;
use led_daemon::{ShowId, ShowRuntime, State};
use led_daemon_bin::{
    descriptor_from_path, profile_by_name, run, Config, ExitReason, Integrity, Journal,
    OutputConfig, Pacer,
};
use led_hardware_profile::{ColorFormat, HardwareProfile, Protocol, RgbOrder, WhiteMode};
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

fn escrever(nome: &str, px: u32) -> String {
    let path = std::env::temp_dir().join(nome);
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, px).unwrap();
    for i in 0..4u32 {
        w.write_frame(&ShowRecord {
            timestamp_ms: i as u64 * 25,
            pixels: vec![PixelColor { r: 10, g: 20, b: 30 }; px as usize],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

fn valido() -> HardwareProfile {
    profile_by_name("esp32-poe-wled-ddp").unwrap()
}

fn socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").unwrap()
}

// ── 1 · o caminho feliz continua a passar ────────────────────────────────────

#[test]
fn um_profile_valido_continua_a_abrir_saida() {
    let sock = socket();
    let cfg = OutputConfig::resolve(&valido(), &sock.local_addr().unwrap().to_string(), 8, 1);
    assert!(cfg.is_ok(), "o catálogo curado não pode ser recusado: {cfg:?}");
}

/// **Todos os presets do catálogo têm de ser válidos.** É este o teste que apanha um preset
/// novo inválido — sem ele, a regra só valeria para os presets que alguém se lembrasse de
/// testar à mão.
#[test]
fn todos_os_presets_do_catalogo_passam_na_validacao() {
    let reg = led_hardware_profile::HardwareRegistry::with_builtin();
    let sock = socket();
    let addr = sock.local_addr().unwrap().to_string();
    for nome in reg.names() {
        let p = reg.profile(nome).unwrap();
        let px = (p.limits.max_pixels as usize).clamp(1, 64);
        let r = OutputConfig::resolve(&p, &addr, px, 1);
        assert!(r.is_ok(), "preset `{nome}` do catálogo é inválido: {:?}", r.err());
    }
}

// ── 2 · cada erro do validador impede a saída ────────────────────────────────

/// Percorre as classes de erro que o ADR-0018 define, uma a uma. Cada uma **tem** de recusar.
#[test]
fn cada_classe_de_erro_do_profile_impede_a_saida() {
    let sock = socket();
    let addr = sock.local_addr().unwrap().to_string();

    let mut casos: Vec<(&str, HardwareProfile)> = Vec::new();

    let mut p = valido();
    p.schema_version = 999;
    casos.push(("schema desconhecida", p));

    let mut p = valido();
    p.capabilities.output_interface = led_hardware_profile::OutputInterface::Spi;
    casos.push(("interface sem driver (SPI)", p));

    // **Preset Art-Net de propósito**: a regra do universo só se aplica a protocolos
    // baseados em universo. O DDP endereça por byte e o validador ignora-a — usar um preset
    // DDP aqui testaria o protocolo errado, e foi o erro que a 1.ª versão deste teste cometeu.
    let mut p = profile_by_name("esp32-devkit-wled-artnet").unwrap();
    p.limits.pixels_per_universe = 500; // 500 × 3 = 1500 canais > 512 do universo
    casos.push(("pixels não cabem no universo (Art-Net)", p));

    let mut p = valido();
    p.limits.max_pixels = 0;
    casos.push(("limite zerado", p));

    let mut p = valido();
    p.power.voltage_v = f32::NAN;
    casos.push(("Power inválido (NaN)", p));

    let mut p = valido();
    p.calibration.gamma = -1.0;
    casos.push(("Calibration inválida", p));

    for (porque, perfil) in casos {
        let r = OutputConfig::resolve(&perfil, &addr, 8, 1);
        assert!(r.is_err(), "`{porque}` devia impedir a saída, mas passou");
        let e = r.unwrap_err();
        assert!(
            e.to_lowercase().contains("profile"),
            "`{porque}`: o erro tem de nomear o profile, veio `{e}`"
        );
    }
}

/// **Um aviso não é um erro.** O preset RGBW-sobre-DDP avisa por desenho (o data type não foi
/// validado em hardware) e **tem** de continuar a funcionar — bloqueá-lo mudaria o
/// significado de `Warning` fixado no ADR-0018.
#[test]
fn um_aviso_nao_impede_a_saida() {
    let sock = socket();
    let p = profile_by_name("esp32-poe-wled-rgbw-ddp").unwrap();
    assert!(matches!(p.capabilities.color, ColorFormat::Rgbw(..)));
    assert_eq!(p.capabilities.protocol, Protocol::Ddp, "é o caso que gera RgbwOverDdpDataType");
    let r = OutputConfig::resolve(&p, &sock.local_addr().unwrap().to_string(), 8, 1);
    assert!(r.is_ok(), "um Warning não pode recusar: {:?}", r.err());
}

/// WiFi declarado **avisa** (ADR-0005 é enforçado pelo `WifiBlockGuard`, contra o host) —
/// mover o bloqueio para aqui seria duplicar o enforcement no sítio errado.
#[test]
fn wifi_declarado_avisa_mas_nao_recusa_na_validacao_estatica() {
    let sock = socket();
    let p = profile_by_name("esp32-devkit-wled-artnet").unwrap();
    assert_eq!(p.capabilities.output_interface, led_hardware_profile::OutputInterface::WiFi);
    let r = OutputConfig::resolve(&p, &sock.local_addr().unwrap().to_string(), 8, 1);
    assert!(r.is_ok(), "o gate do ADR-0005 é do pré-voo, não da validação estática");
}

// ── 3 · a regra que interessa: nunca chega a Ready ───────────────────────────

/// **Um preset que não se consegue resolver não leva o daemon a `Ready`.**
///
/// # O que este teste prova, e o que NÃO prova — corrigido depois de o falsificar
///
/// A primeira versão chamava-se `um_profile_invalido_nunca_chega_a_ready` e **afirmava mais
/// do que exercitava**: `preset-que-nao-existe` falha em `profile_by_name`, que é um ramo
/// *anterior* ao `Stage::open`. Ao plantar o bug "profile inválido prossegue em vez de
/// abortar", este teste **passou** — o ramo que o bug tocava não era o que ele percorria.
///
/// O ramo do `Stage::open` é guardado por `saida_impossivel_impede_o_arranque`
/// (`tests/e2e_output.rs`), que **reprova** com esse bug plantado. E a recusa por profile
/// inválido é guardada por `cada_classe_de_erro_do_profile_impede_a_saida`, acima.
///
/// Não escrevi aqui um terceiro teste quase igual ao do `e2e_output.rs`: seria repetição, e
/// a cobertura já existe. O que faltava era o nome dizer a verdade.
///
/// **Nota sobre alcançabilidade:** um preset que *existe no catálogo* mas é inválido não é
/// alcançável pela CLI hoje — `todos_os_presets_do_catalogo_passam_na_validacao` garante-o.
/// É por isso que a recusa por profile é provada ao nível de `resolve`, e não pelo laço.
#[test]
fn um_preset_que_nao_resolve_nunca_chega_a_ready() {
    let path = escrever("pv_ready.lumyx", 8);
    let sock = socket();
    let cfg = Config {
        tick_ms: 20,
        max_ticks: Some(5),
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: Some(sock.local_addr().unwrap().to_string()),
        // O `custom` é o único preset que o operador edita — aqui é usado como veículo de um
        // erro. O erro em si vem da schema desconhecida, injetada abaixo pelo `--profile`.
        profile: Some("preset-que-nao-existe".to_string()),
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
    assert_ne!(out.final_state, State::Ready, "NUNCA Ready com profile inválido");
    assert_ne!(out.final_state, State::Playing, "e muito menos a tocar");
    assert!(log.contains(r#""notice":"output_failed""#), "{log}");
    let _ = std::fs::remove_file(path);
}

// ── 4 · o `Available` não pode divergir do que o daemon constrói ─────────────

/// **`ALL` cobre todas as variantes.** O `match` de `OutputManager::open` é exaustivo por
/// construção; esta prova obriga a lista declarada a acompanhá-lo. Um protocolo novo sem
/// entrada em `ALL` faria o daemon recusar profiles que sabe construir.
#[test]
fn a_lista_de_protocolos_disponiveis_cobre_todas_as_variantes() {
    use led_daemon_bin::OutputProtocol;
    assert_eq!(OutputProtocol::ALL.len(), 3, "uma entrada por variante");
    for p in OutputProtocol::ALL {
        // `match` exaustivo: acrescentar uma variante ao enum quebra aqui até ser tratada.
        let _: &'static str = match p {
            OutputProtocol::Ddp => "ddp",
            OutputProtocol::ArtNet => "artnet",
            OutputProtocol::Sacn => "sacn",
        };
    }
    for esperado in [Protocol::Ddp, Protocol::ArtNet, Protocol::Sacn] {
        assert!(
            OutputProtocol::ALL.iter().any(|p| p.to_profile() == esperado),
            "{esperado:?} não está em ALL — o daemon recusaria um profile que sabe construir"
        );
    }
}

/// **Um `Available` vazio não pode transformar um profile inválido em válido.** É o controle
/// negativo do desenho: se alguém passar uma lista vazia por engano, tudo tem de reprovar —
/// nunca o contrário.
#[test]
fn nenhuma_lista_de_disponiveis_torna_valido_um_profile_quebrado() {
    let mut p = valido();
    p.schema_version = 999;
    let vazio = led_hardware_profile::Available { interfaces: &[], protocols: &[] };
    let v = led_hardware_profile::validate(&p, &vazio);
    assert!(v.has_errors(), "schema desconhecida não depende de haver drivers");

    // E o inverso: um profile bom com lista vazia **reprova** (não há driver), o que confirma
    // que a lista é consultada de verdade.
    let bom = led_hardware_profile::validate(&valido(), &vazio);
    assert!(bom.has_errors(), "sem drivers declarados, nem o preset bom passa");
}

/// **O formato de cor entra na conta do universo.** 4 canais × 170 px = 680 > 512: um preset
/// que passa em RGB reprova em RGBW sem mudar mais nada. É o que prova que o `channels()` do
/// `ColorFormat` é consultado, e não assumido em 3.
#[test]
fn o_formato_de_cor_entra_na_conta_do_universo() {
    let sock = socket();
    let addr = sock.local_addr().unwrap().to_string();
    let base = profile_by_name("esp32-devkit-wled-artnet").unwrap();
    assert_eq!(base.limits.pixels_per_universe, 170);
    assert!(OutputConfig::resolve(&base, &addr, 8, 1).is_ok(), "em RGB, 170 × 3 = 510 ≤ 512");

    let mut rgbw = base.clone();
    rgbw.capabilities.color = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::MinSubtract);
    assert!(
        OutputConfig::resolve(&rgbw, &addr, 8, 1).is_err(),
        "em RGBW, 170 × 4 = 680 > 512 — a mesma linha de preset deixa de ser válida"
    );
}
