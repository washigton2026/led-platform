//! GS4.3 — **inventário**: que nós se espera, e quais responderam.
//!
//! ## Descoberta por protocolo — o que existe e o que não existe
//!
//! - **Art-Net** tem descoberta a sério: ArtPoll → ArtPollReply. É a que se usa.
//! - **sACN** não define descoberta de nós; a E1.31 descobre *universos* por multicast, não
//!   controladores.
//! - **DDP não tem descoberta nenhuma.** Não é uma lacuna do LUMYX: a especificação não a
//!   define. Um alvo DDP descobre-se pelo Art-Net do mesmo nó (o WLED responde a ArtPoll
//!   independentemente do protocolo de saída — foi assim que o `--require-all` passou a
//!   funcionar em 2026-07-12) ou pelo `/json/info` por HTTP, que é uma dependência de cliente
//!   HTTP que este crate não tem.
//!
//! Inventar um "DDP discovery" seria escrever um protocolo que nenhum controlador fala.

use crate::preflight::{DevicePresence, Presence};
use std::net::IpAddr;

/// Um nó que se espera encontrar, e o que dele se sabe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub addr: IpAddr,
    /// Nome do preset do `HardwareProfile`, quando há um.
    pub profile: Option<String>,
    pub presence: Presence,
}

impl Node {
    /// Responde ao que interessa antes de um show: **posso contar com este nó?**
    pub fn is_confirmed(&self) -> bool {
        self.presence == Presence::AllPresent
    }
}

/// O inventário do rig. É deliberadamente **uma lista de factos**, não um veredito: quem
/// decide se o show pode começar é o pré-voo, com a política num sítio só.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub nodes: Vec<Node>,
}

impl Inventory {
    /// Sonda cada nó esperado. A sonda é **injetada** — a mesma disciplina do pré-voo.
    pub fn probe(expected: &[(IpAddr, Option<String>)], sonda: &dyn DevicePresence) -> Self {
        let nodes = expected
            .iter()
            .map(|(addr, profile)| Node {
                addr: *addr,
                profile: profile.clone(),
                presence: sonda.probe(*addr),
            })
            .collect();
        Self { nodes }
    }

    /// Os que responderam.
    pub fn confirmed(&self) -> Vec<&Node> {
        self.nodes.iter().filter(|n| n.is_confirmed()).collect()
    }

    /// Os que ficaram calados. **É esta a lista que apaga um palco** se ninguém a ler.
    pub fn silent(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| matches!(n.presence, Presence::Missing(_)))
            .collect()
    }

    /// Os que não foi possível sondar — **nem confirmados nem desmentidos**. Mantê-los numa
    /// terceira lista é o que impede "não sei" de ser arredondado para "sim".
    pub fn unverified(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| matches!(n.presence, Presence::Unavailable(_)))
            .collect()
    }

    /// Linha por nó, para o journal.
    pub fn to_lines(&self) -> Vec<String> {
        self.nodes
            .iter()
            .map(|n| {
                let estado = match &n.presence {
                    Presence::AllPresent => "presente".to_string(),
                    Presence::Missing(_) => "AUSENTE".to_string(),
                    Presence::Unavailable(r) => format!("nao sondado ({r})"),
                };
                let perfil = n.profile.as_deref().unwrap_or("sem profile");
                format!("{} · {perfil} · {estado}", n.addr)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sonda(Vec<(IpAddr, Presence)>);
    impl DevicePresence for Sonda {
        fn probe(&self, t: IpAddr) -> Presence {
            self.0
                .iter()
                .find(|(a, _)| *a == t)
                .map(|(_, p)| p.clone())
                .unwrap_or(Presence::Unavailable("desconhecido".into()))
        }
        fn name(&self) -> &'static str {
            "teste"
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// As três categorias são **três**, e nunca duas. "Não sei" não é "sim".
    #[test]
    fn presente_ausente_e_nao_sondado_sao_categorias_distintas() {
        let sonda = Sonda(vec![
            (ip("192.168.2.156"), Presence::AllPresent),
            (ip("192.168.2.157"), Presence::Missing(vec!["192.168.2.157".into()])),
            (ip("192.168.2.158"), Presence::Unavailable("sem rota".into())),
        ]);
        let inv = Inventory::probe(
            &[
                (ip("192.168.2.156"), Some("esp32-poe-wled-ddp".into())),
                (ip("192.168.2.157"), None),
                (ip("192.168.2.158"), None),
            ],
            &sonda,
        );
        assert_eq!(inv.confirmed().len(), 1);
        assert_eq!(inv.silent().len(), 1);
        assert_eq!(inv.unverified().len(), 1);
        assert!(!inv.unverified()[0].is_confirmed(), "não sondado NÃO conta como confirmado");
    }

    #[test]
    fn as_linhas_do_journal_nomeiam_o_no_o_perfil_e_o_estado() {
        let inv = Inventory::probe(
            &[(ip("192.168.2.156"), Some("esp32-poe-wled-ddp".into()))],
            &Sonda(vec![(ip("192.168.2.156"), Presence::Missing(vec!["x".into()]))]),
        );
        let l = &inv.to_lines()[0];
        assert!(l.contains("192.168.2.156") && l.contains("esp32-poe-wled-ddp") && l.contains("AUSENTE"), "{l}");
    }
}
