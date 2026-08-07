//! GS4.1 — a camada de saída do daemon: **uma abstração, três protocolos**.
//!
//! ## O que muda no daemon quando o protocolo muda: **nada**
//!
//! O daemon fala com [`OutputManager::send`] e mais nada. DDP, Art-Net e sACN são escolhidos
//! por **configuração** ([`OutputConfig`]), e a diferença entre eles morre aqui dentro. Trocar
//! `--output ddp://…` por `--output artnet://…` não toca uma linha de `run.rs`.
//!
//! ## Nenhuma segunda implementação
//!
//! Este módulo **não** serializa pacotes. DDP reusa `led_player::DdpOutput`, Art-Net e sACN
//! reusam `led_protocols::{ArtNetDevice, SacnDevice}` por trás do `Hal` — o mesmo caminho que
//! o `led-player` já validou em hardware real (94/94 frames, 2026-07-20). Um segundo
//! serializador seria uma segunda coisa para divergir.
//!
//! ## "Nenhum frame perdido por erro interno"
//!
//! Um erro de envio é **contado e devolvido**, nunca engolido. E não derruba o laço: a regra
//! de degradação segura do `control-protocol.md` diz que o show continua. A distinção que
//! importa é entre *falhar em silêncio* (proibido) e *falhar, registar e prosseguir*
//! (correto) — [`OutputStats`] existe para que a diferença seja observável.

use led_core::{ColorFormat, CompiledLayout, LogicalFrame, OutputError, ProtocolOutput, RgbOrder};
use led_hal::Hal;
use led_hardware_profile::{HardwareProfile, Protocol, Transport};
use led_player::linear_assignments;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Porta padrão de cada protocolo, para o caso de a configuração não a dar.
pub const DDP_PORT: u16 = 4048;
pub const ARTNET_PORT: u16 = 6454;
pub const SACN_PORT: u16 = 5568;

/// Os três protocolos de saída.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputProtocol {
    /// Pixel-nativo, 487 px/datagrama — ~3× menos pacotes que Art-Net para o mesmo rig.
    /// **É o caminho validado em hardware** (WLED, 2026-07-20).
    Ddp,
    /// ArtDmx unicast, 170 px/universo. Também **validado em hardware** (2026-07-23).
    ArtNet,
    /// E1.31 unicast. ⚠️ **Bloqueado no rig atual por firmware**, não por nós: o WLED 16.0.1
    /// não faz bind do receiver na porta 5568 (investigado byte-a-byte em 2026-07-23; um
    /// sender de referência independente falha igual). O código está correto e testado; o
    /// que falta é do outro lado.
    Sacn,
}

impl OutputProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputProtocol::Ddp => "ddp",
            OutputProtocol::ArtNet => "artnet",
            OutputProtocol::Sacn => "sacn",
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            OutputProtocol::Ddp => DDP_PORT,
            OutputProtocol::ArtNet => ARTNET_PORT,
            OutputProtocol::Sacn => SACN_PORT,
        }
    }
    /// O protocolo declarado pelo `HardwareProfile`. **Uma só tradução, num só sítio.**
    pub fn from_profile(p: Protocol) -> Self {
        match p {
            Protocol::Ddp => OutputProtocol::Ddp,
            Protocol::ArtNet => OutputProtocol::ArtNet,
            Protocol::Sacn => OutputProtocol::Sacn,
        }
    }
    /// A tradução inversa, para reusar as derivações do profile.
    pub fn to_profile(self) -> Protocol {
        match self {
            OutputProtocol::Ddp => Protocol::Ddp,
            OutputProtocol::ArtNet => Protocol::ArtNet,
            OutputProtocol::Sacn => Protocol::Sacn,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ddp" => Some(OutputProtocol::Ddp),
            "artnet" | "art-net" => Some(OutputProtocol::ArtNet),
            "sacn" | "e131" => Some(OutputProtocol::Sacn),
            _ => None,
        }
    }
}

/// Como a saída é construída. **É isto que o daemon recebe**; ele nunca vê um driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputConfig {
    pub protocol: OutputProtocol,
    pub addr: SocketAddr,
    pub pixel_count: usize,
    /// Só Art-Net/sACN. O DDP endereça por byte e ignora universos.
    pub first_universe: u16,
    /// Formato e **ordem de canais** do nó. Vem do `HardwareProfile` quando há um.
    ///
    /// ⚠️ Antes do GS4.3 isto era `RgbOrder::Rgb` **fixo no código** — o que estava errado
    /// para o rig real, cujos WLED são **GRB**. Um nó GRB alimentado com bytes RGB acende
    /// vermelho onde devia acender verde. O profile já declarava a ordem certa desde o
    /// ADR-0018; faltava alguém consultá-la.
    pub color: ColorFormat,
    /// Pixels por universo **declarados pelo nó**. Sem profile, o valor clássico de 170.
    pub pixels_per_universe: u16,
}

impl OutputConfig {
    /// Analisa `proto://host[:porta]`, ex.: `ddp://192.168.2.156` ou
    /// `artnet://192.168.2.156:6454`.
    ///
    /// A porta é opcional e cai no padrão do protocolo — escrever `:4048` à mão em cada
    /// invocação é uma oportunidade a mais de errar um número que já é conhecido.
    pub fn parse(s: &str, pixel_count: usize, first_universe: u16) -> Result<Self, String> {
        let (proto_s, resto) = s.split_once("://").ok_or("formato: proto://host[:porta]")?;
        let protocol = OutputProtocol::parse(proto_s)
            .ok_or_else(|| format!("protocolo desconhecido `{proto_s}` (ddp|artnet|sacn)"))?;
        let com_porta = if resto.contains(':') {
            resto.to_string()
        } else {
            format!("{resto}:{}", protocol.default_port())
        };
        let addr: SocketAddr = com_porta
            .parse()
            .map_err(|_| format!("endereço inválido `{com_porta}` (use IP:porta)"))?;
        if pixel_count == 0 {
            return Err("pixel_count tem de ser > 0".into());
        }
        Ok(Self {
            protocol,
            addr,
            pixel_count,
            first_universe,
            color: ColorFormat::Rgb(RgbOrder::Rgb),
            pixels_per_universe: 170,
        })
    }

    /// Constrói a saída a partir de um [`HardwareProfile`]. **É este o caminho preferido.**
    ///
    /// # Porque não há aqui um segundo caminho para o fio
    ///
    /// `from_profile` e [`OutputConfig::parse`] produzem o **mesmo tipo**, e o
    /// [`OutputManager`] continua a ter um único construtor. O que muda é a *procedência* dos
    /// campos: com profile vêm declarados pelo nó, sem profile vêm de omissões documentadas.
    /// Nada no caminho dos bytes se duplica.
    ///
    /// O endereço e o primeiro universo **não** saem do profile: são da *instância*, não do
    /// tipo de hardware — é a separação que o ADR-0018 fixou quando decidiu que `Identity`
    /// não tem endereço.
    pub fn from_profile(
        profile: &HardwareProfile,
        addr: SocketAddr,
        pixel_count: usize,
        first_universe: u16,
    ) -> Result<Self, String> {
        if pixel_count == 0 {
            return Err("pixel_count tem de ser > 0".into());
        }
        if pixel_count as u32 > profile.limits.max_pixels {
            return Err(format!(
                "o show tem {pixel_count} px e o nó `{}` declara no máximo {}",
                profile.identity.model, profile.limits.max_pixels
            ));
        }
        if !profile.transport.heartbeat_is_safe() {
            return Err(format!(
                "heartbeat declarado de {} ms nao respeita o teto de {} ms do LUMYX_GOSL",
                profile.transport.heartbeat_ms,
                Transport::MAX_GAP_MS
            ));
        }
        Ok(Self {
            protocol: OutputProtocol::from_profile(profile.capabilities.protocol),
            addr,
            pixel_count,
            first_universe,
            color: profile.capabilities.color,
            pixels_per_universe: profile.limits.pixels_per_universe,
        })
    }

    /// A ordem de canais, seja qual for o formato.
    pub fn rgb_order(&self) -> RgbOrder {
        match self.color {
            ColorFormat::Rgb(o) => o,
            ColorFormat::Rgbw(o, _) => o,
        }
    }

    /// Em quantos datagramas um frame se parte, **derivado do MTU do profile**.
    pub fn datagrams_per_frame(&self, transport: &Transport) -> u32 {
        transport.datagrams_for(
            self.pixel_count as u32,
            self.protocol.to_profile(),
            self.color,
            self.pixels_per_universe,
        )
    }
}

/// Contadores de saída. **Observáveis de propósito**: um erro engolido é indistinguível de
/// sucesso, e é isso que torna um palco escuro difícil de diagnosticar.
#[derive(Debug, Default)]
pub struct OutputStats {
    pub frames_sent: AtomicU64,
    pub errors: AtomicU64,
}

impl OutputStats {
    pub fn frames(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }
}

/// A saída do daemon. Uma abstração; o protocolo vive só na construção.
pub struct OutputManager {
    out: Box<dyn ProtocolOutput>,
    cfg: OutputConfig,
    stats: OutputStats,
}

impl OutputManager {
    /// Constrói a saída a partir da configuração. **É o único sítio do projeto que sabe qual
    /// protocolo está em uso.**
    pub fn open(cfg: OutputConfig) -> std::io::Result<Self> {
        let out: Box<dyn ProtocolOutput> = match cfg.protocol {
            OutputProtocol::Ddp => Box::new(led_player::DdpOutput::with_format(
                cfg.addr,
                cfg.pixel_count,
                cfg.color,
            )?),
            OutputProtocol::ArtNet => {
                let assigns =
                    linear_assignments(cfg.pixel_count, 1, cfg.first_universe, cfg.rgb_order());
                let layout = CompiledLayout::compile(&assigns);
                let dev = led_protocols::ArtNetDevice::unicast(1, cfg.addr)?;
                Box::new(Hal::new(layout, vec![dev]))
            }
            OutputProtocol::Sacn => {
                let assigns =
                    linear_assignments(cfg.pixel_count, 1, cfg.first_universe, cfg.rgb_order());
                let layout = CompiledLayout::compile(&assigns);
                // CID fixo e nome próprio: um receptor E1.31 distingue fontes por CID, e dois
                // senders com o mesmo CID seriam indistinguíveis no diagnóstico.
                let cid = *b"LUMYX-DAEMON-001";
                let dev = led_protocols::SacnDevice::unicast(1, cfg.addr, cid, "lumyx-daemon")?;
                Box::new(Hal::new(layout, vec![dev]))
            }
        };
        Ok(Self { out, cfg, stats: OutputStats::default() })
    }

    pub fn config(&self) -> &OutputConfig {
        &self.cfg
    }
    pub fn stats(&self) -> &OutputStats {
        &self.stats
    }

    /// Envia um frame. Conta sucesso e erro; **devolve o erro**, nunca o engole.
    pub fn send(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        match self.out.send_frame(frame) {
            Ok(()) => {
                self.stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                self.stats.errors.fetch_add(1, Ordering::Relaxed);
                Err(e)
            }
        }
    }
}

/// **`OutputManager` é ele próprio um `ProtocolOutput`.**
///
/// É isto que faz o `Heartbeat` do `led-hal` usar exatamente o mesmo remetente que o laço:
/// o keep-alive conta nas mesmas estatísticas e sai pelo mesmo socket. Um segundo caminho
/// para o heartbeat seria a definição de caminho paralelo.
impl led_core::ProtocolOutput for OutputManager {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        self.send(frame)
    }
    fn universe_count(&self) -> u16 {
        self.out.universe_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::PixelColor;
    use std::net::UdpSocket;

    fn frame(n: usize, v: u8) -> LogicalFrame {
        LogicalFrame::new(vec![PixelColor { r: v, g: v / 2, b: v / 3 }; n], 0)
    }

    /// Abre um socket UDP num porto livre e devolve `(socket, addr)`.
    fn recetor() -> (UdpSocket, SocketAddr) {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let a = s.local_addr().unwrap();
        (s, a)
    }

    #[test]
    fn parse_de_configuracao() {
        let c = OutputConfig::parse("ddp://127.0.0.1", 720, 1).unwrap();
        assert_eq!(c.protocol, OutputProtocol::Ddp);
        assert_eq!(c.addr.port(), DDP_PORT, "porta omitida cai no padrão do protocolo");

        assert_eq!(
            OutputConfig::parse("artnet://127.0.0.1", 10, 1).unwrap().addr.port(),
            ARTNET_PORT
        );
        assert_eq!(OutputConfig::parse("sacn://127.0.0.1", 10, 1).unwrap().addr.port(), SACN_PORT);
        assert_eq!(
            OutputConfig::parse("ddp://127.0.0.1:9999", 10, 1).unwrap().addr.port(),
            9999,
            "porta explícita vence o padrão"
        );
    }

    #[test]
    fn parse_recusa_o_que_esta_errado() {
        for s in ["127.0.0.1", "xyz://127.0.0.1", "ddp://nao-e-ip", "ddp://"] {
            assert!(OutputConfig::parse(s, 10, 1).is_err(), "devia recusar: {s}");
        }
        assert!(OutputConfig::parse("ddp://127.0.0.1", 0, 1).is_err(), "0 pixels não faz sentido");
    }

    #[test]
    fn aliases_de_protocolo() {
        assert_eq!(OutputProtocol::parse("art-net"), Some(OutputProtocol::ArtNet));
        assert_eq!(OutputProtocol::parse("E131"), Some(OutputProtocol::Sacn));
        assert_eq!(OutputProtocol::parse("DDP"), Some(OutputProtocol::Ddp));
        assert_eq!(OutputProtocol::parse("tcp"), None);
    }

    /// **Bytes reais no fio, nos três protocolos.** Não é mock: um socket UDP recebe.
    ///
    /// É este teste que responde a "o frame saiu do daemon" — separado de "o hardware
    /// recebeu", que exige rig e não pode ser afirmado aqui.
    #[test]
    fn os_tres_protocolos_poem_bytes_no_fio() {
        for proto in ["ddp", "artnet", "sacn"] {
            let (sock, addr) = recetor();
            let cfg = OutputConfig::parse(&format!("{proto}://{addr}"), 8, 1).unwrap();
            let om = OutputManager::open(cfg).unwrap_or_else(|e| panic!("{proto}: {e}"));

            om.send(&frame(8, 200)).unwrap_or_else(|e| panic!("{proto}: {e:?}"));

            let mut buf = [0u8; 2048];
            let n = sock
                .recv(&mut buf)
                .unwrap_or_else(|e| panic!("{proto}: nada chegou ao fio: {e}"));
            assert!(n > 0, "{proto}: datagrama vazio");
            assert_eq!(om.stats().frames(), 1, "{proto}: frame contado");
            assert_eq!(om.stats().errors(), 0, "{proto}: sem erros");
        }
    }

    /// O primeiro byte de cada protocolo é diferente — prova que a seleção por configuração
    /// escolhe mesmo caminhos distintos, e não o mesmo três vezes.
    #[test]
    fn a_configuracao_escolhe_caminhos_realmente_diferentes() {
        let mut primeiros = Vec::new();
        for proto in ["ddp", "artnet", "sacn"] {
            let (sock, addr) = recetor();
            let cfg = OutputConfig::parse(&format!("{proto}://{addr}"), 8, 1).unwrap();
            let om = OutputManager::open(cfg).unwrap();
            om.send(&frame(8, 128)).unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.recv(&mut buf).unwrap();
            primeiros.push((proto, buf[..n.min(12)].to_vec()));
        }
        // Art-Net começa por "Art-Net\0"; sACN por 0x0010 + "ASC-E1.17"; DDP por flags.
        let artnet = &primeiros.iter().find(|(p, _)| *p == "artnet").unwrap().1;
        assert!(artnet.starts_with(b"Art-Net"), "cabeçalho Art-Net: {artnet:?}");
        let sacn = &primeiros.iter().find(|(p, _)| *p == "sacn").unwrap().1;
        assert!(sacn.windows(3).any(|w| w == b"ASC"), "PID do E1.31: {sacn:?}");
        let ddp = &primeiros.iter().find(|(p, _)| *p == "ddp").unwrap().1;
        assert_ne!(&ddp[..7], b"Art-Net", "DDP não pode sair como Art-Net");
    }

    /// Erro de envio é **contado e devolvido** — nunca engolido.
    #[test]
    fn erro_de_envio_e_contado_e_devolvido() {
        // Porta 9 (discard) num endereço não roteável dá erro de envio em muitos sistemas;
        // onde não der, o teste continua válido porque afirma a COERÊNCIA dos contadores,
        // não que a falha aconteça.
        let cfg = OutputConfig::parse("ddp://127.0.0.1:9", 8, 1).unwrap();
        let om = OutputManager::open(cfg).unwrap();
        let r = om.send(&frame(8, 10));
        let (f, e) = (om.stats().frames(), om.stats().errors());
        assert_eq!(f + e, 1, "todo envio conta exatamente uma vez — nunca desaparece");
        assert_eq!(r.is_err(), e == 1, "o contador de erro casa com o Result devolvido");
    }

    #[test]
    fn contadores_acumulam_por_frame() {
        let (sock, addr) = recetor();
        let cfg = OutputConfig::parse(&format!("ddp://{addr}"), 4, 1).unwrap();
        let om = OutputManager::open(cfg).unwrap();
        for i in 0..5u8 {
            om.send(&frame(4, i)).unwrap();
        }
        assert_eq!(om.stats().frames(), 5);
        let mut buf = [0u8; 2048];
        for _ in 0..5 {
            assert!(sock.recv(&mut buf).is_ok(), "os 5 datagramas têm de chegar");
        }
    }
}
