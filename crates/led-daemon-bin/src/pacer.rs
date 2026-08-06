//! Cadência do laço principal — **injetável de propósito**.
//!
//! O laço precisa de dormir entre ticks (o critério "sem busy-loop"), e um teste que
//! verificasse isso com relógio de parede seria instável sob carga — exatamente a classe que
//! o TD-003 deste repo fechou trocando 8 `thread::sleep` por barreiras causais.
//!
//! Com o [`Pacer`] injetado, "dormiu a cada iteração" deixa de ser uma medição de tempo e
//! passa a ser uma **asserção determinística**: o pacer de teste conta as esperas e avança um
//! relógio virtual. Nenhum teste deste crate dorme.

/// Fonte de tempo e de espera do laço principal.
pub trait Pacer {
    /// Milissegundos desde o arranque do daemon. **Monotónico.**
    fn now_ms(&self) -> u64;
    /// Espera até `deadline_ms`. Retorna já se o prazo passou — nunca dorme para trás.
    fn sleep_until(&mut self, deadline_ms: u64);
}

/// O pacer real: `Instant` monotónico + `thread::sleep`.
pub struct SystemPacer {
    start: std::time::Instant,
}

impl Default for SystemPacer {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPacer {
    pub fn new() -> Self {
        Self { start: std::time::Instant::now() }
    }
}

impl Pacer for SystemPacer {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn sleep_until(&mut self, deadline_ms: u64) {
        let now = self.now_ms();
        if deadline_ms > now {
            std::thread::sleep(std::time::Duration::from_millis(deadline_ms - now));
        }
        // Prazo já passado: não dorme. Quem decide o que fazer com o atraso é o laço
        // (`run.rs`), que salta os ticks perdidos em vez de os acumular.
    }
}

#[cfg(test)]
pub(crate) mod test_pacer {
    use super::Pacer;

    /// Pacer de teste: relógio virtual, e regista **cada** espera pedida.
    pub struct VirtualPacer {
        pub now: u64,
        pub sleeps: Vec<u64>,
        /// Atraso injetado por espera — para exercitar o caminho de "tick superado".
        pub lag_ms: u64,
    }

    impl VirtualPacer {
        pub fn new() -> Self {
            Self { now: 0, sleeps: Vec::new(), lag_ms: 0 }
        }
        pub fn with_lag(lag_ms: u64) -> Self {
            Self { now: 0, sleeps: Vec::new(), lag_ms }
        }
    }

    impl Pacer for VirtualPacer {
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn sleep_until(&mut self, deadline_ms: u64) {
            self.sleeps.push(deadline_ms);
            // Avança o relógio para o prazo (mais o atraso injetado), nunca para trás.
            self.now = self.now.max(deadline_ms) + self.lag_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_pacer::VirtualPacer;
    use super::*;

    #[test]
    fn pacer_virtual_avanca_e_regista() {
        let mut p = VirtualPacer::new();
        assert_eq!(p.now_ms(), 0);
        p.sleep_until(20);
        assert_eq!(p.now_ms(), 20);
        p.sleep_until(40);
        assert_eq!(p.sleeps, vec![20, 40]);
    }

    #[test]
    fn pacer_nunca_recua() {
        let mut p = VirtualPacer::new();
        p.sleep_until(100);
        p.sleep_until(50); // prazo no passado
        assert_eq!(p.now_ms(), 100, "o relógio não pode recuar");
    }

    #[test]
    fn pacer_do_sistema_e_monotonico() {
        let p = SystemPacer::new();
        let a = p.now_ms();
        let b = p.now_ms();
        assert!(b >= a);
    }
}
