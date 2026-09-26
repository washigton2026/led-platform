//! ADR-0026 §7–9 — o modelo de verdade que o console transporta.
//!
//! # A regra
//!
//! **Observabilidade não é evidência física.** Um contador local a crescer é o dado mais
//! tentador de mostrar como *"está a funcionar"*, e é o mais local de todos.

use std::time::Duration;

/// Os elos da cadeia, **em ordem**. Cada um só pode ser afirmado com a sua própria evidência.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Elo {
    /// O `sendto` local teve sucesso. **Observabilidade**, não prova de entrega.
    SoftwareSent,
    /// O datagrama chegou à rede do nó. Exige instrumentação que **não existe**.
    NetworkDelivered,
    /// O controlador recebeu — `live:true` do WLED. Exige hardware.
    ControllerReceived,
    /// O controlador confirmou o protocolo — `lm` do WLED. Exige hardware.
    ControllerAcknowledged,
    /// O pixel acendeu com a cor certa. **Observação humana**, nunca automática.
    LedVerified,
}

impl Elo {
    /// Todos os elos, do mais fraco ao mais forte.
    pub const ALL: [Elo; 5] = [
        Elo::SoftwareSent,
        Elo::NetworkDelivered,
        Elo::ControllerReceived,
        Elo::ControllerAcknowledged,
        Elo::LedVerified,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Elo::SoftwareSent => "software_sent",
            Elo::NetworkDelivered => "network_delivered",
            Elo::ControllerReceived => "controller_received",
            Elo::ControllerAcknowledged => "controller_acknowledged",
            Elo::LedVerified => "led_verified",
        }
    }
}

/// Os nove estados que a UI pode apresentar. **Nenhum é calculado aqui** — todos são
/// transportados de uma fonte a montante.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstadoUi {
    /// Medido e dentro do critério.
    Pass,
    /// Medido e fora do critério. É facto sobre o rig.
    Fail,
    /// **Não foi medido.** Não diz nada — e por isso nunca aprova.
    NotMeasured,
    /// Uma guarda impediu (pré-voo, profile inválido, `NeverStarted`).
    Blocked,
    /// `State::Playing`.
    Running,
    /// O console não conseguiu falar com o daemon. **Estado, não erro.**
    Offline,
    /// Continua, mas com erros no fio (`HealthStatus::Warning`, `errors > 0`).
    Degraded,
    /// Ausência de dado. `discovery: null` é null, nunca `[]`.
    Unknown,
    /// Alvo de loopback ou `SimulatorDevice`. Marca permanente no ecrã.
    Simulation,
}

impl EstadoUi {
    /// Todos os estados. **Existe para o contrato gerado** (ADR-0027) poder enumerá-los sem
    /// macro nem `serde`.
    ///
    /// Esta lista é escrita à mão, e por isso pode ficar para trás do `enum`. Não fica: o
    /// gate do contrato lê o **texto** de `enum EstadoUi` e reprova se `ALL` perder alguma
    /// variante. Sem esse controlo negativo, um estado novo esquecido aqui produziria um
    /// TypeScript sem ele — e o ficheiro versionado, gerado pelo mesmo caminho, concordaria.
    /// Verde, e errado (KB-012).
    pub const ALL: [EstadoUi; 9] = [
        EstadoUi::Pass,
        EstadoUi::Fail,
        EstadoUi::NotMeasured,
        EstadoUi::Blocked,
        EstadoUi::Running,
        EstadoUi::Offline,
        EstadoUi::Degraded,
        EstadoUi::Unknown,
        EstadoUi::Simulation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EstadoUi::Pass => "PASS",
            EstadoUi::Fail => "FAIL",
            EstadoUi::NotMeasured => "NOT_MEASURED",
            EstadoUi::Blocked => "BLOCKED",
            EstadoUi::Running => "RUNNING",
            EstadoUi::Offline => "OFFLINE",
            EstadoUi::Degraded => "DEGRADED",
            EstadoUi::Unknown => "UNKNOWN",
            EstadoUi::Simulation => "SIMULATION",
        }
    }

    /// **Só `Pass` aprova.** Existe como função para que nenhum sítio possa esquecer-se dos
    /// outros oito — a mesma guarda única que o `hwcheck::Veredito` já usa.
    pub fn aprova(self) -> bool {
        matches!(self, EstadoUi::Pass)
    }
}

/// Até onde a evidência chega. **Não é um booleano, de propósito.**
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidencia {
    confirmados: Vec<Elo>,
}

impl Evidencia {
    pub fn nova() -> Self {
        Self::default()
    }

    /// Regista que **este** elo tem evidência própria.
    ///
    /// Não implica nenhum outro: confirmar `SoftwareSent` não confirma `NetworkDelivered`, e
    /// confirmar `ControllerReceived` não confirma `LedVerified`. A implicação seria
    /// exatamente o colapso que o ADR-0026 §8 proíbe.
    pub fn confirma(&mut self, elo: Elo) -> &mut Self {
        if !self.confirmados.contains(&elo) {
            self.confirmados.push(elo);
        }
        self
    }

    pub fn tem(&self, elo: Elo) -> bool {
        self.confirmados.contains(&elo)
    }

    /// O estado de **um** elo: `Pass` se confirmado, `NotMeasured` caso contrário.
    ///
    /// Nunca `Fail` — não ter medido não é ter falhado.
    pub fn estado(&self, elo: Elo) -> EstadoUi {
        if self.tem(elo) {
            EstadoUi::Pass
        } else {
            EstadoUi::NotMeasured
        }
    }

    /// O elo mais forte confirmado, se houver. É isto que a UI mostra como profundidade.
    pub fn profundidade(&self) -> Option<Elo> {
        self.confirmados.iter().copied().max()
    }
}

/// Um instantâneo com **idade**. É o que impede dado velho de se apresentar como atual.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instantaneo<T> {
    dado: Option<T>,
    idade: Duration,
    estado: EstadoUi,
}

impl<T> Instantaneo<T> {
    /// Dado fresco, vindo do daemon agora.
    pub fn fresco(dado: T, estado: EstadoUi) -> Self {
        Self { dado: Some(dado), idade: Duration::ZERO, estado }
    }

    /// **O último dado conhecido, marcado como velho.**
    ///
    /// O estado passa a `Offline` — nunca se devolvem zeros, e nunca se apresenta o valor
    /// antigo como se fosse de agora.
    pub fn velho(dado: T, idade: Duration) -> Self {
        Self { dado: Some(dado), idade, estado: EstadoUi::Offline }
    }

    /// Nunca houve dado. **`Unknown`, não zero.**
    pub fn nunca_houve() -> Self {
        Self { dado: None, idade: Duration::ZERO, estado: EstadoUi::Unknown }
    }

    pub fn dado(&self) -> Option<&T> {
        self.dado.as_ref()
    }
    pub fn estado(&self) -> EstadoUi {
        self.estado
    }
    /// Idade do dado — **`None` quando não há dado**.
    ///
    /// Devolver `0` para um instantâneo que nunca existiu seria indistinguível de um dado
    /// acabado de chegar. É exatamente o zero artificial que o ADR-0026 §7 proíbe, e por
    /// isso a ausência tem de ser representável no tipo.
    pub fn stale_ms(&self) -> Option<u64> {
        self.dado.as_ref().map(|_| self.idade.as_millis() as u64)
    }
    /// `true` se o consumidor **não** pode ler isto como o estado corrente.
    pub fn e_velho(&self) -> bool {
        self.estado == EstadoUi::Offline || self.estado == EstadoUi::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **O teste central do ADR-0026 §8.** Confirmar o primeiro elo não confirma nenhum outro.
    #[test]
    fn software_sent_nao_implica_controller_received() {
        let mut e = Evidencia::nova();
        e.confirma(Elo::SoftwareSent);

        assert_eq!(e.estado(Elo::SoftwareSent), EstadoUi::Pass);
        assert_eq!(e.estado(Elo::NetworkDelivered), EstadoUi::NotMeasured);
        assert_eq!(e.estado(Elo::ControllerReceived), EstadoUi::NotMeasured);
        assert_eq!(e.estado(Elo::ControllerAcknowledged), EstadoUi::NotMeasured);
        assert_eq!(e.estado(Elo::LedVerified), EstadoUi::NotMeasured);
    }

    /// **O elo mais forte também não desce a cadeia.** Aceitação do controlador não é pixel.
    #[test]
    fn controller_received_nao_implica_led_verified() {
        let mut e = Evidencia::nova();
        e.confirma(Elo::ControllerReceived);
        assert_eq!(e.estado(Elo::ControllerReceived), EstadoUi::Pass);
        assert_eq!(e.estado(Elo::LedVerified), EstadoUi::NotMeasured, "o olho ainda nao viu");
        assert_eq!(
            e.estado(Elo::SoftwareSent),
            EstadoUi::NotMeasured,
            "e nem sequer implica para tras: cada elo carrega a SUA evidencia"
        );
    }

    /// **Não medido nunca é aprovado** — a guarda única.
    #[test]
    fn so_pass_aprova() {
        assert!(EstadoUi::Pass.aprova());
        for outro in [
            EstadoUi::Fail,
            EstadoUi::NotMeasured,
            EstadoUi::Blocked,
            EstadoUi::Running,
            EstadoUi::Offline,
            EstadoUi::Degraded,
            EstadoUi::Unknown,
            EstadoUi::Simulation,
        ] {
            assert!(!outro.aprova(), "{} nao pode aprovar", outro.as_str());
        }
    }

    /// **Uma cadeia vazia não aprova nada.** É o controle negativo do teste de cima.
    #[test]
    fn sem_evidencia_nenhum_elo_passa() {
        let e = Evidencia::nova();
        assert_eq!(e.profundidade(), None, "profundidade nula, nao SoftwareSent");
        for elo in Elo::ALL {
            assert_eq!(e.estado(elo), EstadoUi::NotMeasured, "{}", elo.as_str());
        }
    }

    /// **Daemon offline não produz zeros artificiais**, e o dado velho diz a sua idade.
    #[test]
    fn offline_preserva_o_ultimo_conhecido_com_idade() {
        let velho = Instantaneo::velho(4210u64, Duration::from_millis(122_000));
        assert_eq!(velho.dado(), Some(&4210), "o ultimo conhecido e preservado");
        assert_eq!(velho.estado(), EstadoUi::Offline, "mas NAO como estado corrente");
        assert_eq!(velho.stale_ms(), Some(122_000), "e a idade viaja com ele");
        assert!(velho.e_velho());

        let nunca: Instantaneo<u64> = Instantaneo::nunca_houve();
        assert_eq!(nunca.dado(), None, "sem dado e None — NUNCA zero");
        assert_eq!(nunca.estado(), EstadoUi::Unknown);

        let fresco = Instantaneo::fresco(7u64, EstadoUi::Running);
        assert_eq!(fresco.stale_ms(), Some(0));
        assert_eq!(
            nunca.stale_ms(),
            None,
            "idade de um dado que nunca existiu nao pode ser 0 — seria igual a `acabou de chegar`"
        );
        assert!(!fresco.e_velho());
    }

    /// Os nove estados têm nomes distintos — colidir dois apagaria uma distinção.
    #[test]
    fn os_nove_estados_sao_distintos() {
        let todos = [
            EstadoUi::Pass,
            EstadoUi::Fail,
            EstadoUi::NotMeasured,
            EstadoUi::Blocked,
            EstadoUi::Running,
            EstadoUi::Offline,
            EstadoUi::Degraded,
            EstadoUi::Unknown,
            EstadoUi::Simulation,
        ];
        let mut nomes: Vec<&str> = todos.iter().map(|e| e.as_str()).collect();
        nomes.sort_unstable();
        nomes.dedup();
        assert_eq!(nomes.len(), 9, "nove estados, nove nomes");
    }
}
