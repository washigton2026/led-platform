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
use led_hal::NetworkGuard;
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

pub use medicao::Detalhe;
pub use override_da_cli::AssumeNoWifi;
pub use politica_rede::{decidir_rede, DecisaoRede, Fio, Sonda, Veredito};
pub use relatorio::Preflight;

/// **O override do operador como TOKEN (TD-029, R8.5 — NV10).**
///
/// `--assume-no-wifi` deixa de ser um `bool` que qualquer código pode pôr a `true`: é um valor
/// de um tipo sem campos públicos, e o seu ÚNICO construtor lê um argumento da linha de
/// comando. O pré-voo recebe-o (`Option<&AssumeNoWifi>`) e não o pode fabricar nem forçar.
pub mod override_da_cli {
    /// O operador passou `--assume-no-wifi` NESTA execução.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AssumeNoWifi {
        _so_pela_cli: (),
    }

    impl AssumeNoWifi {
        /// O único construtor. `Some` sse um dos argumentos é exatamente `--assume-no-wifi`.
        /// Em produção é chamado só pelo parser da CLI (`main.rs`), com o argumento que leu.
        pub fn da_linha_de_comando<S: AsRef<str>>(argv: &[S]) -> Option<Self> {
            argv.iter()
                .any(|a| a.as_ref() == "--assume-no-wifi")
                .then_some(AssumeNoWifi { _so_pela_cli: () })
        }
    }
}

/// **A política de rede do pré-voo (D1–D3 do TD-029), provada por TIPOS (R7.1, R8.5).**
///
/// Módulo próprio de propósito: a privacidade do Rust é por módulo, e é ela a prova.
/// - A ENTRADA de [`decidir_rede`] são só enums sem campos, um `bool` e o token do override:
///   nenhum texto chega ao decisor (N1, N2, N3, N9).
/// - «Sem fio a proteger» é uma ENTRADA ([`Fio`]), não um atalho: não há outro caminho para um
///   `network_ok` (NV1, NV3).
/// - A SAÍDA, [`DecisaoRede`], tem campos privados e nenhum construtor público.
/// - As CHAVES do journal são função só do [`Veredito`] ([`Veredito::chaves`]); os textos vêm
///   do [`Detalhe`] opaco, noutro módulo.
pub mod politica_rede {
    use super::AssumeNoWifi;

    /// Há fio a proteger?
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum Fio {
        /// Sem `--output`: nenhum frame deixa o processo (vacuidade do GS2).
        SemSaida,
        /// Todos os alvos em loopback: nenhum datagrama atravessa interface.
        SoLoopback,
        /// Pelo menos um alvo de rede: o ADR-0005 aplica-se.
        Rede,
    }

    /// O que a sonda de WiFi conseguiu fazer — sem payload.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum Sonda {
        /// A guarda não foi consultada (sem fio a proteger).
        NaoConsultada,
        /// Mediu (e diz se há WiFi ativo no `bool` ao lado).
        Mediu,
        /// Não há sonda para esta plataforma (D1, ADR-0005 «Consequências»).
        NaoSuportada,
        /// A plataforma é suportada e a sonda falhou (D3).
        Falhou,
    }

    /// O que a política decidiu — sem payload.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Veredito {
        SemSaida,
        Loopback,
        Verificado,
        VerificadoFlagSemEfeito,
        WifiAtivo,
        NaoVerificado,
        SondaFalhou,
        AfirmadoPeloOperador,
    }

    impl Veredito {
        /// As chaves do journal deste veredito, por ordem. Só daqui — o caminho dos textos
        /// escreve o detalhe de cada uma, mas não escolhe quais saem (NV4).
        pub fn chaves(self) -> &'static [&'static str] {
            match self {
                Veredito::SemSaida => &["preflight_vacuous"],
                Veredito::Loopback => &["network_local"],
                Veredito::Verificado => &["network_checked"],
                Veredito::VerificadoFlagSemEfeito => &["network_checked", "network_override_unused"],
                Veredito::WifiAtivo => &["network_refused"],
                Veredito::NaoVerificado => &["network_unverified"],
                Veredito::SondaFalhou => &["network_probe_failed"],
                Veredito::AfirmadoPeloOperador => &["network_assumed_by_operator"],
            }
        }
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

    /// **O único decisor.** Função pura e total sobre `Fio × Sonda × wifi_ativo × override`.
    ///
    /// - WiFi ativo → bloqueia SEMPRE, com ou sem override (ADR-0005).
    /// - Sem saída / só loopback → `network_ok` vacuoso, verdadeiro e dito como tal.
    /// - Rede: mediu sem WiFi → verificado (com o override: `network_override_unused`, D2(b));
    ///   não suportada → prossegue com aviso (D1); falhou → bloqueia, salvo override (D2, D3);
    ///   não consultada com fio → bloqueia sempre (não há medição que o override possa suprir).
    pub fn decidir_rede(
        fio: Fio,
        sonda: Sonda,
        wifi_ativo: bool,
        override_da_cli: Option<&AssumeNoWifi>,
    ) -> DecisaoRede {
        use Veredito::*;
        let afirma = override_da_cli.is_some();
        let (network_ok, override_usado, veredito) = match (fio, sonda, wifi_ativo, afirma) {
            (_, _, true, _) => (false, false, WifiAtivo),
            (Fio::SemSaida, _, false, _) => (true, false, SemSaida),
            (Fio::SoLoopback, _, false, _) => (true, false, Loopback),
            (Fio::Rede, Sonda::Mediu, false, false) => (true, false, Verificado),
            (Fio::Rede, Sonda::Mediu, false, true) => (true, false, VerificadoFlagSemEfeito),
            (Fio::Rede, Sonda::NaoSuportada, false, _) => (true, false, NaoVerificado),
            (Fio::Rede, Sonda::NaoConsultada, false, _) => (false, false, SondaFalhou),
            (Fio::Rede, Sonda::Falhou, false, false) => (false, false, SondaFalhou),
            (Fio::Rede, Sonda::Falhou, false, true) => (true, true, AfirmadoPeloOperador),
        };
        DecisaoRede { network_ok, override_usado, veredito }
    }
}

/// **A medição e os TEXTOS, separados da decisão (R8.5 — NV2, NV4).**
///
/// [`medir`] consulta a guarda e devolve a entrada do decisor (sem payload) e um [`Detalhe`]
/// OPACO: o texto do erro, as interfaces e o nome da guarda ficam em campos privados, e só
/// [`Detalhe::notices`] os lê — para o journal. O pré-voo não vê o texto do erro.
pub mod medicao {
    use super::politica_rede::{DecisaoRede, Sonda};
    use led_hal::{NetworkGuard, NetworkPolicyError};

    /// O que a guarda disse (ou porque não foi consultada), opaco fora deste módulo.
    pub struct Detalhe {
        medido: Option<Result<(), NetworkPolicyError>>,
        nome_guarda: &'static str,
        loopback: Vec<String>,
    }

    /// Consulta a guarda UMA vez e reduz o resultado à entrada do decisor, **descartando o
    /// payload**: os padrões só olham para a variante (`{ .. }`).
    pub fn medir(guard: &dyn NetworkGuard) -> (Sonda, bool, Detalhe) {
        let medido = guard.check();
        let (sonda, wifi_ativo) = match &medido {
            Ok(()) => (Sonda::Mediu, false),
            Err(NetworkPolicyError::WifiActive { .. }) => (Sonda::Mediu, true),
            Err(NetworkPolicyError::ProbeUnavailable { .. }) => (Sonda::NaoSuportada, false),
            Err(NetworkPolicyError::ProbeFailed { .. }) => (Sonda::Falhou, false),
        };
        (sonda, wifi_ativo, Detalhe { medido: Some(medido), nome_guarda: guard.name(), loopback: Vec::new() })
    }

    impl Detalhe {
        /// Sem consulta à guarda: sem saída, ou só loopback (os endereços vão para o journal).
        pub(crate) fn sem_consulta(loopback: Vec<String>) -> Self {
            Detalhe { medido: None, nome_guarda: "", loopback }
        }

        /// As linhas do journal: as CHAVES são as do veredito, por ordem
        /// ([`super::Veredito::chaves`]); daqui só sai o detalhe de cada uma.
        pub fn notices(&self, decisao: &DecisaoRede) -> Vec<(&'static str, String)> {
            let interfaces = match &self.medido {
                Some(Err(NetworkPolicyError::WifiActive { interfaces })) => interfaces.join(", "),
                _ => "(interface nao reportada)".into(),
            };
            let razao = match &self.medido {
                Some(Err(NetworkPolicyError::ProbeUnavailable { reason })) => reason.clone(),
                _ => "(razao nao reportada)".into(),
            };
            let (probe, error) = match &self.medido {
                Some(Err(NetworkPolicyError::ProbeFailed { probe, error })) => (*probe, error.clone()),
                None => ("(guarda nao consultada)", "(sem medicao)".into()),
                _ => ("(sonda nao reportada)", "(erro nao reportado)".into()),
            };
            let nome = self.nome_guarda;
            let loopback = self.loopback.join(", ");
            decisao
                .veredito()
                .chaves()
                .iter()
                .map(|&chave| {
                    let detalhe = match chave {
                        "preflight_vacuous" => "sem --output: network_ok e devices_present sao VACUOSOS, nao ha \
                                                saida a proteger"
                            .to_string(),
                        "network_local" => {
                            format!("{loopback} e loopback: nao atravessa interface, ADR-0005 nao se aplica")
                        }
                        "network_checked" => format!("{nome}: sem WiFi ativo"),
                        "network_override_unused" => {
                            "--assume-no-wifi presente mas a sonda VERIFICOU: a flag nao teve efeito".to_string()
                        }
                        "network_refused" => format!("WiFi ATIVO em {interfaces} — ADR-0005 proibe show ao vivo"),
                        "network_unverified" => {
                            format!("NAO foi possivel verificar a rede ({razao}) — prosseguindo com aviso")
                        }
                        "network_assumed_by_operator" => format!(
                            "sonda {probe} FALHOU ({error}); --assume-no-wifi: o operador AFIRMA que nao ha \
                             WiFi ativo — NAO verificado"
                        ),
                        "network_probe_failed" => format!(
                            "sonda {probe} FALHOU ({error}) — output BLOQUEADO; se nao ha WiFi, \
                             --assume-no-wifi permite ao operador afirma-lo"
                        ),
                        outra => format!("(sem texto para {outra})"),
                    };
                    (chave, detalhe)
                })
                .collect()
        }
    }
}

/// O resultado do pré-voo, em módulo próprio: campos privados e UM só construtor, que exige uma
/// [`DecisaoRede`] (R7.1; R8.5 — NV1, NV3: já não há `sem_fio_a_proteger`).
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
        /// O ÚNICO construtor — e o único `PreflightReport { .. }` do crate (NV8: o teste
        /// estrutural conta-o). O `network_ok` é sempre o da decisão.
        pub(crate) fn novo(
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
/// Sem fio a proteger (sem `--output`, ou só loopback) a rede e os controladores são
/// **vacuosos** — e continua a ser verdade. A diferença é que agora isso é uma entrada do
/// decisor ([`Fio`]), e está dito como tal no journal.
pub fn preflight(
    integrity: Integrity,
    output: Option<&OutputConfig>,
    guard: &dyn NetworkGuard,
    presence: &dyn DevicePresence,
    override_da_cli: Option<&AssumeNoWifi>,
) -> Preflight {
    let mut notices = Vec::new();
    let integrity_verified = integrity.satisfies_preflight();

    // ── Rede (ADR-0005: WiFi é proibido ao vivo) ─────────────────────────────
    //
    // Um alvo de **loopback** não atravessa interface nenhuma: o datagrama nasce e morre
    // dentro da máquina. O gate do ADR-0005 protege o *fio*, e aqui não há fio para o WiFi
    // corromper. **Não é um bypass**: um show apontado ao loopback não chega a rig nenhum.
    // `todos_loopback`, nunca `any`: basta UM alvo de rede para haver fio a proteger (ADR-0029 §6).
    // TD-029 (R8.5): só a guarda MEDE (e só com fio); o texto fica no `Detalhe` opaco; o decisor
    // recebe o fio, a sonda sem payload e o token do override, e é o ÚNICO a produzir network_ok.
    let (fio, sonda, wifi_ativo, detalhe) = match output {
        None => (Fio::SemSaida, Sonda::NaoConsultada, false, Detalhe::sem_consulta(Vec::new())),
        Some(cfg) if cfg.todos_loopback() => {
            let quais = cfg.alvos.iter().map(|a| a.addr.ip().to_string()).collect();
            (Fio::SoLoopback, Sonda::NaoConsultada, false, Detalhe::sem_consulta(quais))
        }
        Some(_) => {
            let (sonda, wifi_ativo, detalhe) = medicao::medir(guard);
            (Fio::Rede, sonda, wifi_ativo, detalhe)
        }
    };
    let decisao = decidir_rede(fio, sonda, wifi_ativo, override_da_cli);
    if decisao.override_usado() {
        // D2(a): o aviso VISÍVEL sai em CADA pré-voo que use o override (não 1× por processo).
        eprintln!(
            "AVISO: --assume-no-wifi USADO neste pré-voo — a sonda de rede falhou e o operador \
             AFIRMA que não há WiFi ativo (NAO verificado)."
        );
    }
    notices.extend(detalhe.notices(&decisao));

    // ── Controladores (RT-003: palco escuro sem erro) ────────────────────────
    let devices_present = match output {
        // Sem saída: vacuoso, e o `preflight_vacuous` acima já o diz para os dois campos.
        None => true,
        Some(cfg) if cfg.todos_loopback() => {
            notices.push((
                "devices_unverified",
                "alvo de loopback: NAO ha controladores a descobrir — prosseguindo com aviso".into(),
            ));
            true
        }
        // **Um nó que declara não responder a descoberta não pode ser reprovado por não
        // responder** (dois presets do catálogo declaram `supports_discovery: false`).
        Some(cfg) if !cfg.supports_discovery => {
            notices.push((
                "devices_unverified",
                "o no declara supports_discovery:false — NAO foi sondado, e a sua ausencia nao seria \
                 detetavel por ArtPoll"
                    .to_string(),
            ));
            true
        }
        Some(cfg) => presenca_de_todos(cfg, presence, &mut notices),
    };

    Preflight::novo(integrity_verified, &decisao, devices_present, notices)
}

/// Sonda TODOS os alvos: sondar só o primeiro deixaria os outros por verificar, e o RT-003
/// existe contra o palco escuro por controlador ausente (ADR-0029 §6). **Ausente vence
/// indeterminado, que vence presente** — a hierarquia do `Veredito` do `lumyx-hwcheck`.
fn presenca_de_todos(
    cfg: &OutputConfig,
    presence: &dyn DevicePresence,
    notices: &mut Vec<(&'static str, String)>,
) -> bool {
    let mut ausentes_totais: Vec<String> = Vec::new();
    let mut respondeu: Vec<String> = Vec::new();
    // `Unavailable` de QUALQUER alvo torna o conjunto indeterminado: presente, ausente e NAO
    // SONDADO nunca colapsam em duas categorias.
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
    let agregado = match (ausentes_totais.is_empty(), indeterminado) {
        (false, _) => Presence::Missing(ausentes_totais),
        (true, Some(porque)) => Presence::Unavailable(porque),
        (true, None) => Presence::AllPresent,
    };
    match agregado {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_hal::{NetworkPolicyError, PermissiveGuard};

    /// O token do override como o parser da CLI o cria (o único construtor).
    fn token(flag: bool) -> Option<AssumeNoWifi> {
        if flag { AssumeNoWifi::da_linha_de_comando(&["--assume-no-wifi"]) } else { None }
    }

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
            None,
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
            None,
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
            None,
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
            preflight(Integrity::AssumedByOperator, Some(&cfg), &GuardaFalsa(Ok(())), &sonda, None);

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
            token(assume_no_wifi).as_ref(),
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
            None,
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
                None,
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
            None,
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
            None,
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

    // ── R8.5: a política sobre Fio × Sonda × wifi × override, e o rig REAL ────────────
    use led_hal::NetworkPolicyError as E;

    /// Exaustividade em COMPILAÇÃO: uma variante nova de `Fio`/`Sonda` deixa de compilar aqui.
    fn todos_os_fios() -> [Fio; 3] {
        let v = [Fio::SemSaida, Fio::SoLoopback, Fio::Rede];
        for f in v {
            match f {
                Fio::SemSaida | Fio::SoLoopback | Fio::Rede => {}
            }
        }
        v
    }
    fn todas_as_sondas() -> [Sonda; 4] {
        let v = [Sonda::NaoConsultada, Sonda::Mediu, Sonda::NaoSuportada, Sonda::Falhou];
        for s in v {
            match s {
                Sonda::NaoConsultada | Sonda::Mediu | Sonda::NaoSuportada | Sonda::Falhou => {}
            }
        }
        v
    }

    /// **Tabela exaustiva: `Fio` (3) × `Sonda` (4) × `wifi_ativo` × override = 48 linhas.**
    /// As 16 com fio de rede estão escritas à mão, uma a uma; nas 32 sem fio a proteger o
    /// WiFi ativo continua a bloquear e o resto é vacuoso e dito como tal.
    #[test]
    fn decidir_rede_tabela_exaustiva() {
        use Sonda::*;
        use Veredito::*;
        // (sonda, wifi_ativo, override, network_ok, override_usado, veredito) — com Fio::Rede
        let rede = [
            (NaoConsultada, false, false, false, false, SondaFalhou),
            (NaoConsultada, false, true, false, false, SondaFalhou),
            (NaoConsultada, true, false, false, false, WifiAtivo),
            (NaoConsultada, true, true, false, false, WifiAtivo),
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
        for (s, w, o, ok, ov, v) in rede {
            assert!(vistas.insert((s, w, o)), "linha duplicada");
            let d = decidir_rede(Fio::Rede, s, w, token(o).as_ref());
            assert_eq!((d.network_ok(), d.override_usado(), d.veredito()), (ok, ov, v), "Rede {s:?} wifi={w} ov={o}");
        }
        let mut linhas = vistas.len();
        for fio in todos_os_fios() {
            for s in todas_as_sondas() {
                for w in [false, true] {
                    for o in [false, true] {
                        if fio == Fio::Rede {
                            assert!(vistas.contains(&(s, w, o)), "linha em falta: {s:?} {w} {o}");
                            continue;
                        }
                        let d = decidir_rede(fio, s, w, token(o).as_ref());
                        let esperado = match (fio, w) {
                            (_, true) => (false, false, WifiAtivo),
                            (Fio::SemSaida, false) => (true, false, SemSaida),
                            (Fio::SoLoopback, false) => (true, false, Loopback),
                            (Fio::Rede, _) => unreachable!(),
                        };
                        assert_eq!((d.network_ok(), d.override_usado(), d.veredito()), esperado, "{fio:?} {s:?} {w} {o}");
                        linhas += 1;
                    }
                }
            }
        }
        assert_eq!(linhas, 48, "3 × 4 × 2 × 2");
    }

    /// As chaves do journal de cada veredito — a tabela que o caminho dos textos tem de seguir.
    #[test]
    fn as_chaves_de_cada_veredito() {
        use Veredito::*;
        let t: [(Veredito, &[&str]); 8] = [
            (SemSaida, &["preflight_vacuous"]),
            (Loopback, &["network_local"]),
            (Verificado, &["network_checked"]),
            (VerificadoFlagSemEfeito, &["network_checked", "network_override_unused"]),
            (WifiAtivo, &["network_refused"]),
            (NaoVerificado, &["network_unverified"]),
            (SondaFalhou, &["network_probe_failed"]),
            (AfirmadoPeloOperador, &["network_assumed_by_operator"]),
        ];
        for (v, chaves) in t {
            assert_eq!(v.chaves(), chaves, "{v:?}");
        }
    }

    /// **O token só nasce de `--assume-no-wifi` exato** (NV10).
    #[test]
    fn o_token_do_override_so_nasce_do_argumento_exato() {
        assert!(AssumeNoWifi::da_linha_de_comando(&["--assume-no-wifi"]).is_some());
        for nao in [&[][..], &["--assume-no-wifi=1"], &["--assume-integrity"], &["assume-no-wifi"], &["--ASSUME-NO-WIFI"]] {
            assert!(AssumeNoWifi::da_linha_de_comando(nao).is_none(), "{nao:?}");
        }
    }

    /// Os payloads REAIS das guardas do `led-hal` (network_guard.rs) e o resultado que a
    /// política tem de dar: (resultado, arma sem override, arma com override, chaves de rede
    /// sem override, chaves com override).
    type Real = (Result<(), E>, bool, bool, &'static [&'static str], &'static [&'static str]);
    fn resultados_reais() -> Vec<Real> {
        let mut v: Vec<Real> = vec![(Ok(()), true, true, &["network_checked"], &["network_checked", "network_override_unused"])];
        for i in [vec!["en0"], vec!["wlan0"], vec!["wlx00c0ca123456"], vec!["wlp2s0", "en0"]] {
            let interfaces = i.iter().map(|s| s.to_string()).collect();
            v.push((Err(E::WifiActive { interfaces }), false, false, &["network_refused"], &["network_refused"]));
        }
        // network_guard.rs:149 — o texto real de «SO não suportado» (N9).
        for os in ["windows", "freebsd", "haiku"] {
            let reason = format!("unsupported platform '{os}' — WiFi check not implemented");
            v.push((Err(E::ProbeUnavailable { reason }), true, true, &["network_unverified"], &["network_unverified"]));
        }
        // network_guard.rs:167/173/255/263 — os textos reais do D3 (N1, N5, N8, NV2 «os error 35»).
        for (probe, error) in [
            ("networksetup -listallhardwareports", "No such file or directory (os error 2)"),
            ("networksetup -listallhardwareports", "Resource temporarily unavailable (os error 35)"),
            ("networksetup -listallhardwareports", "exit status exit status: 1"),
            ("sysfs /sys/class/net", "/sys/class/net not found"),
            ("sysfs /sys/class/net", "read_dir: Permission denied (os error 13)"),
        ] {
            v.push((
                Err(E::ProbeFailed { probe, error: error.into() }),
                false,
                true,
                &["network_probe_failed"],
                &["network_assumed_by_operator"],
            ));
        }
        v
    }

    /// Os nomes REAIS das guardas (`led-hal`) e um de teste (N2, N4, NV4).
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

    /// Os 5 nós do rig real (ADR-0029: um show repartido por 5 controladores), em rede.
    const RIG: [&str; 5] = ["192.168.2.161", "192.168.2.162", "192.168.2.163", "192.168.2.164", "192.168.2.165"];
    fn rig_real(discovery: bool) -> OutputConfig {
        let mut cfg = saida();
        cfg.alvos = RIG
            .iter()
            .enumerate()
            .map(|(i, ip)| crate::output::Alvo {
                addr: format!("{ip}:4048").parse().unwrap(),
                first_universe: 1,
                pixel_offset: i as u32 * 144,
                pixel_count: 144,
                escapa_blackout: false,
            })
            .collect();
        cfg.pixel_count = 720;
        cfg.supports_discovery = discovery;
        cfg
    }

    /// Presença por endereço: uns calados, uns que não se conseguem sondar, os outros respondem.
    struct PresencaMista {
        calados: &'static [&'static str],
        indisponiveis: &'static [&'static str],
    }
    impl DevicePresence for PresencaMista {
        fn probe(&self, ip: IpAddr) -> Presence {
            let s = ip.to_string();
            if self.calados.contains(&s.as_str()) {
                Presence::Missing(vec![s])
            } else if self.indisponiveis.contains(&s.as_str()) {
                Presence::Unavailable(format!("{s}: sem rota"))
            } else {
                Presence::AllPresent
            }
        }
        fn name(&self) -> &'static str {
            "mista"
        }
    }

    /// **O rig REAL, linha a linha (R8.5 — NV1, NV3, NV4):** 5 alvos de rede, discovery ligado
    /// e desligado, presença toda / um calado / um impossível de sondar, os 13 payloads reais das
    /// guardas × os nomes reais × override sim/não. Em CADA pré-voo: as chaves do journal, por
    /// ordem, são EXATAMENTE as esperadas (incluindo «flag sem efeito»); `network_ok` e
    /// `devices_present` são os da tabela escrita à mão; os detalhes nomeiam o que a guarda disse.
    #[test]
    fn o_rig_real_de_5_alvos_journal_linha_a_linha() {
        let presencas: [(PresencaMista, &[&str], bool); 3] = [
            (PresencaMista { calados: &[], indisponiveis: &[] }, &["devices_checked"], true),
            (PresencaMista { calados: &["192.168.2.164"], indisponiveis: &[] }, &["devices_missing"], false),
            (PresencaMista { calados: &[], indisponiveis: &["192.168.2.163"] }, &["devices_unverified"], true),
        ];
        let mut casos = 0;
        for (r, arma_sem, arma_com, chaves_sem, chaves_com) in resultados_reais() {
            for nome in NOMES_DE_GUARDA {
                for ov in [false, true] {
                    for discovery in [true, false] {
                        for (presenca, chaves_dev, dev_presente) in &presencas {
                            let cfg = rig_real(discovery);
                            let pf = preflight(
                                Integrity::AssumedByOperator,
                                Some(&cfg),
                                &GuardaNomeada(r.clone(), nome),
                                presenca,
                                token(ov).as_ref(),
                            );
                            let caso = format!("{r:?} guarda={nome} ov={ov} discovery={discovery} {chaves_dev:?}");
                            let mut esperadas: Vec<&str> = if ov { chaves_com.to_vec() } else { chaves_sem.to_vec() };
                            let (dev_chaves, dev_ok): (&[&str], bool) =
                                if discovery { (chaves_dev, *dev_presente) } else { (&["devices_unverified"], true) };
                            esperadas.extend_from_slice(dev_chaves);
                            let obtidas: Vec<&str> = pf.notices().iter().map(|(k, _)| *k).collect();
                            assert_eq!(obtidas, esperadas, "journal linha a linha: {caso}");
                            assert_eq!(pf.report().network_ok, if ov { arma_com } else { arma_sem }, "{caso}");
                            assert_eq!(pf.report().devices_present, dev_ok, "{caso}");
                            let texto: String = pf.notices().iter().map(|(k, d)| format!("{k}: {d} | ")).collect();
                            match &r {
                                Ok(()) => assert!(texto.contains(nome), "network_checked nomeia a guarda: {texto}"),
                                Err(E::WifiActive { interfaces }) => {
                                    for i in interfaces {
                                        assert!(texto.contains(i.as_str()), "{i}: {texto}");
                                    }
                                }
                                Err(E::ProbeUnavailable { reason }) => assert!(texto.contains(reason.as_str()), "{texto}"),
                                Err(E::ProbeFailed { probe, error }) => {
                                    assert!(texto.contains(probe) && texto.contains(error.as_str()), "{texto}");
                                    if ov {
                                        assert!(texto.contains("NAO verificado"), "{texto}");
                                    }
                                }
                            }
                            if discovery && !*dev_presente {
                                assert!(texto.contains("192.168.2.164"), "o nó calado é nomeado: {texto}");
                            }
                            casos += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(casos, 13 * 3 * 2 * 2 * 3, "13 resultados × 3 guardas × override × discovery × 3 presenças");
    }

    /// Sem fio a proteger, também linha a linha: sem saída e só loopback (5 nós locais).
    #[test]
    fn sem_fio_a_proteger_e_uma_entrada_do_decisor_e_fica_no_journal() {
        for ov in [false, true] {
            let pf = preflight(Integrity::AssumedByOperator, None, &GuardaNomeada(Err(E::WifiActive { interfaces: vec!["en0".into()] }), "g"), &SondaFalsa(Presence::AllPresent), token(ov).as_ref());
            let k: Vec<&str> = pf.notices().iter().map(|(k, _)| *k).collect();
            assert_eq!(k, ["preflight_vacuous"], "a guarda nem é consultada sem saída");
            assert!(pf.report().network_ok && pf.report().devices_present);
            let mut cfg = rig_real(true);
            for (i, a) in cfg.alvos.iter_mut().enumerate() {
                a.addr = format!("127.0.0.{}:4048", i + 1).parse().unwrap();
            }
            let pf = preflight(Integrity::AssumedByOperator, Some(&cfg), &GuardaNomeada(Err(E::ProbeFailed { probe: "p", error: "e".into() }), "g"), &SondaFalsa(Presence::Missing(vec!["x".into()])), token(ov).as_ref());
            let k: Vec<&str> = pf.notices().iter().map(|(k, _)| *k).collect();
            assert_eq!(k, ["network_local", "devices_unverified"]);
            assert!(pf.notices()[0].1.contains("127.0.0.5"), "os 5 endereços locais nomeados");
            assert!(pf.report().network_ok && pf.report().devices_present);
        }
    }

    /// **Estrutural (REFORÇO — a prova é a dos tipos):** no código de produção do crate (cada
    /// ficheiro de `src/` sem comentários, cortado no `mod tests`):
    /// - a guarda é consultada num só sítio (`medicao::medir`);
    /// - `decidir_rede` tem um só ponto de chamada;
    /// - o `PreflightReport` é construído por UMA só função (NV8): um literal, zero `all_clear`,
    ///   zero escritas em `.network_ok`;
    /// - o token do override só é criado pelo parser da CLI (`main.rs`) — NV10;
    /// - a política não toca em texto; `medir` só olha para a variante;
    /// - o aviso D2(a) está guardado pela decisão e tem o texto completo (NV7).
    #[test]
    fn estrutural_uma_consulta_um_decisor_um_relatorio_um_token() {
        let sem_comentarios = |t: &str| -> String {
            t.lines().map(|l| match l.find("//") { Some(i) => &l[..i], None => l }).collect::<Vec<_>>().join("\n")
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let (mut ficheiros, mut sondas, mut decisor, mut literais, mut all_clear, mut escritas, mut token_prod, mut token_main) =
            (0, 0, 0, 0, 0, 0, 0, 0);
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            ficheiros += 1;
            let texto = sem_comentarios(&std::fs::read_to_string(&p).unwrap());
            let prod = texto.split("mod tests {").next().unwrap();
            let c: String = prod.chars().filter(|c| !c.is_whitespace()).collect();
            sondas += c.matches(".check(").count() + c.matches("NetworkGuard>::check").count() + c.matches("NetworkGuard::check").count();
            decisor += c.matches("decidir_rede(").count();
            // Literais, não tipos de retorno: recua sobre o caminho (`led_daemon::`) e vê se vem de `->`.
            for (i, _) in c.match_indices("PreflightReport{") {
                let antes = c[..i].trim_end_matches(|ch: char| ch.is_alphanumeric() || ch == '_' || ch == ':');
                if !antes.ends_with("->") {
                    literais += 1;
                }
            }
            all_clear += c.matches("all_clear").count();
            escritas += c.matches(".network_ok=").count();
            token_prod += c.matches("da_linha_de_comando").count();
            if p.file_name().unwrap() == "main.rs" {
                token_main += c.matches("da_linha_de_comando").count();
            }
        }
        assert!(ficheiros >= 5, "premissa: leu o src ({ficheiros})");
        assert_eq!(sondas, 1, "a guarda só é consultada em medicao::medir");
        assert_eq!(decisor, 2, "decidir_rede: 1 definição + 1 chamada (o preflight)");
        assert_eq!((literais, all_clear, escritas), (1, 0, 0), "o PreflightReport só nasce em Preflight::novo (NV8)");
        assert_eq!((token_prod, token_main), (2, 1), "o token: 1 definição + 1 criação, no parser da CLI (NV10)");

        let fonte = sem_comentarios(include_str!("preflight.rs"));
        let politica = &fonte[fonte.find("pub mod politica_rede {").unwrap()..fonte.find("pub mod medicao {").unwrap()];
        for proibido in ["String", "&str", "format!", "interfaces", "reason", "error", "name(", "NetworkPolicyError"] {
            assert!(!politica.contains(proibido), "a política não pode ver texto nem o erro: `{proibido}`");
        }
        let medir = &fonte[fonte.find("pub fn medir(").unwrap()..fonte.find("impl Detalhe {").unwrap()];
        let m: String = medir.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!((m.matches("NetworkPolicyError::").count(), m.matches("{..})").count()), (3, 3), "medir só olha para a variante");
        let depois = &fonte[fonte.find("let decisao = decidir_rede(").expect("a chamada")..];
        let guarda = depois.find("if decisao.override_usado() {").expect("o aviso está guardado pela decisão");
        let ramo = depois[guarda..].trim_start_matches("if decisao.override_usado() {").trim_start();
        assert!(ramo.starts_with("eprintln!(") && ramo[..ramo.find(");").unwrap()].contains("AVISO: --assume-no-wifi USADO"),
                "o ramo do override escreve o aviso COMPLETO em stderr");
    }

    /// Filho do teste seguinte: com o rig REAL de 5 alvos e a sonda falhada + override, dois
    /// pré-voos (discovery ligado e desligado); e um com a sonda OK (sem aviso de USO).
    #[test]
    fn filho_dois_pre_voos_com_override() {
        let falhou = || GuardaNomeada(Err(E::ProbeFailed { probe: "sysfs /sys/class/net", error: "/sys/class/net not found".into() }), "WifiBlockGuard (WiFi-forbidden enforcement)");
        let ok = GuardaNomeada(Ok(()), "WifiBlockGuard (WiFi-forbidden enforcement)");
        let t = token(true);
        let presenca = PresencaMista { calados: &[], indisponiveis: &["192.168.2.163"] };
        let _ = preflight(Integrity::AssumedByOperator, Some(&rig_real(true)), &falhou(), &presenca, t.as_ref());
        let _ = preflight(Integrity::AssumedByOperator, Some(&rig_real(false)), &falhou(), &presenca, t.as_ref());
        let _ = preflight(Integrity::AssumedByOperator, Some(&rig_real(true)), &ok, &presenca, t.as_ref());
    }

    /// **D2(a) com stdout e stderr SEPARADOS e o rig de 5 alvos (R7.1 N6, R8.5 NV7):** dois
    /// pré-voos com override → o aviso COMPLETO duas vezes no **stderr**, nada no stdout (no
    /// binário real o stdout é o journal JSONL).
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
        let aviso = "AVISO: --assume-no-wifi USADO neste pré-voo — a sonda de rede falhou e o operador AFIRMA que não há WiFi ativo (NAO verificado).";
        assert_eq!(err.matches(aviso).count(), 2, "o aviso COMPLETO, um POR pré-voo, em stderr:\n{err}");
        assert_eq!(out.matches("--assume-no-wifi").count(), 0, "nada do aviso no stdout (o journal):\n{out}");
    }
}
