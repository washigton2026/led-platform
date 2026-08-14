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
use led_hal::{CalibrationLut, Hal};
use led_hardware_profile::{
    Calibration as ProfileCalibration, HardwareProfile, Protocol, Transport,
};
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
    /// **Todos os protocolos que o `OutputManager` sabe construir** (ADR-0024).
    ///
    /// Existe para que o `Available{}` passado ao validador não possa divergir do `match` de
    /// [`OutputManager::open`] — que é exaustivo por construção. Um protocolo novo sem entrada
    /// aqui faria o daemon **recusar profiles que sabe construir**; há um teste que obriga
    /// esta lista a cobrir todas as variantes.
    pub const ALL: [OutputProtocol; 3] =
        [OutputProtocol::Ddp, OutputProtocol::ArtNet, OutputProtocol::Sacn];

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

/// **Um nó físico**: onde está, e que fatia do show lhe pertence (ADR-0029).
///
/// É tudo — e só — o que é da **instância**. O ADR-0018 já tinha traçado esta linha ao
/// manter `address` e `first_universe` fora do `HardwareProfile`: o profile descreve um
/// *tipo* de hardware, e cinco nós do mesmo tipo diferem exactamente nestes campos.
///
/// Por isso a `Calibration` **não** está aqui: ela é do profile, e cinco nós do mesmo preset
/// partilham-na por construção. Pô-la neste struct sugeriria que pode divergir por nó — e
/// então o ADR-0019 teria de ser revisitado. Não tem, porque não pode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alvo {
    pub addr: SocketAddr,
    /// Só Art-Net/sACN. O DDP endereça por byte e ignora universos.
    pub first_universe: u16,
    /// Onde começa a fatia deste nó **no show**, em pixels.
    ///
    /// **Derivado, nunca declarado** — o operador dá endereços, e a repartição sai do
    /// `max_pixels` do profile. É a mesma disciplina do `pixels_per_datagram`, que deriva do
    /// MTU em vez de viver escrito ao lado dele: a mesma verdade em dois sítios apodrece no
    /// segundo (GS4.3).
    ///
    /// Com um alvo é sempre `0`, e foi isso que escondeu o TD-016 até agora.
    pub pixel_offset: u32,
    /// Quantos pixels **deste** nó. A soma dos alvos é o show; nenhum deles é o show.
    pub pixel_count: usize,
}

/// Como a saída é construída. **É isto que o daemon recebe**; ele nunca vê um driver.
///
/// Sem `Eq`: a calibração é `f32`, e igualdade total sobre vírgula flutuante não existe.
/// `PartialEq` chega para os testes compararem duas configurações.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputConfig {
    pub protocol: OutputProtocol,
    /// Os nós, por ordem. **Nunca vazio** — uma saída sem alvo não é uma saída.
    pub alvos: Vec<Alvo>,
    /// Os pixels do **show**, não de um nó. É a soma das fatias.
    pub pixel_count: usize,
    /// Formato e **ordem de canais** do nó. Vem do `HardwareProfile` quando há um.
    ///
    /// ⚠️ Antes do GS4.3 isto era `RgbOrder::Rgb` **fixo no código** — o que estava errado
    /// para o rig real, cujos WLED são **GRB**. Um nó GRB alimentado com bytes RGB acende
    /// vermelho onde devia acender verde. O profile já declarava a ordem certa desde o
    /// ADR-0018; faltava alguém consultá-la.
    pub color: ColorFormat,
    /// Pixels por universo **declarados pelo nó**.
    pub pixels_per_universe: u16,
    /// MTU e heartbeat declarados. **A fragmentação deriva daqui** — não é um segundo número.
    pub transport: Transport,
    /// Correção óptica declarada pelo nó (ADR-0019 Emenda 1). **Vem do profile**, como tudo
    /// o resto que é físico — não há aqui um segundo `Calibration`.
    pub calibration: ProfileCalibration,
    /// O nó **declara** responder a descoberta (ArtPoll). Quando é `false`, sondá-lo e
    /// concluir "ausente" seria punir o nó por se comportar como declarou.
    pub supports_discovery: bool,
}

impl OutputConfig {
    /// Resolve a configuração da saída: **tudo o que é físico vem do [`HardwareProfile`]**.
    ///
    /// `spec` é o **endereço da instância** — `host[:porta]` ou `proto://host[:porta]`. É a
    /// única coisa que a CLI ainda diz sobre a saída, porque é a única que **não** é do tipo
    /// de hardware: o ADR-0018 já tinha decidido que `Identity` não tem endereço.
    ///
    /// # O esquema, quando escrito, é conferido — não é uma segunda fonte
    ///
    /// `ddp://…` continua a ser aceite, mas apenas se **concordar** com o protocolo que o
    /// profile declara; discordar é erro, não é uma escolha. Assim o operador pode continuar a
    /// escrever o que já escrevia sem que exista um segundo sítio de onde o protocolo possa
    /// vir. O profile manda; o esquema, quando existe, é uma afirmação a verificar.
    ///
    /// # A porta não é configuração física
    ///
    /// 4048/6454/5568 são **identidade dos protocolos** (IANA/spec), não propriedades deste
    /// nó — mudam quando o protocolo muda, e o protocolo vem do profile. Por isso vivem em
    /// [`OutputProtocol::default_port`] e não no `HardwareProfile`.
    pub fn resolve(
        profile: &HardwareProfile,
        spec: &str,
        pixel_count: usize,
        first_universe: u16,
    ) -> Result<Self, String> {
        let protocol = OutputProtocol::from_profile(profile.capabilities.protocol);

        let (esquema, resto) = match spec.split_once("://") {
            Some((e, r)) => (Some(e), r),
            None => (None, spec),
        };
        if let Some(e) = esquema {
            let escrito = OutputProtocol::parse(e)
                .ok_or_else(|| format!("protocolo desconhecido `{e}` (ddp|artnet|sacn)"))?;
            if escrito != protocol {
                return Err(format!(
                    "`{e}://` contradiz o profile `{}`, que declara {}. \
                     O protocolo vem do HardwareProfile — corrija o esquema ou o preset",
                    profile.identity.model,
                    protocol.as_str()
                ));
            }
        }

        let com_porta = if resto.contains(':') {
            resto.to_string()
        } else {
            format!("{resto}:{}", protocol.default_port())
        };
        let addr: SocketAddr = com_porta
            .parse()
            .map_err(|_| format!("endereço inválido `{com_porta}` (use IP[:porta])"))?;

        Self::from_profile(profile, addr, pixel_count, first_universe)
    }

    /// Constrói a saída a partir de um [`HardwareProfile`]. **É o único construtor** —
    /// [`OutputConfig::resolve`] delega aqui depois de resolver o endereço.
    ///
    /// # Não existe caminho sem profile
    ///
    /// Até ao GS4.3 havia um `parse` que preenchia cor e universos com omissões (`RgbOrder::Rgb`
    /// e 170). Essas omissões **estavam erradas** para o rig real, cujos WLED são GRB, e um
    /// valor errado por omissão é pior que a ausência de valor: parece configuração. Foram
    /// removidas, e com elas a possibilidade de o daemon adivinhar o hardware.
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
        // ── ADR-0024: validação ESTÁTICA, antes de existir saída ──────────────
        //
        // Corre aqui porque o laço já chama esta construção **antes** do pré-voo e do `Arm`:
        // um profile com erro devolve `Err`, o palco não abre, e o daemon termina em
        // `NeverStarted`. Assim um profile inválido **nunca chega a `Ready`** sem que o
        // `PreflightReport` (congelado na GS1.6) precise de um quarto campo.
        //
        // `Warning` **não** recusa: o preset RGBW-sobre-DDP avisa por desenho, e bloqueá-lo
        // mudaria o significado de `Warning` fixado no ADR-0018. O aviso vai para o journal
        // pelo chamador (`abrir_palco`), que é quem tem o journal.
        let disponiveis = Self::drivers_disponiveis();
        let v = led_hardware_profile::validate(
            profile,
            &led_hardware_profile::Available {
                interfaces: &disponiveis.0,
                protocols: &disponiveis.1,
            },
        );
        if v.has_errors() {
            let quais: Vec<String> = v.errors().map(|f| format!("{f:?}")).collect();
            return Err(format!(
                "profile `{}` invalido (ADR-0024): {}",
                profile.identity.model,
                quais.join("; ")
            ));
        }

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
            // Um alvo. A repartição por N nós é a fatia seguinte do ADR-0029; aqui a
            // estrutura passa a poder exprimi-la, e o comportamento não muda: com um alvo,
            // `pixel_offset` é 0 e a fatia é o show inteiro.
            alvos: vec![Alvo { addr, first_universe, pixel_offset: 0, pixel_count }],
            pixel_count,
            color: profile.capabilities.color,
            pixels_per_universe: profile.limits.pixels_per_universe,
            transport: profile.transport,
            calibration: profile.calibration,
            supports_discovery: profile.capabilities.supports_discovery,
        })
    }

    /// O que o `OutputManager` **sabe construir hoje** — a lista que o validador recebe como
    /// dado (ADR-0018 mantém o crate leaf; ADR-0024 fixa que é o `OutputManager` quem a dá).
    ///
    /// `Ethernet` e `WiFi` têm driver: as duas são UDP sobre IP e o daemon fala com ambas. O
    /// bloqueio do ADR-0005 é do `WifiBlockGuard`, contra a interface **do host**, no pré-voo
    /// — pô-lo aqui seria duplicar o enforcement no sítio errado. `Spi`/`Pwm` não têm driver.
    fn drivers_disponiveis() -> ([led_hardware_profile::OutputInterface; 2], [Protocol; 3]) {
        use led_hardware_profile::OutputInterface;
        let protocolos = [
            OutputProtocol::ALL[0].to_profile(),
            OutputProtocol::ALL[1].to_profile(),
            OutputProtocol::ALL[2].to_profile(),
        ];
        ([OutputInterface::Ethernet, OutputInterface::WiFi], protocolos)
    }

    /// Os avisos do validador, para o chamador os registar. Vazio quando não há.
    pub fn avisos_do_profile(profile: &HardwareProfile) -> Vec<String> {
        let d = Self::drivers_disponiveis();
        led_hardware_profile::validate(
            profile,
            &led_hardware_profile::Available { interfaces: &d.0, protocols: &d.1 },
        )
        .warnings()
        .map(|f| format!("{f:?}"))
        .collect()
    }

    /// O **primeiro** alvo. Para os caminhos que ainda só sabem falar com um nó.
    ///
    /// Não é um atalho permanente: existe para que a separação tipo/instância possa entrar
    /// sem reescrever tudo de uma vez. Um caminho que use isto **não** é multi-controlador.
    pub fn primeiro(&self) -> &Alvo {
        self.alvos.first().expect("uma saida tem sempre pelo menos um alvo")
    }

    /// **Todos os alvos são loopback?** (ADR-0029 §6)
    ///
    /// `all`, e não `any`. A excepção do ADR-0005 vale porque um datagrama de loopback não
    /// atravessa interface nenhuma — e basta **um** alvo de rede para haver fio a proteger.
    /// Um `any` desligaria o gate do WiFi para o rig inteiro por causa dos nós que não
    /// contam, que é exactamente a mutação que o controlo negativo
    /// `num_alvo_de_rede_o_wifi_ativo_reprova_mesmo` já apanhou uma vez.
    pub fn todos_loopback(&self) -> bool {
        self.alvos.iter().all(|a| a.addr.ip().is_loopback())
    }

    /// A ordem de canais, seja qual for o formato.
    pub fn rgb_order(&self) -> RgbOrder {
        match self.color {
            ColorFormat::Rgb(o) => o,
            ColorFormat::Rgbw(o, _) => o,
        }
    }

    /// Quantos pixels cabem num datagrama, **derivado do MTU do profile**.
    pub fn pixels_per_datagram(&self) -> usize {
        self.transport.pixels_per_datagram(
            self.protocol.to_profile(),
            self.color,
            self.pixels_per_universe,
        )
    }

    /// Em quantos datagramas um frame se parte, **derivado do MTU do profile**.
    pub fn datagrams_per_frame(&self) -> u32 {
        self.transport.datagrams_for(
            self.pixel_count as u32,
            self.protocol.to_profile(),
            self.color,
            self.pixels_per_universe,
        )
    }
}

/// **ADR-0025 — a cadência pedida cabe no teto que o nó declara?**
///
/// `refresh_hz` é um **limite**, não uma recomendação: vive na struct que o ADR-0018 chama
/// *"ÚNICO lar dos limites"*, ao lado do `max_pixels` — que já recusa desde o GS4.3. Tratar os
/// dois tetos irmãos de maneira diferente é o que a auditoria A3 veio corrigir.
///
/// # Recusa, nunca clampa
///
/// Baixar o `--tick-ms` sozinho faria o journal registar uma cadência e o fio produzir outra.
/// É o precedente do `Strobe` (ADR-0021): *"estroboscópio que muda de frequência sozinho no
/// palco é pior que parâmetro documentado"*.
///
/// # O limite é alcançável
///
/// A comparação é `>`, não `>=`: pedir exatamente o teto declarado é usar o nó como ele diz
/// que pode ser usado. Há um teste dedicado a esta fronteira, porque é a que se escreve errado.
///
/// # Onde isto **não** corre
///
/// No laço. Esta função é chamada **uma vez**, na abertura do palco. Nenhum relógio novo,
/// nenhum segundo scheduler, `Pacer` intocado.
pub fn cadencia_cabe_no_profile(
    profile: &HardwareProfile,
    tick_ms: u64,
) -> Result<(), String> {
    // `tick_ms == 0` já é tratado pelo laço (`period = tick_ms.max(1)`, que impede busy-loop);
    // aqui usa-se a mesma leitura, para que os dois sítios não possam discordar sobre o que
    // "zero" significa.
    let periodo = tick_ms.max(1);
    let pedida = 1_000.0 / periodo as f64;
    let teto = profile.limits.refresh_hz as f64;
    if pedida > teto {
        return Err(format!(
            "cadencia pedida {pedida:.1} Hz (--tick-ms {tick_ms}) excede o teto de {teto:.0} Hz              declarado por `{}` (ADR-0025). Use --tick-ms >= {} ou corrija o preset",
            profile.identity.model,
            (1_000.0 / teto).ceil() as u64
        ));
    }
    Ok(())
}

/// Encontra o preset pelo nome, ou explica quais existem.
///
/// O daemon **não define hardware**: lê o catálogo do `led-hardware-profile`, que é uma
/// tabela `const` onde acrescentar um nó é uma linha (ADR-0018).
pub fn profile_by_name(nome: &str) -> Result<HardwareProfile, String> {
    let reg = led_hardware_profile::HardwareRegistry::with_builtin();
    reg.profile(nome).ok_or_else(|| {
        let mut nomes: Vec<&str> = reg.names().collect();
        nomes.sort_unstable();
        format!("preset `{nome}` nao existe. Disponiveis: {}", nomes.join(", "))
    })
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
    /// A LUT do nó, dobrada **uma vez no arranque** (ADR-0019 §Isolamento do hot-path).
    ///
    /// `None` quando a calibração é a identidade — nesse caso o frame segue **sem cópia e
    /// sem laço**, e o custo de existir esta funcionalidade é exatamente zero.
    lut: Option<CalibrationLut>,
    /// Destino dos pixels corrigidos, dimensionado no arranque. Existe para que o hot path
    /// não aloque, e para que o frame do chamador **nunca** seja mutado: o `Heartbeat`
    /// guarda o último frame válido, e corrigi-lo em cada reenvio escureceria o palco a cada
    /// batida — o mesmo bug cumulativo que o ADR-0019 já tinha apanhado no HAL.
    corrigidos: std::sync::Mutex<Vec<led_core::PixelColor>>,
}

impl OutputManager {
    /// Constrói a saída a partir da configuração. **É o único sítio do projeto que sabe qual
    /// protocolo está em uso.**
    pub fn open(cfg: OutputConfig) -> std::io::Result<Self> {
        let out: Box<dyn ProtocolOutput> = match cfg.protocol {
            // `with_limits`, não `with_format`: é aqui que o MTU declarado deixa de ser
            // decorativo e passa a decidir onde o frame se parte.
            OutputProtocol::Ddp => Box::new(led_player::DdpOutput::with_limits(
                cfg.primeiro().addr,
                cfg.pixel_count,
                cfg.color,
                cfg.pixels_per_datagram(),
                cfg.pixels_per_universe,
            )?),
            OutputProtocol::ArtNet => {
                let assigns =
                    linear_assignments(cfg.pixel_count, 1, cfg.primeiro().first_universe, cfg.rgb_order());
                let layout = CompiledLayout::compile(&assigns);
                let dev = led_protocols::ArtNetDevice::unicast(1, cfg.primeiro().addr)?;
                Box::new(Hal::new(layout, vec![dev]))
            }
            OutputProtocol::Sacn => {
                let assigns =
                    linear_assignments(cfg.pixel_count, 1, cfg.primeiro().first_universe, cfg.rgb_order());
                let layout = CompiledLayout::compile(&assigns);
                // CID fixo e nome próprio: um receptor E1.31 distingue fontes por CID, e dois
                // senders com o mesmo CID seriam indistinguíveis no diagnóstico.
                let cid = *b"LUMYX-DAEMON-001";
                let dev = led_protocols::SacnDevice::unicast(1, cfg.primeiro().addr, cid, "lumyx-daemon")?;
                Box::new(Hal::new(layout, vec![dev]))
            }
        };
        // Identidade não é calibração: gamma 1.0 com brilho 1.0 não muda byte algum, e
        // construir a LUT nesse caso só acrescentaria trabalho por frame.
        let c = cfg.calibration;
        let lut = if c.gamma == 1.0 && c.brightness == 1.0 {
            None
        } else {
            Some(CalibrationLut::new(c.gamma, c.brightness))
        };
        let corrigidos = std::sync::Mutex::new(vec![
            led_core::PixelColor { r: 0, g: 0, b: 0 };
            cfg.pixel_count
        ]);
        Ok(Self { out, cfg, stats: OutputStats::default(), lut, corrigidos })
    }

    pub fn config(&self) -> &OutputConfig {
        &self.cfg
    }
    pub fn stats(&self) -> &OutputStats {
        &self.stats
    }

    /// Envia um frame. Conta sucesso e erro; **devolve o erro**, nunca o engole.
    ///
    /// # A calibração acontece aqui, e é por isso que vale para os três protocolos
    ///
    /// Este é o **único** ponto entre o show e o fio (ADR-0019 Emenda 1). Aplicá-la aqui é o
    /// que impede a assimetria de a ter no Art-Net e não no DDP — que contorna o HAL e por
    /// isso nunca teve onde a receber.
    pub fn send(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        let Some(lut) = &self.lut else {
            return self.enviar(frame);
        };
        let corrigido = {
            let mut buf = self.corrigidos.lock().expect("buffer de calibração");
            if buf.len() != frame.pixels.len() {
                buf.resize(frame.pixels.len(), led_core::PixelColor { r: 0, g: 0, b: 0 });
            }
            for (dst, src) in buf.iter_mut().zip(frame.pixels.iter()) {
                // Por **canal**, antes de o mapper conhecer a ordem do nó: a correção é
                // óptica e não sabe (nem precisa de saber) em que ordem os bytes vão sair.
                dst.r = lut.map(src.r);
                dst.g = lut.map(src.g);
                dst.b = lut.map(src.b);
            }
            LogicalFrame::new(buf.clone(), frame.timestamp_ms)
        };
        self.enviar(&corrigido)
    }

    /// O envio propriamente dito, com a contabilidade. Separado para que **um só sítio**
    /// conte frames e erros, calibrado ou não.
    fn enviar(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
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

    /// Atalho dos testes: preset → configuração resolvida. **Passa pelo mesmo caminho da
    /// produção** — se `resolve` deixar de consultar o profile, estes testes vêem.
    fn cfg_de(preset: &str, addr: &str, px: usize) -> OutputConfig {
        let perfil = profile_by_name(preset).expect("preset do catálogo");
        OutputConfig::resolve(&perfil, addr, px, 1).expect("resolver")
    }

    fn preset_de(proto: &str) -> &'static str {
        match proto {
            "ddp" => "esp32-poe-wled-ddp",
            "artnet" => "esp32-devkit-wled-artnet",
            "sacn" => "falcon-f16v3-sacn",
            outro => panic!("sem preset para {outro}"),
        }
    }
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
        let c = cfg_de("esp32-poe-wled-ddp", "127.0.0.1", 720);
        assert_eq!(c.protocol, OutputProtocol::Ddp);
        assert_eq!(c.primeiro().addr.port(), DDP_PORT, "porta omitida cai no padrão do protocolo");

        assert_eq!(
            cfg_de("esp32-devkit-wled-artnet", "127.0.0.1", 10).primeiro().addr.port(),
            ARTNET_PORT
        );
        assert_eq!(cfg_de("falcon-f16v3-sacn", "127.0.0.1", 10).primeiro().addr.port(), SACN_PORT);
        assert_eq!(
            cfg_de("esp32-poe-wled-ddp", "127.0.0.1:9999", 10).primeiro().addr.port(),
            9999,
            "porta explícita vence o padrão"
        );
    }

    #[test]
    fn resolve_recusa_o_que_esta_errado() {
        let perfil = profile_by_name("esp32-poe-wled-ddp").unwrap();
        for s in ["xyz://127.0.0.1", "artnet://127.0.0.1", "nao-e-ip", "ddp://", ""] {
            assert!(OutputConfig::resolve(&perfil, s, 10, 1).is_err(), "devia recusar: {s}");
        }
        // `127.0.0.1` **sem esquema** é agora a forma canónica: o protocolo vem do profile.
        assert!(OutputConfig::resolve(&perfil, "127.0.0.1", 10, 1).is_ok());
        assert!(
            OutputConfig::resolve(&perfil, "127.0.0.1", 0, 1).is_err(),
            "0 pixels não faz sentido"
        );
        assert!(profile_by_name("nao-existe").is_err(), "preset inexistente é erro, não omissão");
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
            let cfg = cfg_de(preset_de(proto), &addr.to_string(), 8);
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
            let cfg = cfg_de(preset_de(proto), &addr.to_string(), 8);
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
        let cfg = cfg_de("esp32-poe-wled-ddp", "127.0.0.1:9", 8);
        let om = OutputManager::open(cfg).unwrap();
        let r = om.send(&frame(8, 10));
        let (f, e) = (om.stats().frames(), om.stats().errors());
        assert_eq!(f + e, 1, "todo envio conta exatamente uma vez — nunca desaparece");
        assert_eq!(r.is_err(), e == 1, "o contador de erro casa com o Result devolvido");
    }

    #[test]
    fn contadores_acumulam_por_frame() {
        let (sock, addr) = recetor();
        let cfg = cfg_de("esp32-poe-wled-ddp", &addr.to_string(), 4);
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
