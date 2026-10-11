//! Latch de sequência com dois slots (`seqcount_latch`) sobre atómicos — sem `unsafe`, sem
//! memória partilhada entre instâncias (TD-030).
//!
//! # Porque existe
//! O `AudioShare` publicava os escalares num `ArcSwap` (arc-swap 1.9.1). O TD-030 mediu
//! `scalars()` a devolver um snapshot que só OUTRA instância tinha publicado. A reprodução
//! mínima só com o arc-swap 1.9.1 reproduz o mesmo (evidência da ordem F, 1a). Este latch não
//! tem nada global nem reciclado: cada instância só lê os seus próprios atómicos.
//!
//! # Um único escritor, por estrutura
//! [`novo`] é o ÚNICO construtor e devolve exatamente um [`Escritor`]. O `Escritor` não é
//! `Clone` nem `Copy`, e escrever exige `&mut Escritor`. O [`Leitor`] não tem nenhum método
//! de escrita. Nenhum caminho escreve no latch sem o `&mut` do único `Escritor`.
//!
//! # Protocolo (orderings)
//! `seq` só é alterada pelo escritor. Com `seq` par os leitores leem o slot 0; com `seq`
//! ímpar, o slot 1. O escritor escreve sempre o slot onde os leitores NÃO estão. Assim, um
//! escritor parado a meio não obriga ninguém a repetir.
//!
//! Escritor (`s` par à entrada):
//! 1. `seq.store(s+1, Release)`: o slot 1 (escrito na publicação anterior) fica completo
//!    para quem ler `s+1` com `Acquire`. Depois `fence(Release)` e o slot 0 (`Relaxed`).
//! 2. `seq.store(s+2, Release)`: o slot 0 novo fica completo para quem ler `s+2`. Depois
//!    `fence(Release)` e o slot 1 (`Relaxed`).
//!
//! Leitor: `s1 = seq.load(Acquire)`; lê o slot `s1 & 1` (`Relaxed`); `fence(Acquire)`;
//! `s2 = seq.load(Relaxed)`; `s1 == s2` → snapshot válido.
//!
//! O `fence(Release)` antes da escrita de um slot emparelha com o `fence(Acquire)` do leitor
//! depois de o ler. Se o leitor viu um valor novo nesse slot, então vê `seq` já mudada e
//! repete. Referências: H.-J. Boehm, «Can seqlocks get along with programming language memory
//! models?» (MSPC 2012); Linux `seqcount_latch`.

#[cfg(loom)]
use loom::sync::atomic::{fence, AtomicU64, Ordering};
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{fence, AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::Arc;

/// Estado partilhado: a sequência e os dois slots de `W` palavras.
struct Latch<const W: usize> {
    seq: AtomicU64,
    slots: [[AtomicU64; W]; 2],
}

/// O lado de leitura. Pode haver vários (é `Clone`); nenhum escreve.
#[derive(Clone)]
pub(crate) struct Leitor<const W: usize> {
    latch: Arc<Latch<W>>,
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
        debug_assert!(s.is_multiple_of(2), "a sequência fica par entre publicações");
        // 1. leitores → slot 1 (completo); escreve o slot 0.
        l.seq.store(s.wrapping_add(1), Ordering::Release);
        fence(Ordering::Release);
        for (a, v) in l.slots[0].iter().zip(palavras) {
            a.store(v, Ordering::Relaxed);
        }
        // 2. leitores → slot 0 (o novo, completo); escreve o slot 1.
        l.seq.store(s.wrapping_add(2), Ordering::Release);
        fence(Ordering::Release);
        for (a, v) in l.slots[1].iter().zip(palavras) {
            a.store(v, Ordering::Relaxed);
        }
    }
}

impl<const W: usize> Leitor<W> {
    /// Uma tentativa de leitura. `Some` se a sequência não mudou durante a leitura.
    /// Sem lock, sem alocação.
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

    /// Até `k` tentativas. `None` se todas apanharam o escritor a avançar.
    pub(crate) fn ler(&self, k: u32) -> Option<[u64; W]> {
        for _ in 0..k {
            if let Some(p) = self.tentar() {
                return Some(p);
            }
        }
        None
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    #[test]
    fn le_o_inicial_e_depois_o_ultimo_publicado() {
        let (r, mut w) = novo([7u64; 3]);
        assert_eq!(r.ler(1), Some([7; 3]));
        w.escrever([1, 2, 3]);
        assert_eq!(r.ler(1), Some([1, 2, 3]));
        w.escrever([4, 5, 6]);
        assert_eq!(r.ler(1), Some([4, 5, 6]));
        assert_eq!(r.clone().ler(1), Some([4, 5, 6]));
    }

    /// Cada publicação escreve a MESMA marca em todas as palavras; um leitor nunca pode ver
    /// marcas diferentes no mesmo snapshot, nem uma marca que ainda não começou a ser escrita.
    #[test]
    fn concorrente_nunca_mistura_publicacoes() {
        let n: u64 = if cfg!(miri) { 60 } else { 200_000 };
        let (r, mut w) = novo([0u64; 4]);
        let parar = std::sync::Arc::new(AtomicBool::new(false));
        let escritas = std::sync::Arc::new(AtomicU64::new(0));
        let mut leitores = Vec::new();
        for _ in 0..if cfg!(miri) { 1 } else { 3 } {
            let (r, p, e) = (r.clone(), parar.clone(), escritas.clone());
            leitores.push(thread::spawn(move || {
                let (mut ok, mut falhou) = (0u64, 0u64);
                while !p.load(Ordering::Relaxed) {
                    match r.tentar() {
                        Some(v) => {
                            assert!(v.iter().all(|&x| x == v[0]), "snapshot misturado: {v:?}");
                            let feitas = e.load(Ordering::Acquire);
                            assert!(v[0] <= feitas + 1, "marca {} do futuro (feitas={feitas})", v[0]);
                            ok += 1;
                        }
                        None => falhou += 1,
                    }
                }
                (ok, falhou)
            }));
        }
        for i in 1..=n {
            w.escrever([i; 4]);
            escritas.store(i, Ordering::Release);
        }
        parar.store(true, Ordering::Relaxed);
        let mut ok_total = 0;
        for h in leitores {
            let (ok, falhou) = h.join().unwrap();
            ok_total += ok;
            eprintln!("SEQLOCK_CONCORRENTE leituras_ok={ok} tentativas_falhadas={falhou}");
        }
        assert!(ok_total > 0, "nenhuma leitura concluída: o teste não exercitou nada");
        assert_eq!(r.ler(1), Some([n; 4]));
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// loom: 1 escritor (2 publicações) e 1 leitor (2 tentativas). Todas as intercalações
    /// e ordens de memória que o loom modela: um snapshot aceite nunca mistura publicações.
    #[test]
    fn loom_snapshot_aceite_nunca_mistura() {
        loom::model(|| {
            let (r, mut w) = novo([0u64; 2]);
            let t = loom::thread::spawn(move || {
                w.escrever([1, 1]);
                w.escrever([2, 2]);
            });
            for _ in 0..2 {
                if let Some(v) = r.tentar() {
                    assert_eq!(v[0], v[1], "snapshot misturado: {v:?}");
                }
            }
            t.join().unwrap();
            assert_eq!(r.tentar(), Some([2, 2]));
        });
    }
}
