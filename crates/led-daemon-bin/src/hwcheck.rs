//! GS4.5 — a **lógica de medição e veredito** da validação de hardware.
//!
//! ## Porque isto existe como código, e não como um runbook manual
//!
//! O runbook cobre o que um humano observa: a fita acende, a cor é a certa. Mas **latência,
//! jitter, gap de heartbeat e tempo de recuperação não são observáveis a olho** — e um número
//! escrito à mão num documento é uma afirmação, não uma medição. Este módulo é o que
//! transforma as sete últimas linhas do GS4.5 em evidência reproduzível.
//!
//! ## A regra que governa tudo aqui: **não medido nunca é aprovado**
//!
//! [`Veredito`] tem três estados, não dois. Um ambiente sem hardware produz `NaoMedido` — e o
//! relatório **reprova**, com código de saída próprio. Isto não é pessimismo: é a diferença
//! entre "o cabo está bom" e "não havia cabo". Colapsar as duas coisas seria o falso-verde do
//! KB-012 na sua forma mais cara, porque só se descobriria no palco.
//!
//! ## Nada aqui fala com a rede
//!
//! I/O vive no binário `lumyx-hwcheck`. Este módulo recebe **amostras** e decide — o que o
//! torna testável sem rig, que é precisamente o que se pode fazer enquanto o rig não existe.

use std::fmt;

/// Amostras de tempo, em milissegundos.
#[derive(Clone, Debug, Default)]
pub struct Amostras(Vec<f64>);

impl Amostras {
    pub fn new(v: Vec<f64>) -> Self {
        Self(v)
    }
    pub fn push(&mut self, ms: f64) {
        self.0.push(ms);
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn media(&self) -> Option<f64> {
        if self.0.is_empty() {
            return None;
        }
        Some(self.0.iter().sum::<f64>() / self.0.len() as f64)
    }

    pub fn max(&self) -> Option<f64> {
        self.0.iter().copied().fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x))))
    }

    /// Percentil por ordenação (nearest-rank). Poucas amostras é o caso normal aqui, e um
    /// histograma seria precisão fingida sobre 20 medições.
    pub fn percentil(&self, p: f64) -> Option<f64> {
        if self.0.is_empty() {
            return None;
        }
        let mut v = self.0.clone();
        v.sort_by(|a, b| a.partial_cmp(b).expect("tempos não são NaN"));
        let idx = ((p / 100.0 * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
        Some(v[idx])
    }

    /// **Jitter como desvio-padrão** — deliberadamente a mesma definição que o `ping` do macOS
    /// imprime em `stddev`, para que o número seja **comparável** aos 31 ms medidos na bancada
    /// WiFi de 2026-07-20 que confirmaram o ADR-0005. Uma definição diferente daria um número
    /// que não se pode pôr ao lado do histórico.
    pub fn jitter(&self) -> Option<f64> {
        let m = self.media()?;
        if self.0.len() < 2 {
            return None;
        }
        let var = self.0.iter().map(|x| (x - m).powi(2)).sum::<f64>() / self.0.len() as f64;
        Some(var.sqrt())
    }

    /// O maior intervalo entre amostras consecutivas — é isto que o invariante do heartbeat
    /// mede, e não a média (uma média boa esconde um buraco de 3 s).
    pub fn maior_intervalo(&self) -> Option<f64> {
        if self.0.len() < 2 {
            return None;
        }
        Some(self.0.windows(2).map(|w| w[1] - w[0]).fold(f64::MIN, f64::max))
    }
}

/// O veredito de uma etapa. **Três estados, não dois.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Veredito {
    /// Medido, e dentro do critério.
    Passa(String),
    /// Medido, e **fora** do critério. É um facto sobre o rig.
    Reprova(String),
    /// **Não foi medido.** Não diz nada sobre o rig — e por isso não pode aprovar nada.
    NaoMedido(String),
}

impl Veredito {
    pub fn simbolo(&self) -> &'static str {
        match self {
            Veredito::Passa(_) => "PASS",
            Veredito::Reprova(_) => "FALHA",
            Veredito::NaoMedido(_) => "NAO MEDIDO",
        }
    }
    pub fn detalhe(&self) -> &str {
        match self {
            Veredito::Passa(d) | Veredito::Reprova(d) | Veredito::NaoMedido(d) => d,
        }
    }
    /// **Só `Passa` aprova.** Existe como função para que nenhum sítio possa esquecer-se de
    /// tratar o terceiro caso — é a guarda única do módulo.
    pub fn aprova(&self) -> bool {
        matches!(self, Veredito::Passa(_))
    }
}

/// Uma etapa do GS4.5, com o que o runbook exige: objetivo, critério e resultado.
#[derive(Clone, Debug)]
pub struct Etapa {
    pub nome: &'static str,
    pub objetivo: &'static str,
    pub criterio: &'static str,
    pub veredito: Veredito,
}

/// O relatório completo.
#[derive(Clone, Debug, Default)]
pub struct Relatorio {
    pub alvo: String,
    pub preset: String,
    pub etapas: Vec<Etapa>,
}

/// Como o processo termina. Códigos distintos porque **"reprovou" e "não deu para medir" são
/// coisas diferentes**, e um script que as confunda toma a decisão errada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saida {
    /// Todas as etapas medidas e aprovadas.
    TudoAprovado = 0,
    /// Pelo menos uma etapa **medida** reprovou.
    Reprovou = 1,
    /// Pelo menos uma etapa **não foi medida** — nenhuma conclusão sobre o rig é possível.
    Incompleto = 2,
}

impl Relatorio {
    pub fn add(
        &mut self,
        nome: &'static str,
        objetivo: &'static str,
        criterio: &'static str,
        veredito: Veredito,
    ) {
        self.etapas.push(Etapa { nome, objetivo, criterio, veredito });
    }

    /// **A reprovação vence, e a ausência de medição vence a aprovação.**
    pub fn saida(&self) -> Saida {
        if self.etapas.iter().any(|e| matches!(e.veredito, Veredito::Reprova(_))) {
            return Saida::Reprovou;
        }
        if self.etapas.iter().any(|e| matches!(e.veredito, Veredito::NaoMedido(_))) {
            return Saida::Incompleto;
        }
        if self.etapas.is_empty() {
            // Zero etapas não é sucesso: é um harness que não correu (KB-012, o gate que
            // aprova sem exercitar nada).
            return Saida::Incompleto;
        }
        Saida::TudoAprovado
    }

    /// O artefacto de evidência, em Markdown, no formato de `docs/certification/`.
    pub fn markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# GS4.5 — validação de hardware (gerado por `lumyx-hwcheck`)\n\n");
        s.push_str(&format!("- **Alvo:** `{}`\n- **Preset:** `{}`\n", self.alvo, self.preset));
        s.push_str(&format!("- **Veredito global:** {:?}\n\n", self.saida()));
        s.push_str("| Etapa | Objetivo | Critério | Resultado | Evidência |\n");
        s.push_str("|---|---|---|---|---|\n");
        for e in &self.etapas {
            s.push_str(&format!(
                "| {} | {} | {} | **{}** | {} |\n",
                e.nome,
                e.objetivo,
                e.criterio,
                e.veredito.simbolo(),
                e.veredito.detalhe().replace('|', "\\|")
            ));
        }
        s.push_str(
            "\n> **`NAO MEDIDO` não é aprovação.** Uma etapa sem medição não diz nada sobre o \
             rig, e o processo sai com código 2 precisamente para que nenhum script a leia \
             como sucesso.\n",
        );
        s
    }
}

impl fmt::Display for Relatorio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in &self.etapas {
            writeln!(f, "[{:>10}] {:<22} {}", e.veredito.simbolo(), e.nome, e.veredito.detalhe())?;
        }
        write!(f, "\nveredito: {:?}", self.saida())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[f64]) -> Amostras {
        Amostras::new(v.to_vec())
    }

    #[test]
    fn estatisticas_sobre_amostras_conhecidas() {
        let s = a(&[10.0, 20.0, 30.0, 40.0]);
        assert_eq!(s.media(), Some(25.0));
        assert_eq!(s.max(), Some(40.0));
        assert_eq!(s.percentil(50.0), Some(20.0), "nearest-rank em 4 amostras");
        assert_eq!(s.percentil(100.0), Some(40.0));
        assert_eq!(s.maior_intervalo(), Some(10.0));
    }

    /// **O jitter é o desvio-padrão**, para ser comparável ao `stddev` do `ping`. Uma série
    /// constante tem jitter zero — se não tivesse, o número não significaria o que se diz.
    #[test]
    fn jitter_e_desvio_padrao_e_uma_serie_constante_tem_zero() {
        assert_eq!(a(&[5.0, 5.0, 5.0, 5.0]).jitter(), Some(0.0));
        let j = a(&[1.0, 3.0]).jitter().unwrap();
        assert!((j - 1.0).abs() < 1e-9, "σ de [1,3] é 1.0, veio {j}");
        assert_eq!(a(&[7.0]).jitter(), None, "uma amostra não tem dispersão");
        assert_eq!(a(&[]).media(), None, "zero amostras não tem média — e não é zero");
    }

    /// **Zero amostras nunca vira zero milissegundos.** Era esta a forma mais fácil de o
    /// harness mentir: reportar `0.0 ms` de latência num rig que nunca respondeu.
    #[test]
    fn ausencia_de_amostras_e_none_nunca_zero() {
        let vazio = a(&[]);
        assert!(vazio.is_empty());
        for v in [vazio.media(), vazio.max(), vazio.jitter(), vazio.percentil(99.0)] {
            assert_eq!(v, None);
        }
    }

    #[test]
    fn so_passa_aprova() {
        assert!(Veredito::Passa("x".into()).aprova());
        assert!(!Veredito::Reprova("x".into()).aprova());
        assert!(!Veredito::NaoMedido("x".into()).aprova(), "não medido NUNCA aprova");
    }

    /// **A hierarquia dos códigos de saída**, que é o que um script de CI lê.
    #[test]
    fn reprovar_vence_nao_medir_que_vence_aprovar() {
        let mut r = Relatorio::default();
        assert_eq!(r.saida(), Saida::Incompleto, "harness sem etapas não é sucesso");

        r.add("a", "o", "c", Veredito::Passa("1 ms".into()));
        assert_eq!(r.saida(), Saida::TudoAprovado);

        r.add("b", "o", "c", Veredito::NaoMedido("sem rig".into()));
        assert_eq!(r.saida(), Saida::Incompleto, "uma etapa não medida contamina o todo");

        r.add("c", "o", "c", Veredito::Reprova("gap 3000 ms".into()));
        assert_eq!(r.saida(), Saida::Reprovou, "uma reprovação medida vence tudo");
    }

    /// **Controle negativo do relatório**: um relatório só com `NaoMedido` não pode produzir um
    /// documento que se leia como aprovação.
    #[test]
    fn um_relatorio_sem_medicoes_diz_que_nao_mediu() {
        let mut r = Relatorio { alvo: "192.168.2.156".into(), preset: "esp32-poe".into(), ..Default::default() };
        for nome in ["ping", "ddp", "artnet", "sacn", "heartbeat", "recovery"] {
            r.add(nome, "o", "c", Veredito::NaoMedido("rig ausente".into()));
        }
        let md = r.markdown();
        assert_eq!(r.saida(), Saida::Incompleto);
        // Contar a **célula** da tabela (`**...**`), não o texto do rodapé — que também
        // menciona `NAO MEDIDO` para explicar a regra. Contar as duas coisas juntas foi o
        // meu primeiro erro aqui, e o teste apanhou-o.
        assert!(!md.contains("**PASS**"), "nenhuma linha pode dizer PASS:\n{md}");
        assert_eq!(md.matches("**NAO MEDIDO**").count(), 6, "uma célula por etapa");
        assert!(md.contains("não é aprovação"));
    }

    #[test]
    fn o_maior_intervalo_e_o_que_o_heartbeat_mede() {
        // Média boa (250 ms) mas com um buraco de 3 s no meio: a média aprovaria, o máximo não.
        let s = a(&[0.0, 100.0, 200.0, 3_200.0, 3_300.0]);
        assert!(s.media().unwrap() < 1_500.0);
        assert_eq!(s.maior_intervalo(), Some(3_000.0), "é o buraco que interessa, não a média");
    }
}
