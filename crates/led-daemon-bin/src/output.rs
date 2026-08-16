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
    /// **A faixa de universos deste protocolo** (ADR-0029 §7.1). `None` = não endereça por
    /// universo.
    ///
    /// # A sintaxe é comum; a semântica não é
    ///
    /// `IP@N` escreve-se igual nos dois protocolos que usam universos, e isso **não** implica
    /// a mesma faixa. Os valores abaixo estão lidos do código deste repositório, não da
    /// memória de ninguém:
    ///
    /// - **Art-Net** — `artnet.rs` compõe o port-address como `SubUni | (Net << 8) & 0x7F`,
    ///   ou seja **15 bits**: `0..=32767`, e o **zero é válido** (foi ele que a bancada de
    ///   2026-07-23 confirmou alinhado com o `dmx.uni` do WLED).
    /// - **sACN (E1.31)** — `packet.rs` diz, em comentário e em vector de teste,
    ///   *"universe field round-trips 1..=63999"*, começando em **1**. O **zero não é um
    ///   universo E1.31**; e `device.rs::multicast_addr(0)` daria `239.255.0.0`, que não é
    ///   um grupo válido.
    ///
    /// # Uma fonte, não duas
    ///
    /// [`OutputProtocol::usa_universos`] **deriva** daqui. Um booleano ao lado desta faixa
    /// seriam duas verdades sobre a mesma coisa, e a segunda apodreceria em silêncio — é a
    /// regra que o GS4.3 aplicou à fragmentação, aplicada aos universos.
    pub fn faixa_de_universos(self) -> Option<(u16, u16)> {
        match self {
            OutputProtocol::Ddp => None,
            OutputProtocol::ArtNet => Some((0, 32_767)),
            OutputProtocol::Sacn => Some((1, 63_999)),
        }
    }

    /// **Este protocolo endereça por universo?** Derivado de [`Self::faixa_de_universos`].
    ///
    /// É o que decide se `@UNIVERSO` é obrigatório ou proibido no `--output`. O DDP endereça
    /// por **byte** (`pixel_offset`), logo um universo escrito ao lado dele não teria efeito
    /// nenhum — e aceitá-lo em silêncio confirmaria uma crença errada do operador.
    pub fn usa_universos(self) -> bool {
        self.faixa_de_universos().is_some()
    }
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

/// Reparte `total` píxeis por `n` nós, cada um com `max_por_no` no máximo.
///
/// Devolve `(pixel_offset, pixel_count)` por nó, **por ordem dos endereços**. É a mesma
/// disciplina do `pixels_per_datagram`: **derivado**, nunca declarado ao lado da verdade de
/// onde sai (GS4.3 — *"a mesma verdade duas vezes, e a segunda apodrece em silêncio"*).
///
/// # Porque o resto fica no ÚLTIMO nó e não distribuído
///
/// Encher cada nó até ao `max_pixels` e deixar o resto no último é o que torna a repartição
/// **função só do profile e da ordem** — acrescentar um endereço no fim não mexe nas fatias
/// dos anteriores. Distribuir o resto por igual faria a fatia do robô 1 mudar quando alguém
/// ligasse o robô 6, e o operador teria de reconfigurar o rig inteiro para acrescentar um nó.
///
/// # Um nó sem píxeis é recusado, não aceite em silêncio
///
/// Cinco endereços para um show que cabe em dois significa que o operador está enganado
/// sobre uma das duas coisas. Abrir sockets que nunca enviam nada esconderia isso até
/// alguém reparar no palco.
///
/// # O que esta função NÃO decide: `first_universe`
///
/// Todos os alvos recebem o **mesmo** `first_universe`. Isto assume o modelo *"cada
/// controlador tem o seu próprio espaço de universos"*, que é como o WLED se comporta por
/// omissão (`dmx.uni:0`) e o que a validação de hardware de 2026-07-23 observou num nó. O
/// par que identifica um destino passa a ser **(IP, universo)**, e dois nós podem usar o
/// universo 1 sem colidir porque o unicast os separa.
///
/// **Não está verificado com dois nós físicos.** O projecto xLights do operador usa a
/// convenção oposta (universos 1–149 contíguos pelos cinco robôs), e se o rig real exigir
/// essa, a derivação passa a ser `first_universe + i × universos_por_nó` — mudança de uma
/// linha aqui, e é o critério de reversão desta decisão.
fn repartir(total: usize, max_por_no: usize, n: usize) -> Result<Vec<(u32, usize)>, String> {
    if n == 0 {
        return Err("é preciso pelo menos um endereço de saída".into());
    }
    if max_por_no == 0 {
        // O validador do ADR-0018 já emite `ZeroLimit`, e o ADR-0024 impede a saída de
        // abrir. Aqui é uma guarda contra divisão por zero, não uma segunda política.
        return Err("o profile declara max_pixels = 0".into());
    }
    let capacidade = max_por_no.saturating_mul(n);
    if total > capacidade {
        return Err(format!(
            "o show tem {total} px e os {n} nó(s) declaram no máximo {max_por_no} px cada \
             ({capacidade} no total)"
        ));
    }
    let mut fatias = Vec::with_capacity(n);
    for i in 0..n {
        let inicio = i * max_por_no;
        // `saturating_sub`: quando o nó começa **para lá** do fim do show, a subtracção
        // simples estoura. Dá 0, que é a condição de recusa logo abaixo — o caso não é
        // aritmética inválida, é um endereço a mais.
        let conta = total.saturating_sub(inicio).min(max_por_no);
        if conta == 0 {
            return Err(format!(
                "o nó {i} ficaria sem píxeis: {total} px cabem em {} nó(s) de {max_por_no}, \
                 e foram dados {n} endereços",
                inicio.div_ceil(max_por_no)
            ));
        }
        fatias.push((inicio as u32, conta));
    }
    Ok(fatias)
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
    ) -> Result<Self, String> {
        Self::resolve_muitos(profile, std::slice::from_ref(&spec.to_string()), pixel_count)
    }

    /// Resolve **N** especificações de endereço. `resolve` delega aqui com uma lista de um.
    ///
    /// A ordem dos endereços **é** a ordem das fatias (ADR-0029 §2): o primeiro `--output`
    /// recebe o início do show. Por isso a lista não é ordenada nem deduplicada em silêncio —
    /// reordenar mudaria que pixels vão para que robô.
    /// **Sem parâmetro de universo**: ele vem de cada `spec` (ADR-0029 §7). Era aqui que o
    /// `1` implícito do `stage.rs` entrava — e a bancada de 2026-07-23 diz que era o valor
    /// errado para o nó validado.
    pub fn resolve_muitos(
        profile: &HardwareProfile,
        specs: &[String],
        pixel_count: usize,
    ) -> Result<Self, String> {
        let protocol = OutputProtocol::from_profile(profile.capabilities.protocol);
        let mut alvos = Vec::with_capacity(specs.len());
        for spec in specs {
            alvos.push(Self::um_endereco(spec, protocol, profile)?);
        }
        Self::from_profile_muitos(profile, &alvos, pixel_count)
    }

    /// Uma especificação → `(endereço, universo)`, com o esquema conferido contra o profile.
    ///
    /// Aceita `IP[:PORTA][@UNIVERSO]` e `proto://IP[:PORTA][@UNIVERSO]`. IPv6 **entre
    /// colchetes** (`[::1]:6454@0`) — sem eles não há como distinguir os `:` do endereço do
    /// separador de porta, e adivinhar seria a classe de omissão que o §7 proíbe.
    ///
    /// **Não há universo por omissão** (ADR-0029 §7): ausente num protocolo que os usa é
    /// erro, presente num que não os usa é erro.
    fn um_endereco(
        spec: &str,
        protocol: OutputProtocol,
        profile: &HardwareProfile,
    ) -> Result<(SocketAddr, u16), String> {
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

        // O universo, se escrito. `rsplit_once` porque `@` não ocorre em endereços IP nem em
        // IPv6 — o último é inequivocamente o separador.
        let (endereco, universo) = match resto.rsplit_once('@') {
            Some((e, u)) => {
                let n = u.parse::<u16>().map_err(|_| {
                    format!("universo inválido `{u}` em `{spec}` (use um inteiro 0..65535)")
                })?;
                (e, Some(n))
            }
            None => (resto, None),
        };

        // Porta: em IPv6 entre colchetes ela vem depois de `]:`; fora deles, qualquer `:`.
        let tem_porta = if endereco.starts_with('[') {
            endereco.rsplit_once("]:").is_some()
        } else {
            endereco.contains(':')
        };
        let com_porta = if tem_porta {
            endereco.to_string()
        } else {
            format!("{endereco}:{}", protocol.default_port())
        };
        let addr: SocketAddr = com_porta.parse().map_err(|_| {
            format!("endereço inválido `{com_porta}` (use IP[:porta], IPv6 entre colchetes)")
        })?;

        // ── A regra do §7: recusar, nunca adivinhar ──────────────────────────
        match (protocol.faixa_de_universos(), universo) {
            // **Fora da faixa é recusa, não truncatura.** O `build_art_dmx` mascara um
            // universo acima de 32767 em silêncio (40000 → 7232); esta fronteira apanha-o
            // antes de lá chegar. A correcção na origem é fatia própria (ADR-0029 §7.1).
            (Some((min, max)), Some(u)) if u < min || u > max => Err(format!(
                "`{spec}`: universo {u} fora da faixa do {} ({min}..={max}). \
                 A sintaxe é comum aos protocolos, a faixa não é",
                protocol.as_str()
            )),
            (Some(_), Some(u)) => Ok((addr, u)),
            (Some(_), None) => Err(format!(
                "`{spec}`: o preset `{}` usa {} e exige o universo — escreva `{endereco}@N`. \
                 Não há omissão: a bancada de 2026-07-23 mostrou que o universo errado \
                 desloca a fita sem erro nenhum",
                profile.identity.model,
                protocol.as_str()
            )),
            (None, None) => Ok((addr, 0)),
            (None, Some(_)) => Err(format!(
                "`{spec}`: o {} endereça por byte e ignora universos — remova o `@`",
                protocol.as_str()
            )),
        }
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
        Self::from_profile_muitos(profile, &[(addr, first_universe)], pixel_count)
    }

    /// O construtor de **N nós** — e o único que constrói de facto.
    ///
    /// [`OutputConfig::from_profile`] delega aqui com uma lista de um. Não há um caminho para
    /// um nó e outro para N: o que muda é o comprimento da lista (ADR-0029 §1). Um segundo
    /// construtor seria a assimetria que este ADR existe para impedir — e é exactamente a
    /// classe do defeito de 2026-08-07f (*"pior que a ausência uniforme, porque pareceria
    /// feito"*).
    ///
    /// **Todos os alvos partilham o mesmo `profile`**, e isso é uma regra, não uma omissão:
    /// a `Calibration` é do tipo de hardware, logo cinco nós do mesmo preset partilham a LUT
    /// por construção (ADR-0029 §3). Um rig de perfis mistos é recusado por não ser
    /// exprimível aqui, em vez de ser silenciosamente mal calibrado.
    pub fn from_profile_muitos(
        profile: &HardwareProfile,
        alvos_base: &[(SocketAddr, u16)],
        pixel_count: usize,
    ) -> Result<Self, String> {
        let addrs: Vec<SocketAddr> = alvos_base.iter().map(|(a, _)| *a).collect();
        if addrs.is_empty() {
            return Err("é preciso pelo menos um endereço de saída".into());
        }
        // Dois nós no mesmo endereço receberiam fatias diferentes e o controlador veria os
        // dois fluxos — o último a chegar ganha, e o palco pisca. É erro do operador, e é
        // barato de apanhar aqui.
        for (i, a) in addrs.iter().enumerate() {
            if let Some(j) = addrs[..i].iter().position(|b| b == a) {
                return Err(format!(
                    "o endereço {a} aparece duas vezes (posições {j} e {i}); \
                     dois alvos no mesmo nó sobrepõem-se no fio"
                ));
            }
        }
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
        if !profile.transport.heartbeat_is_safe() {
            return Err(format!(
                "heartbeat declarado de {} ms nao respeita o teto de {} ms do LUMYX_GOSL",
                profile.transport.heartbeat_ms,
                Transport::MAX_GAP_MS
            ));
        }

        // A repartição é **derivada** do `max_pixels` e da ordem dos endereços (ADR-0029 §2).
        // Com um alvo, dá exactamente `offset 0, count = show inteiro` — que é o
        // comportamento anterior, sem ramo próprio.
        let fatias = repartir(pixel_count, profile.limits.max_pixels as usize, addrs.len())
            .map_err(|e| format!("{e} (nó `{}`)", profile.identity.model))?;

        let alvos = alvos_base
            .iter()
            .zip(fatias)
            .map(|((addr, first_universe), (pixel_offset, pixel_count))| Alvo {
                addr: *addr,
                // **Declarado por alvo** (ADR-0029 §7). Deixou de haver um valor global: as
                // duas convenções — mesmo universo em todos (WLED) ou contíguos (xLights) —
                // são exprimíveis, e o daemon não escolhe por ninguém.
                first_universe: *first_universe,
                pixel_offset,
                pixel_count,
            })
            .collect();

        Ok(Self {
            protocol: OutputProtocol::from_profile(profile.capabilities.protocol),
            alvos,
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
    /// Uma por no. Nunca vazia — o `resolve` garante pelo menos um alvo.
    saidas: Vec<Saida>,
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
        // **Uma saída por alvo, construída pelo MESMO `match`.** Não há um caminho para um nó
        // e outro para N: o que muda é quantas vezes se percorre a lista (ADR-0029 §1). Um
        // segundo construtor seria a assimetria que este ADR existe para impedir.
        let mut saidas: Vec<Saida> = Vec::with_capacity(cfg.alvos.len());
        for alvo in &cfg.alvos {
            let out: Box<dyn ProtocolOutput> = match cfg.protocol {
                // `with_limits`, não `with_format`: é aqui que o MTU declarado deixa de ser
                // decorativo e passa a decidir onde o frame se parte.
                //
                // E o `with_pixel_offset` é o TD-016: sem ele, os cinco nós receberiam todos
                // o intervalo a partir de zero e acenderiam a mesma coisa.
                OutputProtocol::Ddp => Box::new(
                    led_player::DdpOutput::with_limits(
                        alvo.addr,
                        alvo.pixel_count,
                        cfg.color,
                        cfg.pixels_per_datagram(),
                        cfg.pixels_per_universe,
                    )?
                    .with_pixel_offset(alvo.pixel_offset),
                ),
                OutputProtocol::ArtNet => {
                    // O layout é da FATIA deste nó, e o `first_universe` é dele: é assim que
                    // o equivalente do offset chega ao Art-Net.
                    let assigns = linear_assignments(
                        alvo.pixel_count,
                        1,
                        alvo.first_universe,
                        cfg.rgb_order(),
                    );
                    let layout = CompiledLayout::compile(&assigns);
                    let dev = led_protocols::ArtNetDevice::unicast(1, alvo.addr)?;
                    Box::new(Hal::new(layout, vec![dev]))
                }
                OutputProtocol::Sacn => {
                    let assigns = linear_assignments(
                        alvo.pixel_count,
                        1,
                        alvo.first_universe,
                        cfg.rgb_order(),
                    );
                    let layout = CompiledLayout::compile(&assigns);
                    // CID fixo e nome próprio: um receptor E1.31 distingue fontes por CID, e
                    // dois senders com o mesmo CID seriam indistinguíveis no diagnóstico.
                    let cid = *b"LUMYX-DAEMON-001";
                    let dev =
                        led_protocols::SacnDevice::unicast(1, alvo.addr, cid, "lumyx-daemon")?;
                    Box::new(Hal::new(layout, vec![dev]))
                }
            };
            saidas.push(Saida {
                out,
                alvo: *alvo,
                // Dimensionado no arranque, como o `corrigidos`: a fatia deste nó não muda
                // de tamanho durante o show.
                fatia: std::sync::Mutex::new(vec![
                    led_core::PixelColor { r: 0, g: 0, b: 0 };
                    alvo.pixel_count
                ]),
                stats: OutputStats::default(),
            });
        }
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
        Ok(Self { saidas, cfg, stats: OutputStats::default(), lut, corrigidos })
    }

    pub fn config(&self) -> &OutputConfig {
        &self.cfg
    }

    /// As estatísticas **agregadas**. Existem porque o `Heartbeat` e o journal falam de "a
    /// saída" como um todo.
    ///
    /// ⚠️ Para saber se um nó específico está vivo, use [`OutputManager::por_alvo`]. Um
    /// agregado **não distingue** cinco nós a funcionar de quatro a funcionar e um morto —
    /// e é essa distinção que o ADR-0029 §5 obriga a manter observável.
    pub fn stats(&self) -> &OutputStats {
        &self.stats
    }

    /// **As estatísticas de cada nó, com o endereço.** É isto que impede um nó em silêncio de
    /// ser indistinguível de um nó a funcionar (ADR-0029 §5).
    pub fn por_alvo(&self) -> Vec<(SocketAddr, u64, u64)> {
        self.saidas
            .iter()
            .map(|s| (s.alvo.addr, s.stats.frames(), s.stats.errors()))
            .collect()
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
        let mut primeiro_erro: Option<OutputError> = None;

        for s in &self.saidas {
            // **Caminho rápido do alvo único**: um nó que começa em 0 e cobre o show inteiro
            // recebe o frame tal como veio, sem cópia. É o caminho validado em hardware
            // (94/94 frames, 2026-07-20), e esta fatia não pode torná-lo mais caro.
            let r = if self.saidas.len() == 1 && s.alvo.pixel_offset == 0 {
                s.out.send_frame(frame)
            } else {
                // Cada nó recebe a **sua fatia**; o `pixel_offset` diz ao destino onde ela
                // começa. Um frame mais curto que a fatia não é erro — é um show menor que o
                // rig, e o que falta fica no último valor em vez de piscar a preto.
                let inicio = s.alvo.pixel_offset as usize;
                let fim = (inicio + s.alvo.pixel_count).min(frame.pixels.len());
                let mut buf = s.fatia.lock().expect("buffer da fatia");
                if inicio < fim {
                    let n = fim - inicio;
                    buf[..n].copy_from_slice(&frame.pixels[inicio..fim]);
                }
                s.out.send_frame(&LogicalFrame::new(buf.clone(), frame.timestamp_ms))
            };

            match r {
                Ok(()) => {
                    s.stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                    self.stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    // **Um cabo partido no robô 3 não pode apagar os robôs 1, 2, 4 e 5**
                    // (ADR-0029 §5). O erro é contado no alvo E no agregado, e o laço
                    // continua para os outros; devolve-se o primeiro para quem quiser saber.
                    s.stats.errors.fetch_add(1, Ordering::Relaxed);
                    self.stats.errors.fetch_add(1, Ordering::Relaxed);
                    primeiro_erro.get_or_insert(e);
                }
            }
        }

        match primeiro_erro {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Uma saída **por nó**: o driver, o alvo, o buffer da sua fatia e a sua contabilidade.
///
/// As estatísticas são **por alvo** de propósito. Somá-las tornaria um nó morto
/// indistinguível de um nó vivo — que é a observabilidade a mentir (ADR-0026 §9), e é
/// precisamente o que o operador precisa de ver com cinco robôs no palco.
struct Saida {
    out: Box<dyn ProtocolOutput>,
    alvo: Alvo,
    /// Dimensionado no arranque. Só é usado quando há mais de um alvo.
    fatia: std::sync::Mutex<Vec<led_core::PixelColor>>,
    stats: OutputStats,
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
    /// A **soma** dos universos de todos os nós.
    ///
    /// Aqui somar é correcto, ao contrário das estatísticas: o `universe_count` descreve a
    /// dimensão do rig, e cinco nós de 28 universos ocupam 140. Não é um veredito sobre
    /// saúde — é uma contagem, e uma contagem agrega.
    fn universe_count(&self) -> u16 {
        self.saidas.iter().map(|s| s.out.universe_count()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Atalho dos testes: preset → configuração resolvida. **Passa pelo mesmo caminho da
    /// produção** — se `resolve` deixar de consultar o profile, estes testes vêem.
    fn cfg_de(preset: &str, addr: &str, px: usize) -> OutputConfig {
        let perfil = profile_by_name(preset).expect("preset do catálogo");
        // O universo é declarado por alvo e **obrigatório** nos protocolos que o usam
        // (ADR-0029 §7). O atalho escreve o que cada preset exige, para continuar a
        // atravessar o mesmo caminho da produção nos três protocolos.
        // O universo **mínimo válido do protocolo**, derivado da faixa — não `0` escrito à
        // mão. `0` é válido em Art-Net e inválido em sACN (ADR-0029 §7.1), e um literal aqui
        // faria estes atalhos recusarem metade do catálogo.
        let protocolo = OutputProtocol::from_profile(perfil.capabilities.protocol);
        let spec = match protocolo.faixa_de_universos() {
            Some((min, _)) => format!("{addr}@{min}"),
            None => addr.to_string(),
        };
        OutputConfig::resolve(&perfil, &spec, px).expect("resolver")
    }

    // ── ADR-0029 §2 · a repartição derivada ──────────────────────────────────────
    //
    // Estes testes exercitam `repartir` **directamente**. Os dois testes de fan-out abaixo
    // escrevem `cfg.alvos` à mão para isolar o envio, e por isso não provariam nada sobre a
    // derivação: sem esta secção, `repartir` teria zero cobertura.

    /// Cada nó enche até ao `max_pixels`; **o resto fica no último**.
    #[test]
    fn a_reparticao_enche_cada_no_e_deixa_o_resto_no_ultimo() {
        // Exacto: 5 × 1240 = 6200, o tamanho do rig real.
        assert_eq!(
            repartir(6200, 1240, 5).unwrap(),
            vec![(0, 1240), (1240, 1240), (2480, 1240), (3720, 1240), (4960, 1240)]
        );
        // Com resto: o último leva 500, não 833 espalhados.
        assert_eq!(repartir(2500, 1000, 3).unwrap(), vec![(0, 1000), (1000, 1000), (2000, 500)]);
    }

    /// **A propriedade que justifica o desenho.** Acrescentar um nó no fim não pode mexer nas
    /// fatias dos anteriores — senão ligar o robô 6 obrigaria a reconfigurar os cinco.
    #[test]
    fn acrescentar_um_no_no_fim_nao_mexe_nas_fatias_anteriores() {
        let cinco = repartir(6200, 1240, 5).unwrap();
        let seis = repartir(6200 + 300, 1240, 6).unwrap();
        assert_eq!(&seis[..5], &cinco[..], "as cinco primeiras fatias tem de ficar iguais");
        assert_eq!(seis[5], (6200, 300));
    }

    /// **Com um alvo, nada mudou.** É esta a garantia de que a fatia N=1 continua a ser o
    /// caminho validado em hardware, e não um caso particular de código novo.
    #[test]
    fn com_um_alvo_a_fatia_e_o_show_inteiro_e_o_offset_e_zero() {
        assert_eq!(repartir(720, 1500, 1).unwrap(), vec![(0, 720)]);
        assert_eq!(repartir(1500, 1500, 1).unwrap(), vec![(0, 1500)], "exactamente no limite");
    }

    /// Um show maior que a soma dos nós é **recusado na construção**, e a mensagem traz os
    /// dois números — sem eles o operador não sabe se acrescenta nós ou encurta o show.
    #[test]
    fn um_show_maior_que_a_soma_dos_nos_e_recusado() {
        let e = repartir(6201, 1240, 5).unwrap_err();
        assert!(e.contains("6201") && e.contains("1240") && e.contains("6200"), "{e}");
        // E um passo abaixo passa: a fronteira é uma linha, não uma zona.
        assert!(repartir(6200, 1240, 5).is_ok());
    }

    /// **Um nó que ficaria sem píxeis é recusado.** Abrir um socket que nunca envia esconde
    /// um engano do operador até alguém reparar no palco.
    #[test]
    fn um_no_sem_pixeis_e_recusado_em_vez_de_aberto_em_silencio() {
        let e = repartir(1000, 1000, 3).unwrap_err();
        assert!(e.contains("sem píxeis"), "{e}");
        // 1001 já ocupa o segundo nó, e três continuam a ser demais.
        assert!(repartir(1001, 1000, 2).is_ok());
        assert!(repartir(1001, 1000, 3).is_err());
    }

    /// **A derivação chega ao `Alvo`** — e não fica só na função pura.
    ///
    /// Vai pelo `from_profile_muitos`, que é o construtor real, com o preset do rig.
    #[test]
    fn cinco_enderecos_produzem_cinco_alvos_com_offsets_derivados() {
        let perfil = profile_by_name("esp32-poe-wled-ddp").unwrap();
        // DDP: universo 0 e nunca consultado — o protocolo endereça por byte.
        let addrs: Vec<(SocketAddr, u16)> = (1..=5)
            .map(|i| (format!("192.168.2.{}:4048", 155 + i).parse().unwrap(), 0))
            .collect();

        // **A premissa, afirmada em vez de assumida.** Os offsets abaixo são escritos à mão
        // de propósito — é o precedente do teste de MTU do GS4.3: o valor de uma comparação
        // está em ela ter **duas fontes independentes**, e calcular o esperado a partir do
        // mesmo `max_pixels` que a produção usa colaria as duas. Sem esta linha, mudar o
        // preset faria o teste reprovar sem dizer porquê.
        assert_eq!(
            perfil.limits.max_pixels, 1500,
            "este teste assume o max_pixels deste preset; se ele mudou, actualize os offsets"
        );
        let cfg = OutputConfig::from_profile_muitos(&perfil, &addrs, 6200).expect("resolver");

        assert_eq!(cfg.alvos.len(), 5);
        let offsets: Vec<u32> = cfg.alvos.iter().map(|a| a.pixel_offset).collect();
        let contas: Vec<usize> = cfg.alvos.iter().map(|a| a.pixel_count).collect();
        assert_eq!(offsets, vec![0, 1500, 3000, 4500, 6000]);
        assert_eq!(contas, vec![1500, 1500, 1500, 1500, 200], "o resto fica no ultimo");
        assert_eq!(contas.iter().sum::<usize>(), 6200, "as fatias cobrem o show inteiro");

        // **O universo é o DECLARADO por alvo** (ADR-0029 §7) — deixou de haver um valor
        // global. Aqui todos foram declarados 0 porque o preset é DDP, que os ignora.
        assert_eq!(
            cfg.alvos.iter().map(|a| a.first_universe).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0],
            "o universo vem de cada spec, nao de um parametro global"
        );
    }

    /// O mesmo endereço duas vezes é engano do operador, e os dois fluxos pisariam-se no fio.
    #[test]
    fn o_mesmo_endereco_duas_vezes_e_recusado() {
        let perfil = profile_by_name("esp32-poe-wled-ddp").unwrap();
        let a: SocketAddr = "192.168.2.156:4048".parse().unwrap();
        let b: SocketAddr = "192.168.2.157:4048".parse().unwrap();
        // 3000 px em nós de 1500: os dois enchem, portanto o **único** motivo de recusa
        // possível é a repetição. Com um total pequeno, a recusa viria de "nó sem píxeis" e
        // o teste passaria sem exercitar nada do que afirma.
        let e = OutputConfig::from_profile_muitos(&perfil, &[(a, 0), (b, 0), (a, 0)], 3000).unwrap_err();
        assert!(e.contains("duas vezes"), "{e}");
        assert!(OutputConfig::from_profile_muitos(&perfil, &[(a, 0), (b, 0)], 3000).is_ok());
    }

    /// **ADR-0029 — cinco nós, cinco sockets, cada um com a SUA fatia.**
    ///
    /// É o rig real em miniatura: cinco WLED, cada um com o seu pedaço do show. O que este
    /// teste protege é a diferença entre *acender o palco* e *acender cinco vezes o mesmo
    /// robô* — que é o que acontecia antes do TD-016, e que **parece funcionar**.
    #[test]
    fn cinco_nos_recebem_cada_um_a_sua_fatia_e_nao_todos_a_mesma() {
        use led_protocols::parse_ddp_packet;
        use std::net::UdpSocket;

        const NOS: usize = 5;
        const POR_NO: usize = 8;

        // Cinco receptores reais. Nada de mocks: se o fan-out não abrir cinco sockets, os
        // que faltarem não recebem nada e o teste diz qual.
        let recetores: Vec<UdpSocket> = (0..NOS)
            .map(|_| {
                let s = UdpSocket::bind("127.0.0.1:0").unwrap();
                s.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();
                s
            })
            .collect();

        let mut cfg = cfg_de("esp32-poe-wled-ddp", "127.0.0.1", NOS * POR_NO);
        // **Calibração neutralizada**, pelo mesmo motivo que o `wled_driver.rs` o faz: este
        // teste isola o ENDEREÇAMENTO, e com gamma activa mediria duas coisas ao mesmo tempo
        // sem provar bem nenhuma. A calibração tem o seu ficheiro próprio.
        cfg.calibration =
            led_hardware_profile::Calibration { gamma: 1.0, brightness: 1.0 };
        cfg.alvos = recetores
            .iter()
            .enumerate()
            .map(|(i, r)| Alvo {
                addr: r.local_addr().unwrap(),
                first_universe: 1,
                pixel_offset: (i * POR_NO) as u32,
                pixel_count: POR_NO,
            })
            .collect();

        let cfg_ordem = cfg.rgb_order();
        let mgr = OutputManager::open(cfg).expect("abrir cinco saidas");

        // Cada pixel leva o índice do seu NÓ no canal vermelho. Assim o que chega a cada
        // socket identifica de quem era a fatia — e um nó que receba a fatia errada é
        // imediatamente visível, em vez de exigir aritmética sobre offsets.
        let pixels: Vec<led_core::PixelColor> = (0..NOS * POR_NO)
            .map(|i| led_core::PixelColor { r: (i / POR_NO) as u8, g: 0, b: 0 })
            .collect();
        mgr.send(&LogicalFrame::new(pixels, 0)).expect("envio para os cinco");

        let mut buf = [0u8; 2048];
        for (i, r) in recetores.iter().enumerate() {
            let (n, _) = r.recv_from(&mut buf).unwrap_or_else(|e| {
                panic!("o no {i} NAO recebeu nada — o fan-out nao chegou la: {e}")
            });
            let p = parse_ddp_packet(&buf[..n]).expect("datagrama DDP valido");

            assert_eq!(
                p.offset_bytes,
                (i * POR_NO * 3) as u32,
                "no {i}: o offset tem de ser o DELE, em bytes"
            );
            // O preset e **GRB**: o vermelho e o SEGUNDO byte. O indice vem do proprio
            // profile em vez de escrito a mao — senao este teste partia-se em silencio no dia
            // em que o preset mudasse de ordem, que e o defeito que o GS4.3 apanhou.
            let r_idx = match cfg_ordem {
                RgbOrder::Rgb => 0,
                RgbOrder::Grb => 1,
                RgbOrder::Bgr => 2,
            };
            assert!(
                p.payload.chunks(3).all(|px| px[r_idx] == i as u8),
                "no {i} recebeu a fatia de OUTRO no: vermelho={} (indice {r_idx}, ordem {cfg_ordem:?})",
                p.payload.get(r_idx).copied().unwrap_or(255)
            );
        }

        // **O controlo negativo do conjunto.** Se todos tivessem recebido a mesma coisa, as
        // asserções acima já teriam falhado — mas esta afirma a propriedade directamente, e
        // é a que descreve o defeito em palavras do operador.
        let por_alvo = mgr.por_alvo();
        assert_eq!(por_alvo.len(), NOS, "cinco alvos, cinco contabilidades");
        assert!(
            por_alvo.iter().all(|(_, frames, erros)| *frames == 1 && *erros == 0),
            "cada no tem de contar o SEU envio: {por_alvo:?}"
        );
    }

    /// **Um nó morto não apaga os outros, e não desaparece da contabilidade** (ADR-0029 §5).
    ///
    /// O alvo do meio aponta para uma porta onde ninguém escuta **e** para um endereço que
    /// o SO recusa, para que o `send` falhe de facto. Os outros têm de acender na mesma.
    #[test]
    fn um_no_em_falha_nao_derruba_os_outros_e_a_perda_e_atribuida() {
        use std::net::UdpSocket;

        let vivo1 = UdpSocket::bind("127.0.0.1:0").unwrap();
        vivo1.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();
        let vivo2 = UdpSocket::bind("127.0.0.1:0").unwrap();
        vivo2.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();

        let mut cfg = cfg_de("esp32-poe-wled-ddp", "127.0.0.1", 24);
        cfg.alvos = vec![
            Alvo {
                addr: vivo1.local_addr().unwrap(),
                first_universe: 1,
                pixel_offset: 0,
                pixel_count: 8,
            },
            // Porta 0 num endereço de broadcast: o `connect`/`send` falha localmente.
            Alvo {
                addr: "255.255.255.255:1".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 8,
                pixel_count: 8,
            },
            Alvo {
                addr: vivo2.local_addr().unwrap(),
                first_universe: 1,
                pixel_offset: 16,
                pixel_count: 8,
            },
        ];

        // O `connect` a um endereço de broadcast **passa**; é o `send` que devolve EACCES.
        // Portanto a abertura tem de ter sucesso, e um erro aqui é um cenário diferente do
        // que este teste descreve — nunca um motivo para passar em silêncio.
        let mgr = OutputManager::open(cfg).expect(
            "a abertura tem de passar: o connect a um destino de broadcast nao falha, \
             so o send é que falha. Se falhou, este teste deixou de exercitar o que afirma",
        );
        let pixels = vec![led_core::PixelColor { r: 9, g: 0, b: 0 }; 24];
        let _ = mgr.send(&LogicalFrame::new(pixels, 0));

        let mut buf = [0u8; 2048];
        assert!(vivo1.recv_from(&mut buf).is_ok(), "o no 1 tem de acender apesar do no 2");
        assert!(vivo2.recv_from(&mut buf).is_ok(), "o no 3 tem de acender apesar do no 2");

        // **A perda é atribuída, e é isso que este teste existe para provar.**
        //
        // A versão anterior somava tudo e afirmava `frames + erros == 3` — o que é verdade
        // **sempre**, porque cada alvo incrementa exactamente um dos dois contadores. Passava
        // idêntica se o nó do meio tivesse enviado com sucesso: um gate que não distingue os
        // dois mundos não prova nenhum (KB-012).
        let por_alvo = mgr.por_alvo();
        assert_eq!(por_alvo.len(), 3, "tres alvos, tres contabilidades: {por_alvo:?}");
        for (i, esperado) in [(0usize, (1u64, 0u64)), (1, (0, 1)), (2, (1, 0))] {
            let (addr, frames, erros) = por_alvo[i];
            assert_eq!(
                (frames, erros),
                esperado,
                "alvo {i} ({addr}): esperava (frames, erros)={esperado:?}, veio ({frames}, {erros}).\
                 \n  Se o alvo do meio contou um FRAME, este SO nao recusou o broadcast e o teste\
                 \n  deixou de exercitar a falha — troque o alvo, nao a asserção.\
                 \n  completo: {por_alvo:?}"
            );
        }
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

    /// **ADR-0029 §7.1 — a faixa é do PROTOCOLO, e as fronteiras são testadas nos dois lados.**
    ///
    /// A versão anterior deste teste usava `@0` para os três protocolos e afirmava que
    /// passava nos dois que usam universos. Isso **fixava um defeito como esperado**: `0` não
    /// é um universo E1.31. Um teste que nunca sai da faixa não pode descobrir que a faixa
    /// está errada.
    #[test]
    fn a_faixa_de_universos_e_por_protocolo_e_as_fronteiras_recusam() {
        // (preset, aceites, recusados) — os valores vêm do ADR-0029 §7.1, que os leu do
        // `artnet.rs` e do `packet.rs`.
        let casos: &[(&str, &[u16], &[u16])] = &[
            ("esp32-devkit-wled-artnet", &[0, 32_767], &[32_768]),
            ("falcon-f16v3-sacn", &[1, 63_999], &[0, 64_000]),
        ];
        for (preset, aceites, recusados) in casos {
            let p = profile_by_name(preset).unwrap();
            let proto = OutputProtocol::from_profile(p.capabilities.protocol);
            let (min, max) = proto.faixa_de_universos().expect("usa universos");

            for u in *aceites {
                let r = OutputConfig::resolve(&p, &format!("127.0.0.1@{u}"), 8);
                assert!(r.is_ok(), "{preset}: @{u} está em {min}..={max} e tem de passar: {:?}", r.err());
            }
            for u in *recusados {
                let e = OutputConfig::resolve(&p, &format!("127.0.0.1@{u}"), 8)
                    .expect_err(&format!("{preset}: @{u} está fora de {min}..={max} e tem de recusar"));
                assert!(e.contains("fora da faixa"), "{preset}@{u}: {e}");
            }
        }

        // **O controlo negativo do conjunto.** `0` é o único valor que separa os dois
        // protocolos: válido em Art-Net, inválido em sACN. Se as faixas alguma vez forem
        // unificadas, é esta asserção que o diz — e nenhuma das de cima o faria sozinha.
        let artnet = profile_by_name("esp32-devkit-wled-artnet").unwrap();
        let sacn = profile_by_name("falcon-f16v3-sacn").unwrap();
        assert!(
            OutputConfig::resolve(&artnet, "127.0.0.1@0", 8).is_ok(),
            "Art-Net: o universo 0 é válido — foi o que a bancada de 2026-07-23 mediu"
        );
        assert!(
            OutputConfig::resolve(&sacn, "127.0.0.1@0", 8).is_err(),
            "sACN: o universo 0 NÃO existe em E1.31 — a sintaxe é comum, a semântica não"
        );
    }

    /// **A obrigatoriedade e a proibição do `@`, por protocolo.** Complementa a faixa: aqui a
    /// pergunta é *se* o universo é exigido, ali é *qual* é aceite.
    #[test]
    fn o_universo_e_obrigatorio_onde_conta_e_proibido_onde_nao_conta() {
        // (preset, universo VÁLIDO para este protocolo) — `None` = não usa universos.
        let casos: &[(&str, Option<u16>)] = &[
            ("esp32-poe-wled-ddp", None),
            ("esp32-devkit-wled-artnet", Some(0)),
            ("falcon-f16v3-sacn", Some(1)), // 1, não 0: E1.31 não define o zero
        ];
        for (preset, valido) in casos {
            let p = profile_by_name(preset).unwrap();
            let proto = OutputProtocol::from_profile(p.capabilities.protocol);
            assert_eq!(
                proto.usa_universos(),
                valido.is_some(),
                "{preset}: `usa_universos` tem de derivar da faixa"
            );
            let sem = OutputConfig::resolve(&p, "127.0.0.1", 8);

            match valido {
                Some(u) => {
                    let com = OutputConfig::resolve(&p, &format!("127.0.0.1@{u}"), 8);
                    assert!(com.is_ok(), "{preset}: com `@{u}` tem de passar: {:?}", com.err());
                    let e = sem.expect_err(&format!("{preset}: sem `@` tem de recusar"));
                    assert!(e.contains("exige o universo"), "{preset}: {e}");
                }
                None => {
                    assert!(sem.is_ok(), "{preset}: sem `@` tem de passar: {:?}", sem.err());
                    let e = OutputConfig::resolve(&p, "127.0.0.1@0", 8)
                        .expect_err(&format!("{preset}: com `@` tem de recusar"));
                    assert!(e.contains("ignora universos"), "{preset}: {e}");
                }
            }
        }
    }

    /// O universo declarado **chega ao `Alvo`**, e nós diferentes podem levar universos
    /// diferentes — que é o que torna as duas convenções (WLED e xLights) exprimíveis.
    #[test]
    fn cada_alvo_leva_o_universo_que_foi_declarado() {
        let p = profile_by_name("esp32-devkit-wled-artnet").unwrap();
        let specs = ["10.0.0.1@0".to_string(), "10.0.0.2@28".to_string()];
        // 1600 px enchem o primeiro nó (max 1500) e sobram 100 para o segundo. Com um total
        // pequeno o segundo alvo ficaria sem píxeis e a recusa viria daí — o teste passaria a
        // medir a repartição em vez do universo.
        let cfg = OutputConfig::resolve_muitos(&p, &specs, 1600).expect("resolver");
        assert_eq!(
            cfg.alvos.iter().map(|a| a.first_universe).collect::<Vec<_>>(),
            vec![0, 28],
            "o universo é por alvo; um valor global nao conseguiria exprimir isto"
        );
    }

    /// IPv6 **entre colchetes**, com e sem porta, com o universo colado.
    #[test]
    fn ipv6_entre_colchetes_e_aceite_com_universo() {
        let p = profile_by_name("esp32-devkit-wled-artnet").unwrap();
        let cfg = OutputConfig::resolve(&p, "[::1]:6454@3", 8).expect("com porta");
        assert_eq!(cfg.primeiro().first_universe, 3);
        assert!(cfg.primeiro().addr.is_ipv6());
        // Sem porta: cai na porta do protocolo, e o `@` continua a ser lido.
        let cfg = OutputConfig::resolve(&p, "[::1]@9", 8).expect("sem porta");
        assert_eq!(cfg.primeiro().addr.port(), ARTNET_PORT);
        assert_eq!(cfg.primeiro().first_universe, 9);
    }

    #[test]
    fn resolve_recusa_o_que_esta_errado() {
        let perfil = profile_by_name("esp32-poe-wled-ddp").unwrap();
        for s in [
            "xyz://127.0.0.1",
            "artnet://127.0.0.1",
            "nao-e-ip",
            "ddp://",
            "",
            // ADR-0029 §7: o DDP endereça por byte, logo `@` é erro e não decoração.
            "127.0.0.1@0",
            "127.0.0.1:4048@7",
            // Universo que não é número.
            "127.0.0.1@abc",
        ] {
            assert!(OutputConfig::resolve(&perfil, s, 10).is_err(), "devia recusar: {s}");
        }
        // `127.0.0.1` **sem esquema** é agora a forma canónica: o protocolo vem do profile.
        assert!(OutputConfig::resolve(&perfil, "127.0.0.1", 10).is_ok());
        assert!(
            OutputConfig::resolve(&perfil, "127.0.0.1", 0).is_err(),
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
