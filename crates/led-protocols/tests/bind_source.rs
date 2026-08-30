//! Proves a sender bound to an explicit local address **sends from that address** — read as
//! the source address on the receiving socket, never as "the constructor accepted the value".
//!
//! ## O que está sob teste, e porque é o endereço COMPLETO
//!
//! A propriedade é *"os datagramas saem de onde foi declarado"*. Num host com **uma só**
//! morada que alcança o alvo — todo o runner de CI, e o loopback em qualquer máquina — o
//! *routing table* escolhe exactamente o mesmo IP que nós declaramos, e uma asserção só sobre
//! o IP **não distinguiria** um bind explícito de um bind wildcard: continuaria verde com o
//! defeito reposto. É a forma mais barata do KB-012.
//!
//! Por isso a asserção é sobre o **endereço local completo**, IP *e* porta: a porta declarada é
//! a componente que o wildcard não pode reproduzir (ele pede uma porta efémera), e é ela que
//! faz a falsificação reprovar em qualquer plataforma. As duas componentes viajam pelo mesmo
//! caminho de código — o `Option<SocketAddr>` que chega ao `bind_sender` — portanto provar uma
//! prova o caminho da outra.
//!
//! **O que isto não prova:** que, num host multi-homed, o IP declarado vence o do *routing
//! table*. Isso exige duas moradas que alcancem o alvo, o que um runner de CI não tem — está
//! no teste `#[ignore]` do fim, que se corre na bancada dual-homed com a morada em variável de
//! ambiente. Fica declarado em vez de arredondado.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use led_core::{ColorFormat, DeviceDriver, PixelColor, RgbOrder, UniverseData};
use led_protocols::{packet, ArtNetDevice, DdpDevice, SacnDevice};

/// Um receptor em `127.0.0.1`, com prazo — um teste que pendura perde o diagnóstico.
fn receptor() -> UdpSocket {
    let rx = UdpSocket::bind("127.0.0.1:0").expect("receptor");
    rx.set_read_timeout(Some(Duration::from_secs(5))).expect("prazo");
    rx
}

/// Uma porta local livre para o **remetente**.
///
/// Obtida pelo caminho normal (bind em `:0`, ler, largar). A janela entre largar e voltar a
/// pedir é a corrida habitual deste idioma; se alguém a ocupar entretanto, o bind falha com
/// `AddrInUse` e o teste diz isso — nunca passa por engano.
fn porta_livre() -> u16 {
    UdpSocket::bind("127.0.0.1:0").expect("porta livre").local_addr().unwrap().port()
}

/// A morada local declarada ao remetente.
fn origem_declarada() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], porta_livre()))
}

fn origem_observada(rx: &UdpSocket) -> SocketAddr {
    let mut buf = [0u8; 1600];
    let (_, origem) = rx.recv_from(&mut buf).expect("o datagrama tem de chegar");
    origem
}

fn universo_cheio(universe: u16) -> UniverseData {
    UniverseData { universe, data: vec![7u8; packet::DMX_SLOTS] }
}

/// A mensagem nomeia o defeito em vez de mostrar dois números soltos.
fn assert_origem(observada: SocketAddr, declarada: SocketAddr, protocolo: &str) {
    assert_eq!(
        observada, declarada,
        "{protocolo}: o datagrama saiu de {observada} e foi declarado {declarada} — \
         a morada de origem declarada NAO chegou ao socket (bind wildcard?)"
    );
}

#[test]
fn ddp_sends_from_the_declared_local_address() {
    let rx = receptor();
    let declarada = origem_declarada();
    let mut dev = DdpDevice::bound(
        rx.local_addr().unwrap(),
        0,
        ColorFormat::Rgb(RgbOrder::Rgb),
        Some(declarada),
    )
    .expect("abrir DDP na morada declarada");

    // O socket sabe onde está antes de qualquer envio — mas isto sozinho não distingue
    // declarado de escolhido (depois do `connect`, o wildcard também mostra 127.0.0.1).
    assert_eq!(dev.local_addr().unwrap(), declarada, "o bind é o que foi pedido");

    dev.send_pixels(&[PixelColor::rgb(1, 2, 3)]).expect("enviar");
    assert_origem(origem_observada(&rx), declarada, "DDP");
}

#[test]
fn artnet_sends_from_the_declared_local_address() {
    let rx = receptor();
    let declarada = origem_declarada();
    let dev = ArtNetDevice::unicast_bound(1, rx.local_addr().unwrap(), Some(declarada))
        .expect("abrir Art-Net na morada declarada");

    dev.send_physical(&[universo_cheio(0)]).expect("enviar");
    assert_origem(origem_observada(&rx), declarada, "Art-Net");
}

#[test]
fn sacn_sends_from_the_declared_local_address() {
    let rx = receptor();
    let declarada = origem_declarada();
    let dev = SacnDevice::unicast_bound(
        1,
        rx.local_addr().unwrap(),
        [0x33; 16],
        "lumyx-bind-test",
        Some(declarada),
    )
    .expect("abrir sACN na morada declarada");

    dev.send_physical(&[universo_cheio(1)]).expect("enviar");
    assert_origem(origem_observada(&rx), declarada, "sACN");
}

/// **Controlo negativo:** sem declaração, a origem é a do *routing table* — e a porta é
/// efémera, nunca a que este teste escolheu.
///
/// Sem ele, os três testes acima poderiam estar a afirmar uma coisa que acontece sempre. Este
/// fixa que a porta declarada **não** é o que o wildcard produz, que é exactamente a diferença
/// de que a falsificação depende.
#[test]
fn without_a_declaration_the_source_is_not_the_one_we_chose() {
    let rx = receptor();
    let nao_declarada = origem_declarada(); // livre, e deliberadamente NÃO passada ao device
    let mut dev = DdpDevice::new(rx.local_addr().unwrap(), 0).expect("abrir DDP sem declaração");

    dev.send_pixels(&[PixelColor::rgb(1, 2, 3)]).expect("enviar");
    let observada = origem_observada(&rx);
    assert_eq!(observada.ip(), nao_declarada.ip(), "num host single-homed o IP coincide");
    assert_ne!(
        observada.port(),
        nao_declarada.port(),
        "a porta efémera do wildcard não pode ser a porta que este teste escolheu — \
         se for, o discriminante deixou de discriminar"
    );
}

// ── A componente que a CI não consegue exercitar ─────────────────────────────

/// **Multi-homed:** o IP declarado vence o que o *routing table* escolheria.
///
/// `#[ignore]` porque exige um host com **duas** moradas que alcancem o alvo — a bancada
/// LUMYX é dual-homed na mesma sub-rede (`en0` 192.168.2.32 WiFi, `en7` 192.168.2.163
/// Ethernet), um runner de CI não é. É instrumento, não gate: a precedência é a do
/// `bench_latency.rs`/`bench_contention.rs`, que medem e não votam.
///
/// ```sh
/// LUMYX_BIND_SRC=192.168.2.163 LUMYX_BIND_DST=192.168.2.32 \
///   cargo test -p led-protocols --test bind_source -- --ignored --nocapture
/// ```
///
/// `LUMYX_BIND_DST` é **outra** morada local deste host, usada como alvo: assim o datagrama
/// nunca sai da máquina e o teste não depende de haver um nó ligado.
///
/// # A premissa tem guarda própria
///
/// Antes de afirmar seja o que for, o teste confirma que **este binário consegue entregar** um
/// datagrama entre duas moradas de rede local. Medido em 2026-08-28: no macOS 14 desta bancada
/// nenhum binário de `cargo test` o consegue — o `send` devolve `Ok` e nada chega (privacidade
/// *Local Network* do macOS, concedida por binário; o `python3` do sistema entrega, um binário
/// acabado de compilar não). Sem a guarda, isso apareceria como *"o IP declarado não venceu"*,
/// que é uma acusação falsa ao código. Com ela, o teste diz que **não mediu**.
#[test]
#[ignore = "exige um host multi-homed; ver LUMYX_BIND_SRC/LUMYX_BIND_DST"]
fn on_a_multi_homed_host_the_declared_ip_beats_the_routing_table() {
    let src: std::net::IpAddr = std::env::var("LUMYX_BIND_SRC")
        .expect("LUMYX_BIND_SRC (morada local a declarar)")
        .parse()
        .expect("LUMYX_BIND_SRC é um IP");
    let dst: std::net::IpAddr = std::env::var("LUMYX_BIND_DST")
        .expect("LUMYX_BIND_DST (outra morada local deste host, usada como alvo)")
        .parse()
        .expect("LUMYX_BIND_DST é um IP");

    let rx = UdpSocket::bind(SocketAddr::from((dst, 0))).expect("receptor na 2.ª morada");
    rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let alvo = rx.local_addr().unwrap();

    // O que o *routing table* escolheria para este alvo, medido e não presumido: um socket
    // wildcard ligado ao alvo já revela a origem que o kernel atribuiu.
    let sonda = UdpSocket::bind("0.0.0.0:0").unwrap();
    sonda.connect(alvo).unwrap();
    let escolhida = sonda.local_addr().unwrap().ip();
    assert_ne!(
        escolhida, src,
        "o routing table já escolhe {src} para {dst}: este host não discrimina esta \
         combinação, escolha outra morada em LUMYX_BIND_SRC"
    );

    // ── A premissa: este binário entrega mesmo na rede local? ────────────────
    let base = UdpSocket::bind(SocketAddr::from((escolhida, 0))).expect("sonda de base");
    base.send_to(b"premissa", alvo).expect("envio da premissa");
    let mut buf = [0u8; 64];
    rx.recv_from(&mut buf).unwrap_or_else(|e| {
        panic!(
            "NAO MEDIDO: este binário não entrega datagramas entre moradas de rede local \
             ({escolhida} -> {alvo}): {e}. No macOS isto é a privacidade *Local Network*, \
             concedida por binário. O teste não chegou a exercitar o bind."
        )
    });

    let mut dev = DdpDevice::bound(
        alvo,
        0,
        ColorFormat::Rgb(RgbOrder::Rgb),
        Some(SocketAddr::from((src, 0))),
    )
    .expect("abrir DDP na morada declarada");
    dev.send_pixels(&[PixelColor::rgb(1, 2, 3)]).expect("enviar");

    let observada = origem_observada(&rx).ip();
    println!(
        "multi-homed: routing table escolheria {escolhida}; declarado {src}; observado {observada}"
    );
    assert_eq!(
        observada, src,
        "o IP declarado tem de vencer o do routing table ({escolhida})"
    );
}
