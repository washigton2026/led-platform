//! GS4.2 — o pré-voo, agora **com fio para proteger**.
//!
//! ## O que mudou, e porquê podia mudar sozinho
//!
//! No GS2 `network_ok` e `devices_present` eram verdadeiros por **vacuidade**: um processo
//! sem saída não pode enviar por WiFi nem perder um controlador. Não era um atalho — era
//! logicamente correto, e ficou registado que desapareceria sozinho quando a saída existisse.
//! É este ficheiro a cumpri-lo: **com `--output`, os dois campos passam a ser medidos**.
//!
//! ## As sondas são injetadas
//!
//! [`preflight`] recebe [`NetworkGuard`] e [`DevicePresence`] como **dados**, não os
//! constrói. É a mesma disciplina do validador do ADR-0018, e é o que torna a *lógica* do
//! pré-voo falsificável sem rede, sem WiFi e sem hardware — que é precisamente a parte que
//! não se pode testar no rig quando o rig não existe.
//!
//! ## Sonda indisponível ≠ sonda falhada ≠ sonda reprovada
//!
//! Se a sonda não consegue medir, não se pode concluir nada — nem "há WiFi" nem "não há". Há
//! dois casos, e só um deles foi alguma vez decidido:
//!
//! - **`ProbeUnavailable` — não há sonda para esta plataforma.** É o caso que o ADR-0005
//!   («Consequências») tornou não-fatal em 2026-06-25: **prossegue com aviso**
//!   (`network_unverified`), para não bloquear ambientes sem hardware.
//! - **`ProbeFailed` — a plataforma é suportada e a sonda falhou** (TD-029). Nunca foi
//!   decidido como não-fatal, e por isso **bloqueia** (`network_probe_failed`). O operador pode
//!   AFIRMAR que não há WiFi com `--assume-no-wifi`, por execução; cada pré-voo que o use
//!   regista `network_assumed_by_operator` com a causa da falha. A flag **nunca** desbloqueia
//!   WiFi ativo, e quando a sonda verifica fica `network_override_unused`.
//!
//! O que este módulo garante é que a diferença fica **escrita no journal**: "verificado",
//! "não foi possível verificar" e "afirmado pelo operador" nunca aparecem com a mesma frase —
//! DADO o que a guarda devolve. Uma sonda por interface que falha em silêncio (TD-029, residuais
//! (b)/(d)) chega aqui como `Ok` e é registada como `network_checked`.

use crate::loader::Integrity;
use crate::output::OutputConfig;
use led_hal::{NetworkGuard, NetworkPolicyError};
use std::net::IpAddr;
use std::time::Duration;

/// Quanto tempo esperar por um `ArtPollReply` antes de considerar o controlador calado.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1_500);

/// O veredito de uma sonda de presença de controladores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Todos os esperados responderam.
    AllPresent,
    /// Alguém ficou calado. **É o footgun do palco escuro** (RT-003), apanhado antes do show.
    Missing(Vec<String>),
    /// Não foi possível sondar (sem permissão para a porta, sem rota, alvo local…).
    Unavailable(String),
}

/// Descobrir controladores é uma **capacidade injetada**, para que o pré-voo se possa testar
/// sem rede — e para que o alvo de loopback dos testes não finja ser um rig.
pub trait DevicePresence {
    fn probe(&self, target: IpAddr) -> Presence;
    fn name(&self) -> &'static str;
}

/// A sonda real: ArtPoll em broadcast, a mesma que o `led-player --require-all` usa.
pub struct ArtPollPresence;

impl DevicePresence for ArtPollPresence {
    fn probe(&self, target: IpAddr) -> Presence {
        let IpAddr::V4(ip) = target else {
            return Presence::Unavailable("ArtPoll é IPv4; alvo é IPv6".into());
        };
        // Um alvo de loopback é um socket local, não um controlador: sondá-lo por broadcast
        // não prova nada, e responder "presente" seria inventar um rig que não existe.
        if ip.is_loopback() {
            return Presence::Unavailable(format!("{ip} é loopback — não há rig a descobrir"));
        }
        match led_protocols::discover_controllers(&[ip], DISCOVERY_TIMEOUT) {
            Ok(r) if r.missing.is_empty() => Presence::AllPresent,
            Ok(r) => Presence::Missing(r.missing.iter().map(|i| i.to_string()).collect()),
            Err(e) => Presence::Unavailable(e.to_string()),
        }
    }
    fn name(&self) -> &'static str {
        "artpoll"
    }
}

pub use politica_rede::{classificar, decidir_rede, DecisaoRede, Sonda, Veredito};
pub use relatorio::Preflight;

/// **A política de rede do pré-voo (D1–D3 do TD-029), provada por TIPOS (R7.1).**
///
/// Módulo próprio de propósito: a privacidade do Rust é por módulo, e é ela a prova.
/// - A ENTRADA de [`decidir_rede`] são só enums sem campos e `bool`s: nenhuma `String`, nome de
///   interface, nome de guarda ou texto de erro chega ao decisor, por isso nenhum deles pode
///   mudar a decisão (ronda 11: N1, N2, N3, N9).
/// - A SAÍDA, [`DecisaoRede`], tem campos privados e nenhum construtor público: fora deste
///   módulo ninguém fabrica nem altera uma decisão (N4, N5, N8 escreviam `network_ok`).
/// - Os textos (interfaces, razão, sonda, erro, nome da guarda) vão para o journal por um
///   caminho SEPARADO, [`notices_da_rede`], que lê a decisão e nunca a escreve.
pub mod politica_rede {
    use led_hal::NetworkPolicyError;

    /// O que a sonda de WiFi conseguiu fazer — sem payload.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum Sonda {
        /// Mediu (e diz se há WiFi ativo no `bool` ao lado).
        Mediu,
        /// Não há sonda para esta plataforma (D1, ADR-0005 «Consequências»).
        NaoSuportada,
        /// A plataforma é suportada e a sonda falhou (D3).
        Falhou,
    }

    /// O que a política decidiu — sem payload. Escolhe as chaves do journal.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Veredito {
        /// `network_checked`.
        Verificado,
        /// `network_checked` + `network_override_unused` (D2(b)).
        VerificadoFlagSemEfeito,
        /// `network_refused` (ADR-0005).
        WifiAtivo,
        /// `network_unverified` (D1).
        NaoVerificado,
        /// `network_probe_failed` (D3).
        SondaFalhou,
        /// `network_assumed_by_operator` (D2).
        AfirmadoPeloOperador,
    }

    /// A decisão. Campos privados, sem construtor público: só [`decidir_rede`] a cria.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DecisaoRede {
        network_ok: bool,
        override_usado: bool,
        veredito: Veredito,
    }

    impl DecisaoRede {
        /// O campo `network_ok` do `PreflightReport`.
        pub fn network_ok(&self) -> bool {
            self.network_ok
        }
        /// O operador AFIRMOU, com `--assume-no-wifi`, o que a sonda não conseguiu verificar.
        pub fn override_usado(&self) -> bool {
            self.override_usado
        }
        pub fn veredito(&self) -> Veredito {
            self.veredito
        }
    }

    /// **O único decisor.** Função pura e total sobre `Sonda × wifi_ativo × assume_no_wifi`.
    ///
    /// - WiFi ativo → bloqueia SEMPRE, com ou sem flag (ADR-0005).
    /// - Mediu sem WiFi → verificado; com a flag, `network_override_unused` (D2(b)).
    /// - Não suportada → prossegue com aviso, igual com ou sem flag (D1).
    /// - Falhou → bloqueia; com a flag, prossegue e o override fica usado (D2, D3).
    pub fn decidir_rede(sonda: Sonda, wifi_ativo: bool, assume_no_wifi: bool) -> DecisaoRede {
        use Veredito::*;
        let (network_ok, override_usado, veredito) = match (sonda, wifi_ativo, assume_no_wifi) {
            (_, true, _) => (false, false, WifiAtivo),
            (Sonda::Mediu, false, false) => (true, false, Verificado),
            (Sonda::Mediu, false, true) => (true, false, VerificadoFlagSemEfeito),
            (Sonda::NaoSuportada, false, _) => (true, false, NaoVerificado),
            (Sonda::Falhou, false, false) => (false, false, SondaFalhou),
            (Sonda::Falhou, false, true) => (true, true, AfirmadoPeloOperador),
        };
        DecisaoRede { network_ok, override_usado, veredito }
    }

    /// Reduz o que a guarda devolveu à entrada do decisor, **descartando o payload**: os
    /// padrões só olham para a variante (`{ .. }`), e o teste estrutural exige-o.
    pub fn classificar(medido: &Result<(), NetworkPolicyError>) -> (Sonda, bool) {
        match medido {
            Ok(()) => (Sonda::Mediu, false),
            Err(NetworkPolicyError::WifiActive { .. }) => (Sonda::Mediu, true),
            Err(NetworkPolicyError::ProbeUnavailable { .. }) => (Sonda::NaoSuportada, false),
            Err(NetworkPolicyError::ProbeFailed { .. }) => (Sonda::Falhou, false),
        }
    }
}

/// **O caminho dos TEXTOS, separado da decisão (R7.1).** Lê a decisão (só leitura) e o que a
/// guarda disse, e escreve as linhas do journal. Não devolve nada que volte a [`decidir_rede`].
pub fn notices_da_rede(
    decisao: &DecisaoRede,
    medido: &Result<(), NetworkPolicyError>,
    nome_guarda: &str,
) -> Vec<(&'static str, String)> {
    // A causa, como a guarda a disse. Fora da variante esperada não há texto a inventar.
    let interfaces = match medido {
        Err(NetworkPolicyError::WifiActive { interfaces }) => interfaces.join(", "),
        _ => "(interface nao reportada)".into(),
    };
    let razao = match medido {
        Err(NetworkPolicyError::ProbeUnavailable { reason }) => reason.clone(),
        _ => "(razao nao reportada)".into(),
    };
    let (probe, error) = match medido {
        Err(NetworkPolicyError::ProbeFailed { probe, error }) => (*probe, error.clone()),
        _ => ("(sonda nao reportada)", "(erro nao reportado)".into()),
    };
    let verificado = || ("network_checked", format!("{nome_guarda}: sem WiFi ativo"));
    match decisao.veredito() {
        Veredito::Verificado => vec![verificado()],
        Veredito::VerificadoFlagSemEfeito => vec![
            verificado(),
            // D2(b): a flag foi dada e a sonda mediu — não houve nada a afirmar.
            (
                "network_override_unused",
                "--assume-no-wifi presente mas a sonda VERIFICOU: a flag nao teve efeito".into(),
            ),
        ],
        Veredito::WifiAtivo => vec![(
            "network_refused",
            format!("WiFi ATIVO em {interfaces} — ADR-0005 proibe show ao vivo"),
        )],
        Veredito::NaoVerificado => vec![(
            "network_unverified",
            format!("NAO foi possivel verificar a rede ({razao}) — prosseguindo com aviso"),
        )],
        Veredito::AfirmadoPeloOperador => vec![(
            "network_assumed_by_operator",
            format!(
                "sonda {probe} FALHOU ({error}); --assume-no-wifi: o operador AFIRMA que nao \
                 ha WiFi ativo — NAO verificado"
            ),
        )],
        Veredito::SondaFalhou => vec![(
            "network_probe_failed",
            format!(
                "sonda {probe} FALHOU ({error}) — output BLOQUEADO; se nao ha WiFi, \
                 --assume-no-wifi permite ao operador afirma-lo"
            ),
        )],
    }
}

/// O resultado do pré-voo, em módulo próprio pela mesma razão que a política: os campos são
/// privados, e o `network_ok` de um pré-voo com fio a proteger só pode vir de uma
/// [`DecisaoRede`] (R7.1: N8 escrevia `pf.report.network_ok`).
pub mod relatorio {
    use super::DecisaoRede;
    use led_daemon::PreflightReport;

    /// O relatório que vai para a máquina de estados **e** o que dizer no journal. As duas
    /// coisas juntas, porque um relatório sem a razão é um veredito sem prova.
    pub struct Preflight {
        report: PreflightReport,
        notices: Vec<(&'static str, String)>,
    }

    impl Preflight {
        /// Sem fio a proteger (sem `--output`, ou só loopback): rede e controladores VACUOSOS.
        pub(crate) fn sem_fio_a_proteger(
            integrity_verified: bool,
            notices: Vec<(&'static str, String)>,
        ) -> Self {
            let report = PreflightReport { integrity_verified, network_ok: true, devices_present: true };
            Preflight { report, notices }
        }

        /// Com fio: o `network_ok` é o da decisão, e não há outra forma de o dar.
        pub(crate) fn com_rede(
            integrity_verified: bool,
            decisao: &DecisaoRede,
            devices_present: bool,
            notices: Vec<(&'static str, String)>,
        ) -> Self {
            let report =
                PreflightReport { integrity_verified, network_ok: decisao.network_ok(), devices_present };
            Preflight { report, notices }
        }

        /// O relatório para o `Arm` (uma cópia: o original não muda).
        pub fn report(&self) -> PreflightReport {
            self.report
        }

        /// As linhas do journal, pela ordem em que saem.
        pub fn notices(&self) -> &[(&'static str, String)] {
            &self.notices
        }

        /// Desfaz o pré-voo nas suas duas partes, para quem escreve as notices e arma.
        pub fn em_partes(self) -> (PreflightReport, Vec<(&'static str, String)>) {
            (self.report, self.notices)
        }
    }
}

/// Corre o pré-voo.
///
/// Com `output == None` os dois campos de rede continuam **vacuosos** — e continua a ser
/// verdade: sem fio, não há fio a proteger. A diferença é que agora isso é o caso
/// excecional, e está dito como tal.
pub fn preflight(
    integrity: Integrity,
    output: Option<&OutputConfig>,
    guard: &dyn NetworkGuard,
    presence: &dyn DevicePresence,
    assume_no_wifi: bool,
) -> Preflight {
    let mut notices = Vec::new();
    let integrity_verified = integrity.satisfies_preflight();

    let Some(cfg) = output else {
        notices.push((
            "preflight_vacuous",
            "sem --output: network_ok e devices_present sao VACUOSOS, nao ha saida a proteger"
                .to_string(),
        ));
        return Preflight::sem_fio_a_proteger(integrity_verified, notices);
    };

    // ── Rede (ADR-0005: WiFi é proibido ao vivo) ─────────────────────────────
    //
    // Um alvo de **loopback** não atravessa interface nenhuma: o datagrama nasce e morre
    // dentro da máquina. O gate do ADR-0005 protege o *fio*, e aqui não há fio para o WiFi
    // corromper — é o mesmo raciocínio da vacuidade do GS2, aplicado a um caso concreto e
    // não à ausência de saída. **Não é um bypass**: um show apontado ao loopback não chega a
    // rig nenhum, e por isso não há nada que a regra pudesse salvar.
    // `todos_loopback`, nunca `any`: basta UM alvo de rede para haver fio a proteger, e um
    // `any` desligaria o gate do ADR-0005 para o rig inteiro por causa dos nos que nao contam
    // (ADR-0029 §6).
    if cfg.todos_loopback() {
        let quais: Vec<String> = cfg.alvos.iter().map(|a| a.addr.ip().to_string()).collect();
        notices.push((
            "network_local",
            format!(
                "{} e loopback: nao atravessa interface, ADR-0005 nao se aplica",
                quais.join(", ")
            ),
        ));
        notices.push((
            "devices_unverified",
            "alvo de loopback: NAO ha controladores a descobrir — prosseguindo com aviso".into(),
        ));
        return Preflight::sem_fio_a_proteger(integrity_verified, notices);
    }

    // TD-029 (R7.1): a guarda só MEDE; `classificar` reduz o que mediu a enums sem payload;
    // `decidir_rede` decide só sobre isso; os textos vão para o journal por `notices_da_rede`.
    let medido = guard.check();
    let (sonda, wifi_ativo) = classificar(&medido);
    let decisao = decidir_rede(sonda, wifi_ativo, assume_no_wifi);
    if decisao.override_usado() {
        // D2(a): o aviso VISÍVEL sai em CADA pré-voo que use o override (não 1× por processo).
        eprintln!(
            "AVISO: --assume-no-wifi USADO neste pré-voo — a sonda de rede falhou e o operador \
             AFIRMA que não há WiFi ativo (NAO verificado)."
        );
    }
    notices.extend(notices_da_rede(&decisao, &medido, guard.name()));

    // ── Controladores (RT-003: palco escuro sem erro) ────────────────────────
    //
    // **Um nó que declara não responder a descoberta não pode ser reprovado por não
    // responder.** Dois presets do catálogo declaram `supports_discovery: false`; sondá-los
    // produziria `devices_missing` e o daemon recusaria tocar — o oposto exato do que a
    // descoberta existe para evitar (RT-003 protege contra palco escuro, não contra shows
    // que não arrancam). O profile declara a capacidade; aqui ela é honrada.
    if !cfg.supports_discovery {
        notices.push((
            "devices_unverified",
            "o no declara supports_discovery:false — NAO foi sondado, e a sua ausencia              nao seria detetavel por ArtPoll"
                .to_string(),
        ));
        return Preflight::com_rede(integrity_verified, &decisao, true, notices);
    }
    // Sondar TODOS os alvos: sondar so o primeiro deixaria os outros por verificar, e o
    // RT-003 existe contra o palco escuro por controlador ausente. A resposta de um no nunca
    // mascara o silencio de outro (ADR-0029 §6).
    let mut ausentes_totais: Vec<String> = Vec::new();
    let mut respondeu: Vec<String> = Vec::new();
    // `Unavailable` de QUALQUER alvo torna o conjunto indeterminado: "nao consegui sondar
    // um" nao pode ser arredondado para "os outros responderam". E a mesma regra das tres
    // categorias do `Inventory` — presente, ausente e NAO SONDADO nunca colapsam em duas.
    let mut indeterminado: Option<String> = None;
    for alvo in &cfg.alvos {
        match presence.probe(alvo.addr.ip()) {
            Presence::AllPresent => respondeu.push(alvo.addr.ip().to_string()),
            Presence::Missing(a) => ausentes_totais.extend(a),
            Presence::Unavailable(porque) => {
                indeterminado.get_or_insert(porque);
            }
        }
    }
    // A ordem é deliberada: **ausente vence indeterminado, que vence presente**. Um nó
    // calado é facto sobre o rig; não conseguir sondar outro não o apaga. É a mesma
    // hierarquia do `Veredito` do `lumyx-hwcheck` — reprovar > não medir > aprovar.
    let agregado = match (ausentes_totais.is_empty(), indeterminado) {
        (false, _) => Presence::Missing(ausentes_totais),
        (true, Some(porque)) => Presence::Unavailable(porque),
        (true, None) => Presence::AllPresent,
    };
    let devices_present = match agregado {
        Presence::AllPresent => {
            notices.push(("devices_checked", format!("{} respondeu", respondeu.join(", "))));
            true
        }
        Presence::Missing(ausentes) => {
            notices.push((
                "devices_missing",
                format!("SEM resposta de {} — palco escuro se o show comecar", ausentes.join(", ")),
            ));
            false
        }
        Presence::Unavailable(razao) => {
            notices.push((
                "devices_unverified",
                format!("NAO foi possivel sondar controladores ({razao}) — prosseguindo com aviso"),
            ));
            true
        }
    };

    Preflight::com_rede(integrity_verified, &decisao, devices_present, notices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_hal::PermissiveGuard;

    struct GuardaFalsa(Result<(), NetworkPolicyError>);
    impl NetworkGuard for GuardaFalsa {
        fn check(&self) -> Result<(), NetworkPolicyError> {
            self.0.clone()
        }
        fn name(&self) -> &'static str {
            "falsa"
        }
    }
    struct SondaFalsa(Presence);
    impl DevicePresence for SondaFalsa {
        fn probe(&self, _: IpAddr) -> Presence {
            self.0.clone()
        }
        fn name(&self) -> &'static str {
            "falsa"
        }
    }

    /// Uma sonda que **responde por endereço**, e conta quem foi sondado.
    ///
    /// A `SondaFalsa` acima devolve o mesmo para qualquer alvo — e isso torna-a incapaz de
    /// distinguir *"sondei todos"* de *"sondei o primeiro"*. Descobri-o ao falsificar:
    /// trocar o laço por `.take(1)` **não reprovava nada**, porque com uma resposta
    /// uniforme os dois casos são idênticos. Um teste que não distingue não prova.
    struct SondaPorEndereco {
        calados: Vec<&'static str>,
        sondados: std::cell::RefCell<Vec<String>>,
    }
    impl DevicePresence for SondaPorEndereco {
        fn probe(&self, ip: IpAddr) -> Presence {
            let s = ip.to_string();
            self.sondados.borrow_mut().push(s.clone());
            if self.calados.contains(&s.as_str()) {
                Presence::Missing(vec![s])
            } else {
                Presence::AllPresent
            }
        }
        fn name(&self) -> &'static str {
            "por-endereco"
        }
    }

    fn saida() -> OutputConfig {
        OutputConfig::resolve(
            &crate::output::profile_by_name("esp32-poe-wled-ddp").unwrap(),
            "192.168.2.156",
            720,
        )
        .unwrap()
    }
    fn corre(g: Result<(), NetworkPolicyError>, p: Presence) -> Preflight {
        preflight(
            Integrity::AssumedByOperator,
            Some(&saida()),
            &GuardaFalsa(g),
            &SondaFalsa(p),
            false,
        )
    }
    fn tem(pf: &Preflight, n: &str) -> bool {
        pf.notices().iter().any(|(k, _)| *k == n)
    }

    /// **ADR-0029 §6 — um rig MISTO continua a invocar o gate do ADR-0005.**
    ///
    /// Este teste existe porque a falsificação mostrou que ele faltava: trocar o `all` por
    /// `any` em `todos_loopback()` **não reprovava nada**, e com razão — todos os outros
    /// testes de pré-voo usam **um** alvo, e com um alvo `all` e `any` são indistinguíveis.
    ///
    /// O defeito que isto apanha é silencioso e caro: quatro nós em loopback e um em
    /// `192.168.2.156` atravessam o fio, e um `any` desligaria a proibição de WiFi para o
    /// rig inteiro por causa dos quatro que não contam. Seria a mutação que o
    /// `num_alvo_de_rede_o_wifi_ativo_reprova_mesmo` já apanhou uma vez, reintroduzida pela
    /// porta do lado.
    #[test]
    fn rig_misto_um_alvo_de_rede_basta_para_o_gate_do_wifi_valer() {
        let mut cfg = saida();
        cfg.alvos = vec![
            crate::output::Alvo {
                addr: "127.0.0.1:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 0,
                pixel_count: 360,
                escapa_blackout: false,
            },
            crate::output::Alvo {
                addr: "192.168.2.156:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 360,
                pixel_count: 360,
                escapa_blackout: false,
            },
        ];
        assert!(!cfg.todos_loopback(), "um alvo de rede basta para NAO ser tudo loopback");

        let pf = preflight(
            Integrity::AssumedByOperator,
            Some(&cfg),
            &GuardaFalsa(Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] })),
            &SondaFalsa(Presence::AllPresent),
            false,
        );

        assert!(
            !pf.report().network_ok,
            "ha um alvo de REDE: o ADR-0005 aplica-se ao rig inteiro, e o WiFi ativo reprova"
        );
        assert!(tem(&pf, "network_refused"));
        assert!(
            !tem(&pf, "network_local"),
            "um rig misto NAO pode ser anunciado como local — seria a excecao a alastrar"
        );
    }

    /// **Controlo negativo do teste acima.** Com *todos* os alvos em loopback, a excepção
    /// continua a valer. Sem isto, o teste anterior passaria mesmo que alguém apagasse a
    /// excepção do loopback por completo — e aí os testes que a usam quebrariam por outra
    /// razão, mascarando qual das duas regras se partiu.
    #[test]
    fn rig_todo_em_loopback_mantem_a_excecao_do_adr_0005() {
        let mut cfg = saida();
        cfg.alvos = vec![
            crate::output::Alvo {
                addr: "127.0.0.1:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 0,
                pixel_count: 360,
                escapa_blackout: false,
            },
            crate::output::Alvo {
                addr: "127.0.0.2:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 360,
                pixel_count: 360,
                escapa_blackout: false,
            },
        ];
        assert!(cfg.todos_loopback());

        let pf = preflight(
            Integrity::AssumedByOperator,
            Some(&cfg),
            &GuardaFalsa(Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] })),
            &SondaFalsa(Presence::AllPresent),
            false,
        );
        assert!(pf.report().network_ok, "nenhum datagrama atravessa interface: a excecao vale");
        assert!(tem(&pf, "network_local"));
    }

    /// **ADR-0029 §6 — um nó calado reprova, mesmo que os outros respondam.**
    ///
    /// O RT-003 existe contra o palco escuro por controlador ausente. Sondar só o primeiro
    /// alvo deixaria os outros por verificar, e a resposta de um nó **nunca** pode mascarar
    /// o silêncio de outro.
    #[test]
    fn um_no_calado_reprova_o_preflight_mesmo_com_os_outros_a_responder() {
        let mut cfg = saida();
        cfg.alvos = vec![
            crate::output::Alvo {
                addr: "192.168.2.156:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 0,
                pixel_count: 360,
                escapa_blackout: false,
            },
            crate::output::Alvo {
                addr: "192.168.2.157:4048".parse().unwrap(),
                first_universe: 1,
                pixel_offset: 360,
                pixel_count: 360,
                escapa_blackout: false,
            },
        ];

        // **O primeiro RESPONDE e o segundo está calado.** É esta assimetria que discrimina:
        // com uma sonda uniforme, sondar um ou dois alvos daria o mesmo resultado e o teste
        // passaria mesmo que só o primeiro fosse consultado — que é o defeito.
        let sonda = SondaPorEndereco {
            calados: vec!["192.168.2.157"],
            sondados: std::cell::RefCell::new(Vec::new()),
        };
        let pf =
            preflight(Integrity::AssumedByOperator, Some(&cfg), &GuardaFalsa(Ok(())), &sonda, false);

        assert_eq!(
            sonda.sondados.borrow().len(),
            2,
            "TODOS os alvos tem de ser sondados; sondados={:?}",
            sonda.sondados.borrow()
        );
        assert!(!pf.report().devices_present, "um no calado reprova o rig, mesmo com o outro a responder");
        assert!(tem(&pf, "devices_missing"));

        let aviso = pf.notices().iter().find(|(k, _)| *k == "devices_missing").unwrap();
        assert!(
            aviso.1.contains("192.168.2.157"),
            "o aviso tem de NOMEAR quem falta — com cinco robos, 'SEM resposta' sem dizer de \
             quem manda o operador procurar em cinco sitios: {}",
            aviso.1
        );
    }

    /// **A vacuidade acabou.** Com saída, os dois campos vêm das sondas — e o teste prova-o
    /// pelo lado que interessa: um `false` de sonda tem de chegar ao relatório.
    #[test]
    fn com_saida_os_campos_deixam_de_ser_vacuosos() {
        let pf = corre(
            Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] }),
            Presence::Missing(vec!["192.168.2.156".into()]),
        );
        assert!(!pf.report().network_ok, "WiFi ativo tem de reprovar (ADR-0005)");
        assert!(!pf.report().devices_present, "controlador calado tem de reprovar (RT-003)");
        assert!(!tem(&pf, "preflight_vacuous"), "com saída, nada é vacuoso");
        assert!(tem(&pf, "network_refused") && tem(&pf, "devices_missing"));
    }

    #[test]
    fn tudo_verificado_aprova_e_diz_que_verificou() {
        let pf = corre(Ok(()), Presence::AllPresent);
        assert!(pf.report().network_ok && pf.report().devices_present);
        assert!(tem(&pf, "network_checked") && tem(&pf, "devices_checked"));
    }

    /// Sonda indisponível deixa prosseguir — **mas o journal não diz "verificado"**. É esta
    /// distinção que impede o aviso de virar um carimbo.
    #[test]
    fn sonda_indisponivel_prossegue_mas_nunca_afirma_ter_verificado() {
        let pf = corre(
            Err(NetworkPolicyError::ProbeUnavailable { reason: "SO nao suportado".into() }),
            Presence::Unavailable("loopback".into()),
        );
        assert!(pf.report().network_ok && pf.report().devices_present, "prossegue");
        assert!(tem(&pf, "network_unverified") && tem(&pf, "devices_unverified"));
        assert!(!tem(&pf, "network_checked"), "NAO pode afirmar que verificou");
        assert!(!tem(&pf, "devices_checked"), "NAO pode afirmar que verificou");
    }

    // ── TD-029: sonda FALHADA numa plataforma suportada ──────────────────────

    fn falhada() -> NetworkPolicyError {
        NetworkPolicyError::ProbeFailed {
            probe: "sysfs /sys/class/net",
            error: "read_dir: permission denied".into(),
        }
    }
    fn corre_com(g: Result<(), NetworkPolicyError>, assume_no_wifi: bool) -> Preflight {
        preflight(
            Integrity::AssumedByOperator,
            Some(&saida()),
            &GuardaFalsa(g),
            &SondaFalsa(Presence::AllPresent),
            assume_no_wifi,
        )
    }
    fn detalhe<'a>(pf: &'a Preflight, n: &str) -> &'a str {
        pf.notices().iter().find(|(k, _)| *k == n).map(|(_, d)| d.as_str()).unwrap_or("")
    }

    /// **A inversão do teste acima para o caso que nunca foi decidido.** Numa plataforma
    /// SUPORTADA a sonda falhou: não se sabe se há WiFi, e o ADR-0005 só tornou não-fatal o
    /// caso «não há sonda para esta plataforma». O output fica BLOQUEADO.
    #[test]
    fn sonda_falhada_numa_plataforma_suportada_bloqueia() {
        let pf = corre_com(Err(falhada()), false);
        assert!(!pf.report().network_ok, "sonda falhada sem override tem de BLOQUEAR (TD-029)");
        assert!(tem(&pf, "network_probe_failed"));
        let d = detalhe(&pf, "network_probe_failed");
        assert!(
            d.contains("sysfs /sys/class/net") && d.contains("permission denied"),
            "a recusa tem de nomear a sonda e o erro: {d}"
        );
        assert!(d.contains("--assume-no-wifi"), "e dizer como o operador pode afirmar: {d}");
        assert!(!tem(&pf, "network_checked") && !tem(&pf, "network_unverified"));
    }

    /// **O caso decidido no ADR-0005 não muda.** Plataforma não suportada continua a
    /// prosseguir com aviso — com ou sem a flag, que aqui não tem nada a afirmar.
    #[test]
    fn sonda_nao_suportada_mantem_o_comportamento_atual() {
        for flag in [false, true] {
            let pf = corre_com(
                Err(NetworkPolicyError::ProbeUnavailable { reason: "SO nao suportado".into() }),
                flag,
            );
            assert!(pf.report().network_ok, "nao suportado continua nao-fatal (flag={flag})");
            assert!(tem(&pf, "network_unverified"), "flag={flag}");
            assert!(!tem(&pf, "network_probe_failed") && !tem(&pf, "network_checked"));
            assert!(!tem(&pf, "network_assumed_by_operator"), "a flag nao se aplica (flag={flag})");
        }
        // D1 «exatamente como hoje»: com a flag, as notices sao IGUAIS as de sem flag — nem um
        // `network_override_unused` (que diria que a sonda VERIFICOU), nem outro (MP8a).
        let sem = corre_com(Err(NetworkPolicyError::ProbeUnavailable { reason: "SO nao suportado".into() }), false);
        let com = corre_com(Err(NetworkPolicyError::ProbeUnavailable { reason: "SO nao suportado".into() }), true);
        assert_eq!(sem.notices(), com.notices(), "nao suportado: a flag nao muda nada no journal");
        assert_eq!(sem.report().network_ok, com.report().network_ok);
    }

    /// **D2 — o override é explícito e deixa rasto.** Sem a flag bloqueia; com ela passa,
    /// mas o journal diz que foi AFIRMADO, com a causa da falha — nunca «verificado».
    #[test]
    fn override_sem_flag_bloqueia_com_flag_passa_e_regista() {
        let sem = corre_com(Err(falhada()), false);
        assert!(!sem.report().network_ok);
        let com = corre_com(Err(falhada()), true);
        assert!(com.report().network_ok, "com --assume-no-wifi o operador assume");
        assert!(tem(&com, "network_assumed_by_operator"));
        let d = detalhe(&com, "network_assumed_by_operator");
        assert!(
            d.contains("sysfs /sys/class/net") && d.contains("permission denied"),
            "o evento tem de registar a CAUSA da falha da sonda: {d}"
        );
        assert!(!tem(&com, "network_checked"), "afirmado nunca e verificado");
        assert!(!tem(&com, "network_probe_failed"), "com override o veredito e outro");
    }

    /// **O override nunca desbloqueia WiFi ATIVO.** É a diferença entre «não consegui ver» e
    /// «vi WiFi»: o operador só pode afirmar o que a sonda não mediu.
    #[test]
    fn override_nunca_desliga_o_bloqueio_de_wifi_ativo() {
        let pf = corre_com(Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] }), true);
        assert!(!pf.report().network_ok, "WiFi ATIVO bloqueia sempre, com ou sem --assume-no-wifi");
        assert!(tem(&pf, "network_refused"));
        assert!(!tem(&pf, "network_assumed_by_operator"));
    }

    /// **D2(b) — flag presente e a sonda verificou.** A flag não teve efeito, e isso também
    /// fica dito: um operador que a usa por hábito tem de o ver.
    #[test]
    fn flag_com_sonda_ok_regista_que_nao_teve_efeito() {
        let pf = corre_com(Ok(()), true);
        assert!(pf.report().network_ok && tem(&pf, "network_checked"));
        assert!(tem(&pf, "network_override_unused"), "a flag sem efeito tem de ficar no journal");
        let sem_flag = corre_com(Ok(()), false);
        assert!(!tem(&sem_flag, "network_override_unused"), "sem flag nao ha nada a dizer");
    }

    /// Sem saída a vacuidade continua correta — e continua a ser **dita**.
    #[test]
    fn sem_saida_continua_vacuoso_e_declarado() {
        let pf = preflight(
            Integrity::AssumedByOperator,
            None,
            &PermissiveGuard,
            &SondaFalsa(Presence::AllPresent),
            false,
        );
        assert!(pf.report().network_ok && pf.report().devices_present);
        assert!(tem(&pf, "preflight_vacuous"));
    }

    /// A integridade nunca foi vacuosa, e a saída não a torna verdadeira.
    #[test]
    fn a_integridade_e_independente_da_saida() {
        for saida_cfg in [None, Some(&saida())] {
            let pf = preflight(
                Integrity::NotVerified,
                saida_cfg,
                &PermissiveGuard,
                &SondaFalsa(Presence::AllPresent),
                false,
            );
            assert!(!pf.report().integrity_verified, "sem afirmação do operador, reprova");
        }
    }

    /// Loopback dispensa o gate do WiFi **por não haver fio**, não por indulgência — e
    /// continua a recusar-se a afirmar que descobriu controladores.
    #[test]
    fn alvo_de_loopback_nao_invoca_o_gate_do_wifi_mas_tambem_nao_carimba_nada() {
        let cfg = OutputConfig::resolve(
            &crate::output::profile_by_name("esp32-poe-wled-ddp").unwrap(),
            "127.0.0.1:9999",
            4,
        )
        .unwrap();
        let pf = preflight(
            Integrity::AssumedByOperator,
            Some(&cfg),
            // Uma guarda que reprovaria SEMPRE: se fosse consultada, o teste falhava.
            &GuardaFalsa(Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] })),
            &SondaFalsa(Presence::AllPresent),
            false,
        );
        assert!(pf.report().network_ok, "loopback não atravessa interface");
        assert!(tem(&pf, "network_local"));
        assert!(!tem(&pf, "network_refused"), "a guarda não devia ter sido consultada");
        assert!(!tem(&pf, "devices_checked"), "e nunca se afirma ter descoberto um rig local");
        assert!(tem(&pf, "devices_unverified"));
    }

    /// **O gate do WiFi é real num alvo real.** É o controle negativo do teste de cima: se o
    /// ramo de loopback alastrasse para endereços de rede, este ficaria vermelho.
    #[test]
    fn num_alvo_de_rede_o_wifi_ativo_reprova_mesmo() {
        let pf = corre(
            Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] }),
            Presence::AllPresent,
        );
        assert!(!pf.report().network_ok, "192.168.2.156 não é loopback: o ADR-0005 aplica-se");
        assert!(tem(&pf, "network_refused"));
    }

    /// **Um nó que declara não responder a descoberta não é reprovado por não responder.**
    ///
    /// A sonda usada aqui devolve `Missing` — o veredito mais severo. Se a guarda deixasse de
    /// existir, o relatório reprovaria e o daemon recusaria tocar um nó que se comporta
    /// exatamente como o seu preset declara.
    #[test]
    fn um_no_que_nao_faz_discovery_nao_e_reprovado_por_nao_responder() {
        let mut cfg = saida();
        cfg.supports_discovery = false;
        let pf = preflight(
            Integrity::AssumedByOperator,
            Some(&cfg),
            &GuardaFalsa(Ok(())),
            &SondaFalsa(Presence::Missing(vec!["192.168.2.156".into()])),
            false,
        );
        assert!(pf.report().devices_present, "declarar que nao responde nao e estar ausente");
        assert!(tem(&pf, "devices_unverified"), "e o journal tem de dizer que NAO sondou");
        assert!(!tem(&pf, "devices_missing"), "nunca pode reportar ausencia de quem nao sonda");
        assert!(!tem(&pf, "devices_checked"), "e muito menos afirmar que verificou");
    }

    /// **Controle negativo do teste acima.** Com `supports_discovery: true`, a mesma sonda a
    /// devolver `Missing` **tem** de reprovar — senão a guarda teria apagado o gate do RT-003.
    #[test]
    fn com_discovery_declarado_a_ausencia_continua_a_reprovar() {
        let pf = corre(Ok(()), Presence::Missing(vec!["192.168.2.156".into()]));
        assert!(!pf.report().devices_present, "o gate do RT-003 continua a valer");
        assert!(tem(&pf, "devices_missing"));
    }

    /// A sonda real **recusa-se a inventar um rig** num alvo de loopback.
    #[test]
    fn a_sonda_real_nao_finge_descobrir_um_loopback() {
        let r = ArtPollPresence.probe("127.0.0.1".parse().unwrap());
        assert!(matches!(r, Presence::Unavailable(_)), "loopback não é um controlador: {r:?}");
    }

    // ── R7.1: a política provada por TIPOS, e a tabela sobre o domínio novo ─────────
    use led_hal::NetworkPolicyError as E;

    /// Força a exaustividade em COMPILAÇÃO: uma `Sonda` nova deixa de compilar aqui, e a tabela
    /// abaixo tem de ganhar as suas linhas.
    fn todas_as_sondas() -> [Sonda; 3] {
        let v = [Sonda::Mediu, Sonda::NaoSuportada, Sonda::Falhou];
        for s in v {
            match s {
                Sonda::Mediu | Sonda::NaoSuportada | Sonda::Falhou => {}
            }
        }
        v
    }

    /// **Tabela exaustiva sobre o domínio da decisão: `Sonda` (3) × `wifi_ativo` × flag = 12
    /// linhas**, sem omissões nem repetições, cada uma com `network_ok`, `override_usado` e o
    /// veredito. A entrada do decisor não tem mais nada: não há payload que possa variar.
    #[test]
    fn decidir_rede_tabela_exaustiva() {
        use Sonda::*;
        use Veredito::*;
        // (sonda, wifi_ativo, flag, network_ok, override_usado, veredito)
        let tabela = [
            (Mediu, false, false, true, false, Verificado),
            (Mediu, false, true, true, false, VerificadoFlagSemEfeito),
            (Mediu, true, false, false, false, WifiAtivo),
            (Mediu, true, true, false, false, WifiAtivo),
            (NaoSuportada, false, false, true, false, NaoVerificado),
            (NaoSuportada, false, true, true, false, NaoVerificado),
            (NaoSuportada, true, false, false, false, WifiAtivo),
            (NaoSuportada, true, true, false, false, WifiAtivo),
            (Falhou, false, false, false, false, SondaFalhou),
            (Falhou, false, true, true, true, AfirmadoPeloOperador),
            (Falhou, true, false, false, false, WifiAtivo),
            (Falhou, true, true, false, false, WifiAtivo),
        ];
        let mut vistas = std::collections::BTreeSet::new();
        for (s, w, f, ..) in &tabela {
            assert!(vistas.insert((*s, *w, *f)), "linha duplicada: {s:?} wifi={w} flag={f}");
        }
        let dominio: Vec<_> = todas_as_sondas()
            .into_iter()
            .flat_map(|s| [false, true].into_iter().flat_map(move |w| [false, true].map(|f| (s, w, f))))
            .collect();
        assert_eq!(dominio.len(), 12);
        for x in &dominio {
            assert!(vistas.contains(x), "linha em falta: {x:?}");
        }
        for (s, w, f, ok, ov, v) in tabela {
            let d = decidir_rede(s, w, f);
            assert_eq!((d.network_ok(), d.override_usado(), d.veredito()), (ok, ov, v), "{s:?} wifi={w} flag={f}");
        }
    }

    /// Os payloads REAIS que as guardas do `led-hal` produzem (network_guard.rs), mais as
    /// interfaces que a ronda 11 usou (`wlx…`, adaptadores USB em Linux — N3).
    fn resultados_reais() -> Vec<(Result<(), E>, Sonda, bool)> {
        let mut v: Vec<(Result<(), E>, Sonda, bool)> = vec![(Ok(()), Sonda::Mediu, false)];
        for i in [vec!["en0"], vec!["wlan0"], vec!["wlx00c0ca123456"], vec!["wlp2s0", "en0"]] {
            let interfaces = i.iter().map(|s| s.to_string()).collect();
            v.push((Err(E::WifiActive { interfaces }), Sonda::Mediu, true));
        }
        // network_guard.rs:149 — o texto real de «SO não suportado» (N9).
        for os in ["windows", "freebsd", "haiku"] {
            let reason = format!("unsupported platform '{os}' — WiFi check not implemented");
            v.push((Err(E::ProbeUnavailable { reason }), Sonda::NaoSuportada, false));
        }
        // network_guard.rs:167/173/255/263 — os textos reais do D3 (N1, N5, N8).
        for (probe, error) in [
            ("networksetup -listallhardwareports", "No such file or directory (os error 2)".to_string()),
            ("networksetup -listallhardwareports", "exit status exit status: 1".to_string()),
            ("sysfs /sys/class/net", "/sys/class/net not found".to_string()),
            ("sysfs /sys/class/net", "read_dir: Permission denied (os error 13)".to_string()),
        ] {
            v.push((Err(E::ProbeFailed { probe, error }), Sonda::Falhou, false));
        }
        v
    }

    /// Os nomes REAIS das guardas (`led-hal`) e um de teste (N2, N4).
    const NOMES_DE_GUARDA: [&str; 3] =
        ["WifiBlockGuard (WiFi-forbidden enforcement)", "PermissiveGuard (no enforcement)", "guarda-teste"];

    struct GuardaNomeada(Result<(), E>, &'static str);
    impl NetworkGuard for GuardaNomeada {
        fn check(&self) -> Result<(), NetworkPolicyError> {
            self.0.clone()
        }
        fn name(&self) -> &'static str {
            self.1
        }
    }

    /// **`classificar` descarta o payload**: com os textos reais, a entrada do decisor é só a
    /// da variante.
    #[test]
    fn classificar_ve_so_a_variante_com_os_textos_reais() {
        for (r, sonda, wifi) in resultados_reais() {
            assert_eq!(classificar(&r), (sonda, wifi), "{r:?}");
        }
    }

    /// **O pré-voo inteiro, com os payloads e os nomes de guarda REAIS × flag:** o `network_ok`
    /// do relatório é o da tabela (escrito à mão, não recalculado por `decidir_rede`), e as
    /// notices nomeiam o que a guarda disse. Um segundo decisor em `preflight` — pelo nome da
    /// guarda, pelo texto do erro — reprova aqui (N4, N5).
    #[test]
    fn o_pre_voo_com_textos_e_nomes_reais_decide_pela_variante() {
        let mut casos = 0;
        for (r, sonda, wifi) in resultados_reais() {
            for nome in NOMES_DE_GUARDA {
                for flag in [false, true] {
                    let pf = preflight(
                        Integrity::AssumedByOperator,
                        Some(&saida()),
                        &GuardaNomeada(r.clone(), nome),
                        &SondaFalsa(Presence::AllPresent),
                        flag,
                    );
                    let esperado = match (sonda, wifi, flag) {
                        (_, true, _) => false,
                        (Sonda::Mediu, false, _) | (Sonda::NaoSuportada, false, _) => true,
                        (Sonda::Falhou, false, f) => f,
                    };
                    assert_eq!(pf.report().network_ok, esperado, "{r:?} guarda={nome} flag={flag}");
                    let texto: String = pf.notices().iter().map(|(k, d)| format!("{k}: {d} | ")).collect();
                    match &r {
                        Ok(()) => assert!(texto.contains(nome), "network_checked nomeia a guarda: {texto}"),
                        Err(E::WifiActive { interfaces }) => {
                            for i in interfaces {
                                assert!(texto.contains(i.as_str()), "a interface {i} tem de constar: {texto}");
                            }
                        }
                        Err(E::ProbeUnavailable { reason }) => assert!(texto.contains(reason.as_str()), "{texto}"),
                        Err(E::ProbeFailed { probe, error }) => {
                            assert!(texto.contains(probe) && texto.contains(error.as_str()), "{texto}");
                            let chave = if flag { "network_assumed_by_operator" } else { "network_probe_failed" };
                            assert!(texto.starts_with(chave), "{texto}");
                            if flag {
                                assert!(texto.contains("NAO verificado"), "{texto}");
                            }
                        }
                    }
                    casos += 1;
                }
            }
        }
        assert_eq!(casos, 12 * 3 * 2, "12 resultados reais × 3 guardas × 2 flags");
    }

    /// **Estrutural (reforço — a prova é a dos tipos):** a guarda é consultada num só sítio, e o
    /// módulo da política não toca em texto. Comentários fora antes de cortar no `mod tests`.
    #[test]
    fn a_politica_nao_ve_texto_e_a_guarda_e_consultada_uma_vez() {
        let sem_comentarios = |t: &str| -> String {
            t.lines().map(|l| match l.find("//") { Some(i) => &l[..i], None => l }).collect::<Vec<_>>().join("\n")
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let (mut sondas, mut ficheiros) = (0, 0);
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            ficheiros += 1;
            let texto = sem_comentarios(&std::fs::read_to_string(&p).unwrap());
            let prod = texto.split("mod tests {").next().unwrap();
            let codigo: String = prod.chars().filter(|c| !c.is_whitespace()).collect();
            sondas += codigo.matches(".check(").count() + codigo.matches("NetworkGuard::check").count();
        }
        assert!(ficheiros >= 5, "premissa: leu o src ({ficheiros})");
        assert_eq!(sondas, 1, "a guarda só é consultada num sítio (o preflight)");

        let fonte = sem_comentarios(include_str!("preflight.rs"));
        let politica = &fonte[fonte.find("pub mod politica_rede {").unwrap()..fonte.find("pub fn notices_da_rede(").unwrap()];
        for proibido in ["String", "&str", "format!", "interfaces", "reason", "error", "name("] {
            assert!(!politica.contains(proibido), "a política não pode ver texto: `{proibido}`");
        }
        let compacto: String = politica.chars().filter(|c| !c.is_whitespace()).collect();
        let padroes = compacto.matches("NetworkPolicyError::").count();
        let sem_campos = compacto.matches("{..})").count();
        assert_eq!((padroes, sem_campos), (3, 3), "classificar só olha para a variante (`{{ .. }}`)");
        // D2(a): o aviso em stderr está guardado pela decisão.
        let depois = &fonte[fonte.find("let decisao = decidir_rede(").expect("a chamada")..];
        let guarda = depois.find("if decisao.override_usado() {").expect("o aviso está guardado pela decisão");
        assert!(depois[guarda..].trim_start_matches("if decisao.override_usado() {").trim_start().starts_with("eprintln!"));
    }

    /// Filho do teste seguinte: dois pré-voos com a sonda falhada e a flag, e um com a sonda OK.
    /// Corre também no conjunto normal (escreve dois avisos em stderr, sem afirmar nada).
    #[test]
    fn filho_dois_pre_voos_com_override() {
        let falhou = || Err(NetworkPolicyError::ProbeFailed { probe: "sonda-x", error: "erro-y".into() });
        let _ = corre_com(falhou(), true);
        let _ = corre_com(falhou(), true);
        let _ = corre_com(Ok(()), true); // sonda OK + flag: sem aviso de USO
    }

    /// **D2(a) medido, com stdout e stderr SEPARADOS (R7.1, N6):** re-executa o PRÓPRIO binário
    /// de teste só com o filho acima, com pipes distintos. Dois pré-voos com override → dois
    /// avisos no **stderr** e **nenhum** no stdout — no binário real o stdout é o journal JSONL,
    /// e um aviso lá dentro seria uma linha não-JSON no journal e nada no terminal.
    #[test]
    fn o_aviso_sai_em_stderr_em_cada_pre_voo_com_override() {
        let o = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "preflight::tests::filho_dois_pre_voos_com_override", "--nocapture", "--test-threads=1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        assert!(o.status.success() && out.contains("1 passed"), "premissa: o filho correu:\n{out}\n{err}");
        assert_eq!(err.matches("AVISO: --assume-no-wifi USADO").count(), 2, "um aviso POR pré-voo, em stderr:\n{err}");
        assert_eq!(out.matches("--assume-no-wifi").count(), 0, "nada do aviso no stdout (o journal):\n{out}");
    }
}
