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
//! ## Sonda indisponível ≠ sonda reprovada
//!
//! Se a sonda não consegue medir, não se pode concluir nada — nem "há WiFi" nem "não há". A
//! política do repo já está fixada desde 2026-06-25: `ProbeUnavailable` **deixa prosseguir
//! com aviso**, em vez de bloquear ambientes sem hardware. O que este módulo garante é que a
//! diferença fica **escrita no journal**: "verificado" e "não foi possível verificar" nunca
//! aparecem com a mesma frase.

use crate::loader::Integrity;
use crate::output::OutputConfig;
use led_daemon::PreflightReport;
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

/// O resultado do pré-voo: o relatório que vai para a máquina de estados **e** o que dizer no
/// journal. As duas coisas juntas, porque um relatório sem a razão é um veredito sem prova.
pub struct Preflight {
    pub report: PreflightReport,
    pub notices: Vec<(&'static str, String)>,
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
) -> Preflight {
    let mut notices = Vec::new();
    let integrity_verified = integrity.satisfies_preflight();

    let Some(cfg) = output else {
        notices.push((
            "preflight_vacuous",
            "sem --output: network_ok e devices_present sao VACUOSOS, nao ha saida a proteger"
                .to_string(),
        ));
        return Preflight {
            report: PreflightReport { integrity_verified, network_ok: true, devices_present: true },
            notices,
        };
    };

    // ── Rede (ADR-0005: WiFi é proibido ao vivo) ─────────────────────────────
    //
    // Um alvo de **loopback** não atravessa interface nenhuma: o datagrama nasce e morre
    // dentro da máquina. O gate do ADR-0005 protege o *fio*, e aqui não há fio para o WiFi
    // corromper — é o mesmo raciocínio da vacuidade do GS2, aplicado a um caso concreto e
    // não à ausência de saída. **Não é um bypass**: um show apontado ao loopback não chega a
    // rig nenhum, e por isso não há nada que a regra pudesse salvar.
    if cfg.addr.ip().is_loopback() {
        notices.push((
            "network_local",
            format!("{} e loopback: nao atravessa interface, ADR-0005 nao se aplica", cfg.addr.ip()),
        ));
        notices.push((
            "devices_unverified",
            "alvo de loopback: NAO ha controladores a descobrir — prosseguindo com aviso".into(),
        ));
        return Preflight {
            report: PreflightReport { integrity_verified, network_ok: true, devices_present: true },
            notices,
        };
    }

    let network_ok = match guard.check() {
        Ok(()) => {
            notices.push(("network_checked", format!("{}: sem WiFi ativo", guard.name())));
            true
        }
        Err(NetworkPolicyError::WifiActive { interfaces }) => {
            notices.push((
                "network_refused",
                format!("WiFi ATIVO em {} — ADR-0005 proibe show ao vivo", interfaces.join(", ")),
            ));
            false
        }
        Err(NetworkPolicyError::ProbeUnavailable { reason }) => {
            notices.push((
                "network_unverified",
                format!("NAO foi possivel verificar a rede ({reason}) — prosseguindo com aviso"),
            ));
            true
        }
    };

    // ── Controladores (RT-003: palco escuro sem erro) ────────────────────────
    let devices_present = match presence.probe(cfg.addr.ip()) {
        Presence::AllPresent => {
            notices.push(("devices_checked", format!("{} respondeu", cfg.addr.ip())));
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

    Preflight {
        report: PreflightReport { integrity_verified, network_ok, devices_present },
        notices,
    }
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

    fn saida() -> OutputConfig {
        OutputConfig::parse("ddp://192.168.2.156", 720, 1).unwrap()
    }
    fn corre(g: Result<(), NetworkPolicyError>, p: Presence) -> Preflight {
        preflight(
            Integrity::AssumedByOperator,
            Some(&saida()),
            &GuardaFalsa(g),
            &SondaFalsa(p),
        )
    }
    fn tem(pf: &Preflight, n: &str) -> bool {
        pf.notices.iter().any(|(k, _)| *k == n)
    }

    /// **A vacuidade acabou.** Com saída, os dois campos vêm das sondas — e o teste prova-o
    /// pelo lado que interessa: um `false` de sonda tem de chegar ao relatório.
    #[test]
    fn com_saida_os_campos_deixam_de_ser_vacuosos() {
        let pf = corre(
            Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] }),
            Presence::Missing(vec!["192.168.2.156".into()]),
        );
        assert!(!pf.report.network_ok, "WiFi ativo tem de reprovar (ADR-0005)");
        assert!(!pf.report.devices_present, "controlador calado tem de reprovar (RT-003)");
        assert!(!tem(&pf, "preflight_vacuous"), "com saída, nada é vacuoso");
        assert!(tem(&pf, "network_refused") && tem(&pf, "devices_missing"));
    }

    #[test]
    fn tudo_verificado_aprova_e_diz_que_verificou() {
        let pf = corre(Ok(()), Presence::AllPresent);
        assert!(pf.report.network_ok && pf.report.devices_present);
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
        assert!(pf.report.network_ok && pf.report.devices_present, "prossegue");
        assert!(tem(&pf, "network_unverified") && tem(&pf, "devices_unverified"));
        assert!(!tem(&pf, "network_checked"), "NAO pode afirmar que verificou");
        assert!(!tem(&pf, "devices_checked"), "NAO pode afirmar que verificou");
    }

    /// Sem saída a vacuidade continua correta — e continua a ser **dita**.
    #[test]
    fn sem_saida_continua_vacuoso_e_declarado() {
        let pf = preflight(
            Integrity::AssumedByOperator,
            None,
            &PermissiveGuard,
            &SondaFalsa(Presence::AllPresent),
        );
        assert!(pf.report.network_ok && pf.report.devices_present);
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
            );
            assert!(!pf.report.integrity_verified, "sem afirmação do operador, reprova");
        }
    }

    /// Loopback dispensa o gate do WiFi **por não haver fio**, não por indulgência — e
    /// continua a recusar-se a afirmar que descobriu controladores.
    #[test]
    fn alvo_de_loopback_nao_invoca_o_gate_do_wifi_mas_tambem_nao_carimba_nada() {
        let cfg = OutputConfig::parse("ddp://127.0.0.1:9999", 4, 1).unwrap();
        let pf = preflight(
            Integrity::AssumedByOperator,
            Some(&cfg),
            // Uma guarda que reprovaria SEMPRE: se fosse consultada, o teste falhava.
            &GuardaFalsa(Err(NetworkPolicyError::WifiActive { interfaces: vec!["en0".into()] })),
            &SondaFalsa(Presence::AllPresent),
        );
        assert!(pf.report.network_ok, "loopback não atravessa interface");
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
        assert!(!pf.report.network_ok, "192.168.2.156 não é loopback: o ADR-0005 aplica-se");
        assert!(tem(&pf, "network_refused"));
    }

    /// A sonda real **recusa-se a inventar um rig** num alvo de loopback.
    #[test]
    fn a_sonda_real_nao_finge_descobrir_um_loopback() {
        let r = ArtPollPresence.probe("127.0.0.1".parse().unwrap());
        assert!(matches!(r, Presence::Unavailable(_)), "loopback não é um controlador: {r:?}");
    }
}
