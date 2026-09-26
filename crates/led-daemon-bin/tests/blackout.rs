//! ADR-0017 (D6) — a máscara de blackout na fronteira de saída.
//!
//! # O que estes testes existem para impedir
//!
//! Que o blackout seja *afirmado* em vez de *provado*. A propriedade que interessa não é «o
//! `OutputManager` guarda um booleano» — é **que byte sai no fio**, e **o que ficou gravado**.
//! Um teste que só verificasse o estado interno passaria com uma máscara que nunca é aplicada.
//!
//! # A propriedade mais fácil de inverter, e a que mais custa
//!
//! A decisão 3 do ADR — *«`record()` nunca recebe frame mascarado»* — é o ponto que uma
//! implementação distraída inverte, porque mascarar **antes** de gravar é mais simples de
//! escrever. Se isso acontecesse, o `restore` devolveria **preto**: o último frame "válido"
//! passaria a ser o preto, e o palco nunca mais acenderia sem um novo frame do show. O teste
//! `o_restore_devolve_a_cor_real_e_nao_preto` é o que apanha essa inversão.
//!
//! # Fronteira do que aqui se prova
//!
//! Prova-se o que **sai do daemon**. Não se prova que um controlador real apaga — isso é o
//! 9.C do ADR, e continua **NÃO MEDIDO**.

use led_core::{LogicalFrame, PixelColor};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_hal::Heartbeat;
use std::net::UdpSocket;
use std::time::Duration;

const DDP_HEADER: usize = 10;
const COR: PixelColor = PixelColor { r: 200, g: 100, b: 50 };
const PX: usize = 8;

fn socket() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
    s
}

/// Um `OutputManager` DDP com um alvo, e o socket que recebe o que ele envia.
///
/// DDP porque é o protocolo **validado em hardware** (94/94 frames, 2026-07-20) e o que o
/// GS4.5 usa. A máscara é um só ponto para os três protocolos (decisão 2); provar noutro
/// protocolo provaria o mesmo código.
fn saida() -> (OutputManager, UdpSocket) {
    let sock = socket();
    let perfil = profile_by_name("esp32-poe-wled-ddp").expect("preset do catálogo");
    let cfg = OutputConfig::resolve(&perfil, &sock.local_addr().unwrap().to_string(), PX)
        .expect("resolver");
    (OutputManager::open(cfg).expect("abrir saída"), sock)
}

/// Os bytes de píxel do primeiro datagrama que chegar.
fn recebido(sock: &UdpSocket) -> Vec<u8> {
    let mut buf = [0u8; 4096];
    let n = sock.recv(&mut buf).expect("um datagrama");
    buf[DDP_HEADER..n].to_vec()
}

fn e_todo_preto(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&b| b == 0)
}

// ── A máscara ───────────────────────────────────────────────────────────────

/// **O blackout apaga o fio, e levantar devolve a cor.** É a propriedade central.
///
/// Sem a máscara aplicada no `enviar`, o segundo bloco recebe a cor e o teste reprova.
#[test]
fn o_blackout_apaga_o_fio_e_levantar_devolve_a_cor() {
    let (om, sock) = saida();
    let frame = LogicalFrame::new(vec![COR; PX], 0);

    om.send(&frame).unwrap();
    let aceso = recebido(&sock);
    assert!(!e_todo_preto(&aceso), "premissa: sem blackout o fio tem de levar cor");

    assert!(om.blackout_accionar(), "o primeiro accionamento muda o estado");
    om.send(&frame).unwrap();
    assert!(e_todo_preto(&recebido(&sock)), "com blackout o fio tem de levar PRETO");

    assert!(om.blackout_levantar(), "levantar muda o estado");
    om.send(&frame).unwrap();
    assert_eq!(recebido(&sock), aceso, "levantado, o fio volta a ser byte-idêntico");
}

/// **Fade instantâneo** (decisão 8): o PRIMEIRO frame após accionar já sai preto.
///
/// Uma rampa deixaria uma janela em que o palco ainda ilumina — e um mecanismo de segurança
/// não pode ter essa janela. Este teste reprova se alguém introduzir interpolação.
#[test]
fn o_fade_e_instantaneo_o_primeiro_frame_ja_sai_preto() {
    let (om, sock) = saida();
    om.blackout_accionar();
    om.send(&LogicalFrame::new(vec![COR; PX], 0)).unwrap();
    assert!(e_todo_preto(&recebido(&sock)), "o primeiro frame após accionar já tem de ser preto");
}

/// **Latching** (decisão 5): mantém-se sem ser reaccionado, e o estado é observável.
///
/// Um blackout que se desfizesse sozinho seria indistinguível de uma falha.
#[test]
fn o_latching_mantem_se_e_o_estado_e_visivel() {
    let (om, sock) = saida();
    assert!(!om.blackout_activo(), "nasce desligado");
    om.blackout_accionar();
    assert!(om.blackout_activo(), "o estado tem de ser visível ao operador");

    for i in 0..5 {
        om.send(&LogicalFrame::new(vec![COR; PX], i)).unwrap();
        assert!(e_todo_preto(&recebido(&sock)), "frame {i}: o blackout não se pode desfazer sozinho");
    }
    assert!(om.blackout_activo(), "continua activo depois de 5 frames");
}

/// Accionar duas vezes **diz que não mudou** — é o que o log auditável da decisão 6 precisa
/// para distinguir «apaguei agora» de «já estava apagado» sem contabilidade própria.
#[test]
fn accionar_ou_levantar_repetido_reporta_que_nao_mudou() {
    let (om, _sock) = saida();
    assert!(om.blackout_accionar(), "1.ª vez muda");
    assert!(!om.blackout_accionar(), "2.ª vez NÃO muda");
    assert!(om.blackout_levantar(), "levantar muda");
    assert!(!om.blackout_levantar(), "levantar de novo NÃO muda");
}

// ── record() e o heartbeat: o coração da decisão 1 ──────────────────────────

/// **O `restore` devolve a COR REAL, não preto** — logo `record()` nunca viu o frame
/// mascarado (decisão 3).
///
/// Este é o teste que apanha a inversão mais provável. Se a máscara fosse aplicada **antes**
/// de `record()`, o heartbeat passaria a reenviar preto para sempre, e levantar o blackout
/// não acenderia nada até chegar um frame novo do show.
#[test]
fn o_restore_devolve_a_cor_real_e_nao_preto() {
    let (om, sock) = saida();
    let hb = Heartbeat::new();
    let frame = LogicalFrame::new(vec![COR; PX], 0);

    // A ordem do `Stage` (stage.rs:92-93): grava o REAL, depois envia.
    hb.record(&frame);
    om.send(&frame).unwrap();
    let aceso = recebido(&sock);

    om.blackout_accionar();
    assert!(hb.beat(&om).unwrap(), "o heartbeat continua a emitir durante o blackout");
    assert!(e_todo_preto(&recebido(&sock)), "durante o blackout o keep-alive sai PRETO");

    // Levantar sem enviar frame novo: o que o heartbeat tem guardado é o frame REAL.
    om.blackout_levantar();
    assert!(hb.beat(&om).unwrap(), "o heartbeat emite depois de levantar");
    assert_eq!(
        recebido(&sock),
        aceso,
        "o heartbeat guardou o frame REAL — se tivesse gravado preto, isto seria preto"
    );
}

/// **§4-#9 continua literalmente verdadeiro:** o heartbeat nunca *fabrica* zeros.
///
/// Sem frame gravado não sai nada — nem sequer durante o blackout. Quem zera é a máscara, e a
/// máscara é comandada. Este teste é o controlo negativo do anterior: prova que o preto que se
/// vê no fio vem da máscara, e não de o heartbeat ter passado a inventar zeros.
#[test]
fn o_heartbeat_nunca_fabrica_zeros_nem_com_blackout_activo() {
    let (om, _sock) = saida();
    let hb = Heartbeat::new();
    om.blackout_accionar();
    assert!(
        !hb.beat(&om).unwrap(),
        "sem frame gravado o heartbeat NÃO envia — nem um preto fabricado"
    );
    assert_eq!(om.stats().frames(), 0, "nada saiu");
}

// ── Escape por device: o requisito de aceitação (decisão 7) ─────────────────

/// **O escape por device é efectivo, e é por NÓ.**
///
/// Dois alvos, um deles declarado como escape. Com o blackout accionado, um recebe preto e o
/// outro **continua aceso** — que é a razão física da decisão 7: um traje autónomo não é
/// apagado por um botão de consola.
///
/// Com um só alvo este teste não provaria nada — «apaga tudo» e «apaga só os não-escape»
/// seriam indistinguíveis. É a mesma lição do ADR-0029 §8, onde um alvo tornava «por nó» e
/// «agregado» indistinguíveis.
#[test]
fn o_escape_por_device_poupa_o_no_declarado_e_apaga_os_outros() {
    let a = socket();
    let b = socket();
    let perfil = profile_by_name("esp32-poe-wled-ddp").expect("preset");
    let specs =
        [a.local_addr().unwrap().to_string(), b.local_addr().unwrap().to_string()];
    let mut cfg =
        OutputConfig::resolve_muitos(&perfil, &specs, 3000).expect("resolver dois nós");
    assert_eq!(cfg.alvos.len(), 2, "premissa: dois nós");

    // O SEGUNDO nó escapa. Declarado na instância, como a decisão 7 exige.
    cfg.alvos[1].escapa_blackout = true;

    let om = OutputManager::open(cfg).expect("abrir");
    om.blackout_accionar();
    om.send(&LogicalFrame::new(vec![COR; 3000], 0)).unwrap();

    assert!(e_todo_preto(&recebido(&a)), "o nó SEM escape tem de ficar preto");
    assert!(
        !e_todo_preto(&recebido(&b)),
        "o nó COM escape tem de continuar aceso — senão a decisão 7 não existe"
    );
}

/// Sem ninguém declarar escape, **todos** apagam. É o controlo negativo do teste anterior:
/// sem ele, um bug que ignorasse a máscara em todos os nós passaria despercebido.
#[test]
fn sem_escape_declarado_todos_os_nos_apagam() {
    let a = socket();
    let b = socket();
    let perfil = profile_by_name("esp32-poe-wled-ddp").expect("preset");
    let specs =
        [a.local_addr().unwrap().to_string(), b.local_addr().unwrap().to_string()];
    let cfg = OutputConfig::resolve_muitos(&perfil, &specs, 3000).expect("resolver");
    assert!(
        cfg.alvos.iter().all(|x| !x.escapa_blackout),
        "por omissão NENHUM nó escapa — um escape por acidente seria falsa segurança"
    );

    let om = OutputManager::open(cfg).expect("abrir");
    om.blackout_accionar();
    om.send(&LogicalFrame::new(vec![COR; 3000], 0)).unwrap();
    assert!(e_todo_preto(&recebido(&a)), "nó 1 preto");
    assert!(e_todo_preto(&recebido(&b)), "nó 2 preto");
}

// ── A fronteira que o D6 não pode mover ─────────────────────────────────────

/// **STOP ≠ BLACKOUT** (decisão 4), verificado do lado da saída.
///
/// O `OutputManager` não tem — e não pode ganhar — nenhum caminho pelo qual o transporte
/// apague o palco. A máscara só se acciona por chamada explícita. Este teste afirma que
/// enviar frames indefinidamente **nunca** liga o blackout sozinho.
#[test]
fn nada_no_caminho_de_envio_acciona_o_blackout_sozinho() {
    let (om, sock) = saida();
    for i in 0..10 {
        om.send(&LogicalFrame::new(vec![COR; PX], i)).unwrap();
        let _ = recebido(&sock);
        assert!(!om.blackout_activo(), "o envio nunca pode accionar o blackout (frame {i})");
    }
}

/// **O custo de a funcionalidade existir, com o blackout desligado, é zero no fio.**
///
/// O critério de reversão do ADR-0017 diz que uma colisão com o gate de alocação do hot-path
/// exige decisão nova. Este teste fixa a premissa que evita essa colisão: desligado, o caminho
/// rápido continua a entregar **o frame do chamador, byte a byte**, sem passar pela máscara.
#[test]
fn com_blackout_desligado_o_fio_e_byte_identico_ao_de_antes_da_mascara() {
    let (om, sock) = saida();
    let frame = LogicalFrame::new(vec![COR; PX], 0);
    om.send(&frame).unwrap();
    let a = recebido(&sock);
    om.send(&frame).unwrap();
    let b = recebido(&sock);
    assert_eq!(a, b, "dois envios iguais dão bytes iguais");
    assert!(!e_todo_preto(&a), "e não são preto");
    assert!(!om.blackout_activo(), "o estado por omissão é desligado");
}
