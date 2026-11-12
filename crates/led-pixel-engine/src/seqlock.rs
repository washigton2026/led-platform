//! Latch de sequência com dois slots sobre atómicos — sem `unsafe`, sem memória partilhada entre instâncias (TD-030).
//!
//! # Porque existe
//! O `AudioShare` publicava os escalares num `ArcSwap` (arc-swap 1.9.1). O TD-030 mediu `scalars()` a devolver um
//! snapshot que só OUTRA instância tinha publicado, e a reprodução mínima só com o arc-swap 1.9.1 reproduz (ordem F,
//! 1a). Este latch não tem nada global nem reciclado: cada instância só lê os seus próprios atómicos.
//!
//! # Um único escritor, por estrutura
//! [`novo`] é o ÚNICO construtor e devolve exatamente um [`Escritor`]. O `Escritor` não é `Clone` nem `Copy`, e
//! escrever exige `&mut Escritor`. O [`Leitor`] não tem nenhum método de escrita.
//!
//! # Protocolo (orderings) — um incremento por publicação
//! `seq` é a versão da última publicação COMPLETA e só o escritor a altera. Os leitores leem o slot `seq & 1`. O
//! escritor escreve o OUTRO slot (`(seq + 1) & 1`) e só depois avança `seq`. Assim, a paridade alterna a cada
//! publicação e os dois slots são lidos mesmo sem concorrência; um escritor parado a meio não obriga ninguém a
//! repetir.
//!
//! Escritor (`s` = `seq` à entrada):
//! 1. `fence(Release)`, depois escreve o slot `(s + 1) & 1` (`Relaxed`). A cerca ordena o `seq = s` da publicação
//!    anterior ANTES destas escritas: quem ainda lê esse slot com `s - 1` vê `seq ≥ s` e repete.
//! 2. `seq.store(s + 1, Release)`: o slot novo fica completo para quem ler `s + 1` com `Acquire`.
//!
//! Leitor: `s1 = seq.load(Acquire)`; lê o slot `s1 & 1` (`Relaxed`); `fence(Acquire)`; `s2 = seq.load(Relaxed)`;
//! `s1 == s2` → snapshot válido.
//!
//! O `fence(Release)` do escritor antes de escrever um slot emparelha com o `fence(Acquire)` do leitor depois de o
//! ler: se o leitor viu um valor novo nesse slot, também vê `seq` já avançada e repete. Referências: H.-J. Boehm,
//! «Can seqlocks get along with programming language memory models?» (MSPC 2012); Linux `seqcount_latch`.

#[cfg(loom)]
use loom::sync::atomic::{fence, AtomicU64, Ordering};
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{fence, AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::Arc;

/// Estado partilhado: a versão e os dois slots de `W` palavras.
struct Latch<const W: usize> {
    seq: AtomicU64,
    slots: [[AtomicU64; W]; 2],
}

/// O lado de leitura. Pode haver vários (é `Clone`); nenhum escreve.
#[derive(Clone)]
pub(crate) struct Leitor<const W: usize> {
    latch: Arc<Latch<W>>,
}

/// Todas as tentativas de [`Leitor::ler`] apanharam o escritor a avançar. `tentativas` é CONTADA dentro do ciclo,
/// nunca copiada de `k` (TD-030, ronda 2: o oráculo loom O3 e o estrutural O2-bis dependem disso).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Esgotado {
    pub(crate) tentativas: u32,
}

/// O ÚNICO lado de escrita. Não é `Clone` nem `Copy`.
pub(crate) struct Escritor<const W: usize> {
    latch: Arc<Latch<W>>,
}

/// Cria o latch com `inicial` nos dois slots. Devolve um leitor e o único escritor.
pub(crate) fn novo<const W: usize>(inicial: [u64; W]) -> (Leitor<W>, Escritor<W>) {
    let slot = || inicial.map(AtomicU64::new);
    let latch = Arc::new(Latch { seq: AtomicU64::new(0), slots: [slot(), slot()] });
    (Leitor { latch: latch.clone() }, Escritor { latch })
}

impl<const W: usize> Escritor<W> {
    /// Publica `palavras`. Exclusivo pelo `&mut self`; nunca bloqueia nem aloca.
    pub(crate) fn escrever(&mut self, palavras: [u64; W]) {
        let l = &*self.latch;
        let s = l.seq.load(Ordering::Relaxed); // só este escritor a altera
        fence(Ordering::Release);
        for (a, v) in l.slots[(s.wrapping_add(1) & 1) as usize].iter().zip(palavras) {
            a.store(v, Ordering::Relaxed);
        }
        l.seq.store(s.wrapping_add(1), Ordering::Release);
    }
}

impl<const W: usize> Leitor<W> {
    /// Uma tentativa de leitura. `Some` se a versão não mudou durante a leitura. Sem lock, sem alocação.
    pub(crate) fn tentar(&self) -> Option<[u64; W]> {
        let l = &*self.latch;
        let s1 = l.seq.load(Ordering::Acquire);
        let slot = &l.slots[(s1 & 1) as usize];
        let mut palavras = [0u64; W];
        for (p, a) in palavras.iter_mut().zip(slot.iter()) {
            *p = a.load(Ordering::Relaxed);
        }
        fence(Ordering::Acquire);
        let s2 = l.seq.load(Ordering::Relaxed);
        (s1 == s2).then_some(palavras)
    }

    /// Até `k` tentativas. `Err(Esgotado)` se todas apanharam o escritor a avançar, com o número de tentativas feitas.
    pub(crate) fn ler(&self, k: u32) -> Result<[u64; W], Esgotado> {
        let mut tentativas = 0u32;
        for _ in 0..k {
            tentativas += 1;
            if let Some(p) = self.tentar() {
                return Ok(p);
            }
        }
        Err(Esgotado { tentativas })
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    /// A paridade da versão atual (o slot que os leitores leem). Só para os oráculos; lida no próprio módulo.
    fn paridade<const W: usize>(r: &Leitor<W>) -> u64 {
        r.latch.seq.load(Ordering::SeqCst) & 1
    }

    #[derive(Debug, Clone)]
    enum Op {
        Escreve([u64; 4]),
        Le,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![any::<[u64; 4]>().prop_map(Op::Escreve), Just(Op::Le)]
    }

    /// ORÁCULO DE MODELO SEQUENCIAL (proptest, sem macros de item): contra um registo de referência, cada leitura
    /// devolve, bit a bit, a última escrita (ou o inicial). Conta as leituras feitas em cada paridade: os DOIS slots
    /// têm de ser lidos.
    #[test]
    fn modelo_sequencial_contra_registo() {
        use proptest::test_runner::{Config, TestCaseError, TestRunner};
        let casos = if cfg!(miri) { 4 } else { 512 };
        let mut runner = TestRunner::new(Config { cases: casos, ..Config::default() });
        let estrategia = (any::<[u64; 4]>(), prop::collection::vec(op(), 1..64));
        let resultado = runner.run(&estrategia, |(inicial, ops)| {
            let (r, mut w) = novo(inicial);
            let mut referencia = inicial;
            let mut por_paridade = [0usize; 2];
            let verifica = |r: &Leitor<4>, esperado: [u64; 4], por_paridade: &mut [usize; 2]| -> Result<(), TestCaseError> {
                por_paridade[paridade(r) as usize] += 1;
                prop_assert_eq!(r.ler(1), Ok(esperado));
                prop_assert_eq!(r.tentar(), Some(esperado));
                Ok(())
            };
            for o in &ops {
                match o {
                    Op::Escreve(p) => { w.escrever(*p); referencia = *p; }
                    Op::Le => verifica(&r, referencia, &mut por_paridade)?,
                }
            }
            // no fim, uma leitura em cada paridade: lê-se, escreve-se mais uma vez e lê-se de novo
            verifica(&r, referencia, &mut por_paridade)?;
            let extra = referencia.map(|x| !x);
            w.escrever(extra);
            verifica(&r, extra, &mut por_paridade)?;
            prop_assert!(por_paridade[0] > 0 && por_paridade[1] > 0, "os dois slots têm de ser lidos: {:?}", por_paridade);
            Ok(())
        });
        assert!(resultado.is_ok(), "modelo sequencial: {resultado:?}");
        eprintln!("SEQLOCK_MODELO casos={casos}");
    }

    /// A paridade alterna a cada publicação e cada slot devolve exatamente o que lá foi escrito.
    #[test]
    fn cada_publicacao_alterna_o_slot_lido() {
        let (r, mut w) = novo([7u64; 2]);
        assert_eq!((paridade(&r), r.ler(1)), (0, Ok([7, 7])));
        for i in 1..=6u64 {
            w.escrever([i, !i]);
            assert_eq!((paridade(&r), r.ler(1)), (i & 1, Ok([i, !i])), "publicação {i}");
        }
    }

    /// ORÁCULO DE FRESCURA CONCORRENTE: cada publicação escreve a versão `v` em todas as palavras e só depois é
    /// marcada como concluída. Uma leitura INICIADA depois de `c` publicações concluídas nunca devolve uma versão < `c`
    /// (nem o inicial, 0, se `c > 0`); a versão vista por cada leitor nunca recua; as palavras nunca se misturam.
    #[test]
    fn frescura_concorrente_monotonica_por_leitor() {
        let n: u64 = if cfg!(miri) { 60 } else { 200_000 };
        let (r, mut w) = novo([0u64; 4]);
        let parar = std::sync::Arc::new(AtomicBool::new(false));
        let concluidas = std::sync::Arc::new(AtomicU64::new(0));
        let mut leitores = Vec::new();
        for _ in 0..if cfg!(miri) { 1 } else { 3 } {
            let (r, p, c) = (r.clone(), parar.clone(), concluidas.clone());
            leitores.push(thread::spawn(move || {
                let (mut lidas, mut ultima) = (0u64, 0u64);
                while !p.load(Ordering::Relaxed) {
                    let antes = c.load(Ordering::Acquire);
                    let Some(v) = r.tentar() else { continue };
                    assert!(v.iter().all(|&x| x == v[0]), "snapshot misturado: {v:?}");
                    assert!(v[0] >= antes, "versão {} mais antiga do que as {antes} concluídas antes da leitura", v[0]);
                    assert!(v[0] >= ultima, "a versão recuou: {} depois de {ultima}", v[0]);
                    ultima = v[0];
                    lidas += 1;
                }
                (lidas, ultima)
            }));
        }
        for i in 1..=n {
            w.escrever([i; 4]);
            concluidas.store(i, Ordering::Release);
        }
        parar.store(true, Ordering::Relaxed);
        for h in leitores {
            let (lidas, ultima) = h.join().unwrap();
            assert!(lidas > 0, "o leitor não leu nada: o teste não exercitou nada");
            eprintln!("SEQLOCK_FRESCURA leituras={lidas} ultima_versao={ultima}");
        }
        assert_eq!(r.ler(1), Ok([n; 4]));
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// loom: 1 escritor (2 publicações, versões 1 e 2 em todas as palavras) e 1 leitor (3 tentativas), em todas as
    /// intercalações e ordens de memória que o loom modela. Um snapshot aceite nunca mistura publicações e a versão
    /// vista nunca recua.
    #[test]
    fn loom_snapshot_aceite_nunca_mistura_nem_recua() {
        loom::model(|| {
            let (r, mut w) = novo([0u64; 2]);
            let t = loom::thread::spawn(move || {
                w.escrever([1, 1]);
                w.escrever([2, 2]);
            });
            let mut ultima = 0;
            for _ in 0..3 {
                if let Some(v) = r.tentar() {
                    assert_eq!(v[0], v[1], "snapshot misturado: {v:?}");
                    assert!(v[0] >= ultima, "a versão recuou: {v:?} depois de {ultima}");
                    ultima = v[0];
                }
            }
            t.join().unwrap();
            assert_eq!(r.tentar(), Some([2, 2]));
        });
    }

    /// O3 (ronda 2): `ler(k)` com `k` publicações concorrentes (k = 1 e k = 2). Em cada intercalação, um `Err(e)` tem
    /// `e.tentativas == k` e um `Ok(v)` é coerente. Não-vacuidade: um contador `std`, fora do modelo, conta os `Err`, e
    /// exige-se pelo menos um por `k` — senão o oráculo não exercitou nada.
    ///
    /// O leitor é a thread LANÇADA e as publicações correm na principal: com o leitor na principal e k = 1, o loom só
    /// explorou UMA intercalação (medido: 1 iteração, 0 `Err`); assim explora 10 (3 `Err`) com k = 1 e 1583 (361 `Err`)
    /// com k = 2 (`ronda2/g2/exp-loom/`).
    #[test]
    fn loom_ler_esgotado_conta_as_tentativas() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        for k in [1u32, 2] {
            let erros = std::sync::Arc::new(AtomicUsize::new(0));
            let e2 = erros.clone();
            loom::model(move || {
                let (r, mut w) = novo([0u64; 2]);
                let e3 = e2.clone();
                let t = loom::thread::spawn(move || match r.ler(k) {
                    Ok(v) => assert_eq!(v[0], v[1], "snapshot misturado: {v:?}"),
                    Err(e) => {
                        assert_eq!(e.tentativas, k, "Esgotado com {} tentativas para k = {k}", e.tentativas);
                        e3.fetch_add(1, Relaxed);
                    }
                });
                for v in 1..=u64::from(k) {
                    w.escrever([v, v]);
                }
                t.join().unwrap();
            });
            let n = erros.load(Relaxed);
            eprintln!("LOOM_O3 k={k} err={n}");
            assert!(n >= 1, "k = {k}: nenhuma intercalação deu Err — o oráculo não exercitou nada");
        }
    }
}
