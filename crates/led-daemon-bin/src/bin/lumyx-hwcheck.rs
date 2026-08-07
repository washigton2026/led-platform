//! GS4.5 — `lumyx-hwcheck`: mede o que o olho não vê.
//!
//! Executa contra hardware real as etapas do GS4.5 que **exigem número**: alcance, latência,
//! jitter, aceitação de cada protocolo, gap de heartbeat e recuperação de queda de cabo.
//!
//! # Nenhuma segunda implementação
//!
//! Não há aqui um socket DDP, um serializador ou um descobridor novo: usa o
//! [`OutputManager`] do daemon, o `ArtPoll` do `led-protocols` e o `HardwareProfile` do
//! catálogo. O que este binário acrescenta é **cronometragem e veredito** — se ele passar e o
//! daemon falhar, um dos dois está a falar com hardware diferente, e isso seria o defeito.
//!
//! # O que faz quando não há rig
//!
//! **Reprova com código 2.** Não produz zeros, não produz médias de zero amostras, e nenhuma
//! linha do relatório diz `PASS`. Ver [`led_daemon_bin::hwcheck`].

use led_core::{LogicalFrame, PixelColor};
use led_daemon_bin::hwcheck::{Amostras, Relatorio, Saida, Veredito};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

const USAGE: &str = "\
lumyx-hwcheck — medição de hardware do GS4.5

USO:
    lumyx-hwcheck <IP> --profile <PRESET> [OPÇÕES]

OPÇÕES:
    --profile PRESET   Preset do HardwareProfile (obrigatório)
    --pixels N         Pixels do nó (padrão: o max_pixels do preset)
    --amostras N       Amostras de ArtPoll para latência/jitter (padrão: 20)
    --out CAMINHO      Escreve o relatório Markdown neste ficheiro
    --cabo             Etapa interativa: pede para desligar e religar o cabo
    -h, --help

CÓDIGOS DE SAÍDA:
    0  tudo medido e aprovado
    1  alguma etapa MEDIDA reprovou
    2  alguma etapa NÃO foi medida — nenhuma conclusão sobre o rig é possível

NOTA:
    Sem hardware na rede, o código de saída é 2 e nenhuma etapa diz PASS.
    Isso é o comportamento correto, não uma avaria do harness.
";

/// Limites do `LUMYX_GOSL` e do ADR-0005, escritos onde o veredito é dado.
const GAP_CRITICO_MS: f64 = 2_400.0;
/// O jitter medido na bancada WiFi em 2026-07-20, que confirmou o ADR-0005. Ethernet tem de
/// ser **ordens de grandeza** melhor; 5 ms é folgado e ainda assim 6× melhor que o WiFi.
const JITTER_MAX_MS: f64 = 5.0;
const WIFI_JITTER_HISTORICO_MS: f64 = 31.0;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut ip: Option<String> = None;
    let (mut preset, mut pixels, mut out) = (None::<String>, None::<usize>, None::<String>);
    let (mut amostras, mut cabo) = (20u32, false);

    let mut i = 0;
    while i < argv.len() {
        // Copiar o argumento **antes** de criar o closure: ele empresta `i` mutavelmente.
        let a = argv[i].clone();
        let mut valor = |n: &str| -> String {
            i += 1;
            argv.get(i).cloned().unwrap_or_else(|| {
                eprintln!("erro: {n} exige um valor");
                std::process::exit(2);
            })
        };
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            "--profile" => preset = Some(valor("--profile")),
            "--pixels" => pixels = valor("--pixels").parse().ok(),
            "--amostras" => amostras = valor("--amostras").parse().unwrap_or(20),
            "--out" => out = Some(valor("--out")),
            "--cabo" => cabo = true,
            outro if outro.starts_with('-') => {
                eprintln!("erro: opção desconhecida {outro}\n\n{USAGE}");
                std::process::exit(2);
            }
            outro => ip = Some(outro.to_string()),
        }
        i += 1;
    }

    let (Some(ip), Some(preset)) = (ip, preset) else {
        eprintln!("erro: faltam <IP> e/ou --profile\n\n{USAGE}");
        std::process::exit(2);
    };
    let perfil = match profile_by_name(&preset) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(2);
        }
    };
    let px = pixels.unwrap_or(perfil.limits.max_pixels as usize);
    let alvo_v4: Ipv4Addr = match ip.parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("erro: `{ip}` não é um IPv4");
            std::process::exit(2);
        }
    };

    let mut r = Relatorio { alvo: ip.clone(), preset: preset.clone(), ..Default::default() };

    // ── 1. Alcance, latência e jitter, por ArtPoll ────────────────────────────
    //
    // ArtPoll é o único **round-trip** que o LUMYX já fala: DDP e sACN são fire-and-forget e
    // não podem medir latência por definição. E não precisa de root, ao contrário do ICMP.
    let (lat, motivo) = medir_artpoll(alvo_v4, amostras);
    match (lat.media(), lat.jitter()) {
        (Some(m), Some(j)) => {
            let p99 = lat.percentil(99.0).unwrap_or(m);
            let d = format!(
                "{}/{} respostas · media {m:.2} ms · p99 {p99:.2} ms · jitter {j:.2} ms \
                 (WiFi 2026-07-20: {WIFI_JITTER_HISTORICO_MS} ms)",
                lat.len(),
                amostras
            );
            r.add(
                "alcance+latencia",
                "o no responde, e o caminho e estavel",
                "todas as amostras respondem · jitter < 5 ms",
                if lat.len() == amostras as usize && j < JITTER_MAX_MS {
                    Veredito::Passa(d)
                } else {
                    Veredito::Reprova(d)
                },
            );
        }
        _ => r.add(
            "alcance+latencia",
            "o no responde, e o caminho e estavel",
            "todas as amostras respondem · jitter < 5 ms",
            Veredito::NaoMedido(motivo),
        ),
    }

    // ── 2. Identidade do controlador, por HTTP ───────────────────────────────
    let info = ler_json_info(alvo_v4);
    match &info {
        Ok(j) => r.add(
            "controlador",
            "saber com que firmware se esta a falar",
            "responde /json/info com ver e freeheap",
            Veredito::Passa(resumo_info(j)),
        ),
        Err(e) => r.add(
            "controlador",
            "saber com que firmware se esta a falar",
            "responde /json/info com ver e freeheap",
            Veredito::NaoMedido(e.clone()),
        ),
    }

    // ── 3. Aceitação de cada protocolo ───────────────────────────────────────
    //
    // A evidência de aceitação é o `lm`/`live` do próprio WLED — mais forte que tcpdump
    // (precedente de 2026-07-23). Enviar sem confirmar seria medir o `sendto`, não o rig.
    for (proto, preset_do_proto) in
        [("DDP", "Ddp"), ("Art-Net", "ArtNet"), ("sACN", "Sacn")]
    {
        let esperado = format!("{:?}", perfil.capabilities.protocol);
        if esperado != preset_do_proto {
            r.add(
                nome_estatico(proto),
                "o no aceita este protocolo",
                "WLED reporta live:true e lm igual ao protocolo",
                Veredito::NaoMedido(format!(
                    "o preset `{preset}` declara {esperado}; para medir {proto} use o preset \
                     correspondente (o protocolo vem do HardwareProfile, GS4.4)"
                )),
            );
            continue;
        }
        let v = medir_aceitacao(&perfil, alvo_v4, px, proto);
        r.add(
            nome_estatico(proto),
            "o no aceita este protocolo",
            "WLED reporta live:true e lm igual ao protocolo",
            v,
        );
    }

    // ── 4. Gap de heartbeat, medido no relógio de parede ─────────────────────
    //
    // **Só conta se o controlador estiver confirmado.** UDP é fire-and-forget: o `sendto`
    // para um IP inexistente tem sucesso local, e sem esta guarda o harness diria PASS
    // contra um rig ausente — mediria a cadência do *remetente*, não a do palco. Foi
    // exatamente isso que a primeira execução deste binário fez, e é a razão da guarda.
    r.add(
        "heartbeat",
        "o palco nunca fica mais de 2400 ms sem frame",
        "maior intervalo entre envios < 2400 ms, COM o controlador confirmado",
        match &info {
            Ok(_) => medir_heartbeat(&perfil, alvo_v4, px),
            Err(e) => {
                let local = medir_heartbeat(&perfil, alvo_v4, px);
                Veredito::NaoMedido(format!(
                    "cadencia local: {} — mas o controlador nao responde ({e}); \
                     isto mede o remetente, NAO o palco",
                    local.detalhe()
                ))
            }
        },
    );

    // ── 5. Queda de cabo e recuperação (exige o operador) ────────────────────
    r.add(
        "queda+recovery",
        "o sistema recupera de uma falha do meio fisico",
        "o envio volta a ter sucesso, e o no NAO reinicia (uptime monotonico)",
        if cabo {
            medir_queda_de_cabo(&perfil, alvo_v4, px, info.as_ref().ok())
        } else {
            Veredito::NaoMedido(
                "etapa interativa: correr com --cabo, com o operador presente".into(),
            )
        },
    );

    println!("{r}");
    if let Some(caminho) = &out {
        match std::fs::write(caminho, r.markdown()) {
            Ok(()) => println!("\nrelatorio: {caminho}"),
            Err(e) => eprintln!("aviso: nao consegui escrever {caminho}: {e}"),
        }
    }

    let saida = r.saida();
    if saida == Saida::Incompleto {
        eprintln!(
            "\nINCOMPLETO: ha etapas NAO MEDIDAS. Isto nao diz nada sobre o rig — \
             nao registe como validacao."
        );
    }
    std::process::exit(saida as i32);
}

/// Os nomes das etapas são `&'static str` no relatório; este mapa evita alocar nomes dinâmicos
/// que depois não casariam entre execuções.
fn nome_estatico(proto: &str) -> &'static str {
    match proto {
        "DDP" => "protocolo:ddp",
        "Art-Net" => "protocolo:artnet",
        _ => "protocolo:sacn",
    }
}

/// Mede latência de ida-e-volta por ArtPoll. Devolve as amostras e, se falhar, o porquê.
fn medir_artpoll(alvo: Ipv4Addr, n: u32) -> (Amostras, String) {
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => return (Amostras::default(), format!("nao consegui abrir socket: {e}")),
    };
    if let Err(e) = sock.set_read_timeout(Some(Duration::from_millis(500))) {
        return (Amostras::default(), format!("timeout: {e}"));
    }
    let mut poll = [0u8; led_protocols::ART_POLL_LEN];
    led_protocols::build_art_poll(&mut poll);
    // Porta do Art-Net (IANA 6454): identidade do protocolo, não do nó (GS4.4).
    let destino = SocketAddr::from((alvo, 6454));

    let mut amostras = Amostras::default();
    let mut ultimo_erro = String::from("nenhuma resposta a ArtPoll");
    for _ in 0..n {
        let t0 = Instant::now();
        if let Err(e) = sock.send_to(&poll, destino) {
            ultimo_erro = format!("envio de ArtPoll falhou: {e}");
            continue;
        }
        let mut buf = [0u8; 1024];
        match sock.recv_from(&mut buf) {
            Ok((len, origem)) => {
                let ok = led_protocols::parse_art_poll_reply(&buf[..len])
                    .map(|r| r.ip == alvo)
                    .unwrap_or(false);
                if ok {
                    amostras.push(t0.elapsed().as_secs_f64() * 1000.0);
                } else {
                    // Resposta de outro nó não conta como resposta deste — é a mesma regra do
                    // controle negativo da descoberta (RT-003).
                    ultimo_erro = format!("resposta de {origem}, que nao e o alvo");
                }
            }
            Err(e) => ultimo_erro = format!("sem resposta: {e}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (amostras, ultimo_erro)
}

/// Lê `/json/info` do WLED por HTTP 1.0 — std puro, sem dependência nova.
fn ler_json_info(alvo: Ipv4Addr) -> Result<String, String> {
    let mut s = TcpStream::connect_timeout(
        &SocketAddr::from((alvo, 80)),
        Duration::from_millis(1_500),
    )
    .map_err(|e| format!("HTTP {alvo}:80 inacessivel: {e}"))?;
    s.set_read_timeout(Some(Duration::from_millis(2_000))).ok();
    s.write_all(format!("GET /json/info HTTP/1.0\r\nHost: {alvo}\r\n\r\n").as_bytes())
        .map_err(|e| format!("envio HTTP falhou: {e}"))?;
    let mut corpo = String::new();
    s.read_to_string(&mut corpo).map_err(|e| format!("leitura HTTP falhou: {e}"))?;
    corpo
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .ok_or_else(|| "resposta HTTP sem corpo".into())
}

fn campo(json: &str, chave: &str) -> Option<String> {
    let marca = format!("\"{chave}\":");
    let resto = json.split_once(&marca)?.1.trim_start();
    let fim = resto.find([',', '}']).unwrap_or(resto.len());
    Some(resto[..fim].trim().trim_matches('"').to_string())
}

fn resumo_info(json: &str) -> String {
    let g = |k: &str| campo(json, k).unwrap_or_else(|| "?".into());
    format!("ver {} · arch {} · freeheap {} · uptime {} s", g("ver"), g("arch"), g("freeheap"), g("uptime"))
}

/// Envia frames reais e confirma **no próprio controlador** que foram aceites.
fn medir_aceitacao(
    perfil: &led_hardware_profile::HardwareProfile,
    alvo: Ipv4Addr,
    px: usize,
    proto: &str,
) -> Veredito {
    let cfg = match OutputConfig::resolve(perfil, &alvo.to_string(), px, 1) {
        Ok(c) => c,
        Err(e) => return Veredito::NaoMedido(format!("configuracao: {e}")),
    };
    let om = match OutputManager::open(cfg) {
        Ok(o) => o,
        Err(e) => return Veredito::NaoMedido(format!("abrir saida: {e}")),
    };
    // Valor baixo de propósito: o ABL do WLED é a rede de segurança, não a primeira linha.
    let frame = LogicalFrame::new(vec![PixelColor { r: 64, g: 0, b: 0 }; px], 0);
    for _ in 0..40 {
        if let Err(e) = om.send(&frame) {
            return Veredito::Reprova(format!("envio falhou: {e:?}"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    match ler_json_info(alvo) {
        Ok(j) => {
            let live = campo(&j, "live").as_deref() == Some("true");
            let lm = campo(&j, "lm").unwrap_or_default();
            let d = format!(
                "{} frames enviados, {} erros · live:{live} · lm:\"{lm}\"",
                om.stats().frames(),
                om.stats().errors()
            );
            // O `lm` do WLED é a evidência de aceitação adotada em 2026-07-23.
            if live && lm.to_lowercase().contains(&proto.to_lowercase().replace('-', "")) {
                Veredito::Passa(d)
            } else if live {
                Veredito::Reprova(format!("{d} — live mas com outro protocolo a mandar"))
            } else {
                Veredito::Reprova(format!("{d} — o controlador NAO aceitou (live:false)"))
            }
        }
        Err(e) => Veredito::NaoMedido(format!(
            "enviei {} frames, mas nao consigo confirmar aceitacao: {e}",
            om.stats().frames()
        )),
    }
}

/// Mede o **maior intervalo real** entre envios consecutivos ao longo de 6 s.
fn medir_heartbeat(
    perfil: &led_hardware_profile::HardwareProfile,
    alvo: Ipv4Addr,
    px: usize,
) -> Veredito {
    let cfg = match OutputConfig::resolve(perfil, &alvo.to_string(), px, 1) {
        Ok(c) => c,
        Err(e) => return Veredito::NaoMedido(format!("configuracao: {e}")),
    };
    let om = match OutputManager::open(cfg) {
        Ok(o) => o,
        Err(e) => return Veredito::NaoMedido(format!("abrir saida: {e}")),
    };
    let frame = LogicalFrame::new(vec![PixelColor { r: 64, g: 0, b: 0 }; px], 0);
    let periodo = perfil.transport.heartbeat_ms as u64;
    let t0 = Instant::now();
    let mut carimbos = Amostras::default();
    let mut erros = 0u32;
    while t0.elapsed() < Duration::from_secs(6) {
        match om.send(&frame) {
            Ok(()) => carimbos.push(t0.elapsed().as_secs_f64() * 1000.0),
            Err(_) => erros += 1,
        }
        std::thread::sleep(Duration::from_millis(periodo));
    }
    match carimbos.maior_intervalo() {
        Some(g) => {
            let d = format!(
                "periodo declarado {periodo} ms · maior intervalo real {g:.0} ms · {} envios · {erros} erros",
                carimbos.len()
            );
            if g < GAP_CRITICO_MS && erros == 0 {
                Veredito::Passa(d)
            } else {
                Veredito::Reprova(d)
            }
        }
        None => Veredito::NaoMedido("menos de dois envios: nao ha intervalo a medir".into()),
    }
}

/// Etapa interativa: o operador puxa o cabo, e o harness cronometra a recuperação.
fn medir_queda_de_cabo(
    perfil: &led_hardware_profile::HardwareProfile,
    alvo: Ipv4Addr,
    px: usize,
    info_antes: Option<&String>,
) -> Veredito {
    let cfg = match OutputConfig::resolve(perfil, &alvo.to_string(), px, 1) {
        Ok(c) => c,
        Err(e) => return Veredito::NaoMedido(format!("configuracao: {e}")),
    };
    let om = match OutputManager::open(cfg) {
        Ok(o) => o,
        Err(e) => return Veredito::NaoMedido(format!("abrir saida: {e}")),
    };
    let uptime_antes: Option<u64> =
        info_antes.and_then(|j| campo(j, "uptime")).and_then(|v| v.parse().ok());

    println!("\n>>> DESLIGUE O CABO DE REDE AGORA. A medir por 60 s...");
    let frame = LogicalFrame::new(vec![PixelColor { r: 64, g: 0, b: 0 }; px], 0);
    let t0 = Instant::now();
    let (mut caiu_em, mut voltou_em) = (None::<f64>, None::<f64>);

    while t0.elapsed() < Duration::from_secs(60) {
        let t = t0.elapsed().as_secs_f64();
        let ok = om.send(&frame).is_ok() && ler_json_info(alvo).is_ok();
        match (caiu_em, voltou_em, ok) {
            (None, _, false) => {
                caiu_em = Some(t);
                println!("    queda detetada a {t:.1} s — RELIGUE O CABO");
            }
            (Some(_), None, true) => {
                voltou_em = Some(t);
                println!("    recuperado a {t:.1} s");
                break;
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let (Some(c), Some(v)) = (caiu_em, voltou_em) else {
        return Veredito::NaoMedido(format!(
            "nao observei ciclo completo em 60 s (queda: {caiu_em:?}, retorno: {voltou_em:?}) — \
             o cabo foi mesmo desligado?"
        ));
    };

    // Um reset do ESP32 invalida a etapa: recuperar reiniciando não é recuperar.
    let uptime_depois: Option<u64> =
        ler_json_info(alvo).ok().and_then(|j| campo(&j, "uptime")).and_then(|v| v.parse().ok());
    let d = format!(
        "queda a {c:.1} s · recuperado {:.1} s depois · uptime {:?} -> {:?}",
        v - c,
        uptime_antes,
        uptime_depois
    );
    match (uptime_antes, uptime_depois) {
        (Some(a), Some(b)) if b < a => {
            Veredito::Reprova(format!("{d} — o controlador REINICIOU (uptime regrediu)"))
        }
        (Some(_), Some(_)) => Veredito::Passa(d),
        _ => Veredito::NaoMedido(format!("{d} — sem uptime nao sei se houve reset")),
    }
}
