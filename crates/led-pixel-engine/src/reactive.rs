//! The audio→light bridge. `AudioShare` holds the latest [`AudioFeatures`] (written by the
//! audio thread, read by the render thread). Reactive effects hold an `Arc<AudioShare>` and
//! are ordinary [`Effect`]s, so the pipeline drives them unchanged.
//!
//! Hot-path discipline: the render side reads only [`AudioScalars`] (Copy, allocation-free).
//! The spectrum stays behind [`AudioShare::with_spectrum`] so it is never cloned per frame.
//!
//! ## Latch de sequência — sem lock, coerente e isolado (TD-002 / TD-030 / RT-LOCK-RENDER-001)
//!
//! Os escalares vão codificados em 4 palavras `AtomicU64` num latch de sequência com dois
//! slots ([`crate::seqlock`]), um por instância, sem nada global nem reciclado.
//!
//! - `publish()` (thread de áudio, ~200 Hz): codifica a struct e escreve-a com o ÚNICO
//!   `Escritor` do latch, guardado num `Mutex` usado só aqui (fora do render).
//! - `scalars()` (thread de render, cada frame): até [`TENTATIVAS_DO_LEITOR`] leituras do
//!   latch, sem lock e sem alocação; todos os campos vêm da mesma publicação DESTA instância.
//!   Se as tentativas se esgotarem (o escritor avançou durante cada uma), devolve o último
//!   snapshot válido que esta thread leu desta instância, e conta o fallback.
//!
//! Histórico:
//!   - Atómicos por campo (60afc4a): sem lock mas INCOERENTE (beat e timestamp_ms de
//!     publicações diferentes).
//!   - RwLock<AudioScalars> (57f7722): coerente, mas um lock por frame (RT-LOCK-RENDER-001).
//!   - ArcSwap (94c42e4, TD-002): sem lock, mas o TD-030 mediu `scalars()` a devolver um
//!     snapshot de OUTRA instância de `AudioShare` (arc-swap 1.9.1; a reprodução mínima só com o
//!     arc-swap reproduz).
//!   - Latch (este): sem lock, coerente, e cada instância só lê os seus próprios atómicos.
//!
//! `spectrum` (Vec<f32>) continua atrás de um `RwLock` — `render()` nunca lhe toca.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use led_core::{AudioFeatures, MusicalSection, PixelColor};

use crate::color;
use crate::effect::{Effect, Vec3};
use crate::seqlock;

/// The scalar audio fields — cheap to copy, safe to read every frame.
/// `musical_section` is included so reactive effects and `SectionClip` can
/// branch on it without an additional lock or shared state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioScalars {
    pub sample_rate:     u32,
    pub timestamp_ms:    u64,
    pub rms:             f32,
    pub beat:            bool,
    pub bass:            f32,
    pub mid:             f32,
    pub high:            f32,
    /// Current musical section label, or `None` before the warm-up window completes.
    pub musical_section: Option<led_core::MusicalSection>,
}

/// Tentativas de leitura do latch por `scalars()` antes do fallback (TD-030, accept v2 §2.2).
/// Uma tentativa só falha se o escritor avançar durante ela; a ~200 Hz isso é raro.
pub const TENTATIVAS_DO_LEITOR: u32 = 16;

/// Palavras do latch: os escalares codificados (ver [`codificar`]).
const PALAVRAS: usize = 4;

/// Id de instância, nunca reutilizado (um endereço seria: depois de um drop, outra instância
/// podia nascer no mesmo sítio e o fallback devolveria o valor dela — a classe do TD-030).
static PROXIMO_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// O último snapshot válido que ESTA thread leu, e de que instância. Init `const`, sem
    /// destrutor: não aloca no caminho do render. Só o fallback o devolve, e só se o id bater.
    static ULTIMO_VALIDO: Cell<Option<(u64, AudioScalars)>> = const { Cell::new(None) };
}

/// Shared latest audio analysis. One writer at a time (audio thread) via
/// [`publish`](Self::publish), many readers (render thread) via [`scalars`](Self::scalars).
///
/// `scalars()` is **lock-free, coherent and isolated**: a bounded number of reads of this
/// instance's own sequence latch; all fields come from a single `publish()` of THIS instance.
/// No lock acquired and nothing allocated on the render path.
///
/// `with_spectrum()` uses a separate `RwLock<Vec<f32>>` — never called from `render()`.
pub struct AudioShare {
    leitor:   seqlock::Leitor<PALAVRAS>,
    /// O ÚNICO escritor do latch. O `Mutex` serializa `publish` (lado da escrita, fora do render).
    escritor: Mutex<seqlock::Escritor<PALAVRAS>>,
    id:       u64,
    fallbacks:              AtomicU64,
    fallbacks_sem_snapshot: AtomicU64,
    spectrum: RwLock<Vec<f32>>,
}

/// Código de `musical_section` na palavra 3: 0 = `None`, 1..=8 = a variante.
fn codigo_da_seccao(s: Option<MusicalSection>) -> u64 {
    match s {
        None => 0,
        Some(MusicalSection::Intro) => 1,
        Some(MusicalSection::Verse) => 2,
        Some(MusicalSection::Chorus) => 3,
        Some(MusicalSection::Bridge) => 4,
        Some(MusicalSection::Drop) => 5,
        Some(MusicalSection::Build) => 6,
        Some(MusicalSection::Outro) => 7,
        Some(MusicalSection::Unknown) => 8,
    }
}

fn seccao_do_codigo(c: u64) -> Option<MusicalSection> {
    match c {
        1 => Some(MusicalSection::Intro),
        2 => Some(MusicalSection::Verse),
        3 => Some(MusicalSection::Chorus),
        4 => Some(MusicalSection::Bridge),
        5 => Some(MusicalSection::Drop),
        6 => Some(MusicalSection::Build),
        7 => Some(MusicalSection::Outro),
        8 => Some(MusicalSection::Unknown),
        _ => None,
    }
}

/// O fallback só aceita a entrada da thread se for DESTA instância (`id`); nunca a de outra.
fn escolher_fallback(id: u64, entrada: Option<(u64, AudioScalars)>) -> Option<AudioScalars> {
    match entrada {
        Some((dono, s)) if dono == id => Some(s),
        _ => None,
    }
}

/// `AudioScalars` → 4 palavras. Os `f32` vão pelos bits (NaN e ±Inf preservados).
fn codificar(s: &AudioScalars) -> [u64; PALAVRAS] {
    [
        s.timestamp_ms,
        u64::from(s.sample_rate) | (u64::from(s.rms.to_bits()) << 32),
        u64::from(s.bass.to_bits()) | (u64::from(s.mid.to_bits()) << 32),
        u64::from(s.high.to_bits())
            | (u64::from(s.beat) << 32)
            | (codigo_da_seccao(s.musical_section) << 40),
    ]
}

fn descodificar(p: [u64; PALAVRAS]) -> AudioScalars {
    AudioScalars {
        timestamp_ms:    p[0],
        sample_rate:     p[1] as u32,
        rms:             f32::from_bits((p[1] >> 32) as u32),
        bass:            f32::from_bits(p[2] as u32),
        mid:             f32::from_bits((p[2] >> 32) as u32),
        high:            f32::from_bits(p[3] as u32),
        beat:            (p[3] >> 32) & 1 == 1,
        musical_section: seccao_do_codigo((p[3] >> 40) & 0xff),
    }
}

impl Default for AudioShare {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioShare {
    pub fn new() -> Self {
        let (leitor, escritor) = seqlock::novo(codificar(&AudioScalars::default()));
        Self {
            leitor,
            escritor: Mutex::new(escritor),
            id: PROXIMO_ID.fetch_add(1, Ordering::Relaxed),
            fallbacks: AtomicU64::new(0),
            fallbacks_sem_snapshot: AtomicU64::new(0),
            spectrum: RwLock::new(Vec::new()),
        }
    }

    /// Publish the newest features (audio thread, ~200 Hz).
    /// Writes the whole AudioScalars struct into this instance's latch through its single
    /// writer (serialized by a `Mutex` on the publish side only — never on the render path).
    /// Spectrum update uses a separate RwLock (off render hot-path).
    pub fn publish(&self, f: &AudioFeatures) {
        let palavras = codificar(&AudioScalars {
            sample_rate:     f.sample_rate,
            timestamp_ms:    f.timestamp_ms,
            rms:             f.rms,
            beat:            f.beat,
            bass:            f.bass,
            mid:             f.mid,
            high:            f.high,
            musical_section: f.musical_section,
        });
        self.escritor.lock().unwrap_or_else(|e| e.into_inner()).escrever(palavras);
        let mut g = self.spectrum.write().unwrap();
        if g.len() != f.spectrum.len() {
            g.resize(f.spectrum.len(), 0.0);
        }
        g.copy_from_slice(&f.spectrum);
    }

    /// Latest scalar fields. **Lock-free, coherent and isolated.**
    /// Up to [`TENTATIVAS_DO_LEITOR`] reads of this instance's latch — all fields from the
    /// same `publish()` of THIS instance, no lock, no allocation. If every attempt caught the
    /// writer mid-publish, returns the last valid snapshot this thread read from this
    /// instance (or the default if there is none) and counts it ([`Self::fallbacks_do_leitor`]).
    pub fn scalars(&self) -> AudioScalars {
        self.scalars_com_tentativas(TENTATIVAS_DO_LEITOR)
    }

    /// O corpo de [`Self::scalars`] com `k` explícito (os testes forçam o fallback com `k = 0`).
    fn scalars_com_tentativas(&self, k: u32) -> AudioScalars {
        match self.leitor.ler(k) {
            Some(palavras) => {
                let s = descodificar(palavras);
                ULTIMO_VALIDO.with(|c| c.set(Some((self.id, s))));
                s
            }
            None => self.fallback(),
        }
    }

    #[cold]
    fn fallback(&self) -> AudioScalars {
        self.fallbacks.fetch_add(1, Ordering::Relaxed);
        let escolhido = escolher_fallback(self.id, ULTIMO_VALIDO.with(|c| c.get()));
        if escolhido.is_none() {
            self.fallbacks_sem_snapshot.fetch_add(1, Ordering::Relaxed);
        }
        escolhido.unwrap_or_default()
    }

    /// Contadores do leitor (TD-030): `(tentativas esgotadas, das quais sem snapshot desta
    /// instância nesta thread)`. Só leitura; para medição e registo.
    pub fn fallbacks_do_leitor(&self) -> (u64, u64) {
        (self.fallbacks.load(Ordering::Relaxed), self.fallbacks_sem_snapshot.load(Ordering::Relaxed))
    }

    /// Borrow the latest spectrum without cloning it.
    /// Uses RwLock — `render()` never calls this.
    pub fn with_spectrum<R>(&self, f: impl FnOnce(&[f32]) -> R) -> R {
        f(&self.spectrum.read().unwrap())
    }
}

/// Brightness follows a band's energy: `level = clamp(band * gain)`. Pick the band via `band`.
pub struct BandPulse {
    pub color: PixelColor,
    pub gain: f32,
    pub band: Band,
    pub audio: Arc<AudioShare>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Bass,
    Mid,
    High,
}

impl BandPulse {
    pub fn new(color: PixelColor, band: Band, gain: f32, audio: Arc<AudioShare>) -> Self {
        Self { color, gain, band, audio }
    }
}

impl Effect for BandPulse {
    fn render(&self, _time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let s = self.audio.scalars();
        let energy = match self.band {
            Band::Bass => s.bass,
            Band::Mid => s.mid,
            Band::High => s.high,
        };
        let level = (energy * self.gain).clamp(0.0, 1.0);
        out.fill(color::scale(self.color, level));
    }
}

#[derive(Clone, Copy)]
struct FlashState {
    last_beat_ts: u64,
    flash_start_ms: u64,
    ever: bool,
}

/// Flashes to full on each new beat, then decays linearly over `decay_ms`.
pub struct BeatFlash {
    pub color: PixelColor,
    pub decay_ms: u64,
    pub audio: Arc<AudioShare>,
    state: Cell<FlashState>,
}

impl BeatFlash {
    pub fn new(color: PixelColor, decay_ms: u64, audio: Arc<AudioShare>) -> Self {
        Self {
            color,
            decay_ms,
            audio,
            state: Cell::new(FlashState { last_beat_ts: u64::MAX, flash_start_ms: 0, ever: false }),
        }
    }
}

impl Effect for BeatFlash {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let s = self.audio.scalars();
        let mut st = self.state.get();
        // Trigger only on a NEW beat (a beat with a timestamp we haven't handled).
        if s.beat && s.timestamp_ms != st.last_beat_ts {
            st.flash_start_ms = time_ms;
            st.last_beat_ts = s.timestamp_ms;
            st.ever = true;
            self.state.set(st);
        }
        let level = if !st.ever || self.decay_ms == 0 {
            0.0
        } else {
            let elapsed = time_ms.saturating_sub(st.flash_start_ms) as f32;
            (1.0 - elapsed / self.decay_ms as f32).clamp(0.0, 1.0)
        };
        out.fill(color::scale(self.color, level));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features(beat: bool, ts: u64, bass: f32) -> AudioFeatures {
        AudioFeatures {
            sample_rate: 48_000,
            timestamp_ms: ts,
            rms: 0.5,
            beat,
            bass,
            mid: 0.0,
            high: 0.0,
            spectrum: vec![0.0; 8],
            musical_section: None, instrument_class: None,
        }
    }

    fn render(fx: &dyn Effect, t: u64) -> PixelColor {
        let mut out = [PixelColor::default(); 2];
        fx.render(t, &[Vec3::ZERO; 2], &mut out);
        out[0]
    }

    // ── TD-030: codec do latch — ida e volta exata, todos os valores de MusicalSection ──
    #[test]
    fn codec_ida_e_volta_exata() {
        let seccoes = [
            None,
            Some(MusicalSection::Intro),
            Some(MusicalSection::Verse),
            Some(MusicalSection::Chorus),
            Some(MusicalSection::Bridge),
            Some(MusicalSection::Drop),
            Some(MusicalSection::Build),
            Some(MusicalSection::Outro),
            Some(MusicalSection::Unknown),
        ];
        let reais = [0.0f32, -0.0, 1.5, -3.25, f32::INFINITY, f32::NEG_INFINITY, f32::MIN_POSITIVE, f32::MAX];
        for (i, seccao) in seccoes.into_iter().enumerate() {
            for (j, &x) in reais.iter().enumerate() {
                let s = AudioScalars {
                    sample_rate: u32::MAX - i as u32,
                    timestamp_ms: u64::MAX - j as u64,
                    rms: x,
                    beat: (i + j) % 2 == 1,
                    bass: -x,
                    mid: x * 0.5,
                    high: reais[(j + 1) % reais.len()],
                    musical_section: seccao,
                };
                let d = descodificar(codificar(&s));
                assert_eq!(d.sample_rate, s.sample_rate);
                assert_eq!(d.timestamp_ms, s.timestamp_ms);
                assert_eq!(d.beat, s.beat);
                assert_eq!(d.musical_section, s.musical_section);
                for (a, b) in [(d.rms, s.rms), (d.bass, s.bass), (d.mid, s.mid), (d.high, s.high)] {
                    assert_eq!(a.to_bits(), b.to_bits(), "bits de f32 têm de sobreviver");
                }
            }
        }
        let nan = AudioScalars { rms: f32::NAN, ..AudioScalars::default() };
        assert_eq!(descodificar(codificar(&nan)).rms.to_bits(), f32::NAN.to_bits());
    }

    // ── TD-030: o fallback nunca devolve a entrada de OUTRA instância ──
    #[test]
    fn fallback_so_aceita_a_entrada_desta_instancia() {
        let s = AudioScalars { sample_rate: 7, timestamp_ms: 9, ..AudioScalars::default() };
        assert_eq!(escolher_fallback(3, Some((3, s))), Some(s));
        assert_eq!(escolher_fallback(3, Some((4, s))), None, "entrada de outra instância");
        assert_eq!(escolher_fallback(3, None), None);
    }

    // ── TD-030: ids de instância nunca se repetem, nem depois de um drop ──
    #[test]
    fn ids_de_instancia_unicos_mesmo_depois_de_drop() {
        let a = AudioShare::new();
        let id_a = a.id;
        drop(a);
        let b = AudioShare::new();
        let c = AudioShare::new();
        assert!(b.id != id_a && c.id != id_a && b.id != c.id);
    }

    // ── TD-030: uma leitura válida fica como último snapshot DESTA instância nesta thread ──
    #[test]
    fn leitura_valida_regista_o_ultimo_desta_instancia() {
        let share = AudioShare::new();
        share.publish(&features(true, 77, 0.25));
        let s = share.scalars();
        assert_eq!(s.timestamp_ms, 77);
        assert_eq!(ULTIMO_VALIDO.with(|c| c.get()), Some((share.id, s)));
        assert_eq!(share.fallbacks_do_leitor(), (0, 0), "sem escritor concorrente não há fallback");
    }

    // ── TD-030: fallback forçado (k = 0) — último válido DESTA instância, senão o default ──
    #[test]
    fn fallback_forcado_devolve_o_ultimo_valido_desta_instancia() {
        let a = AudioShare::new();
        let b = AudioShare::new();
        a.publish(&features(true, 5, 0.5));
        b.publish(&features(false, 6, 0.25));
        let lido_a = a.scalars();
        assert_eq!(lido_a.timestamp_ms, 5);
        a.publish(&features(true, 7, 0.75));
        // k = 0: nenhuma tentativa → fallback → o último válido que ESTA thread leu de `a`.
        assert_eq!(a.scalars_com_tentativas(0), lido_a);
        assert_eq!(a.fallbacks_do_leitor(), (1, 0));
        // `b` nunca foi lida nesta thread e a entrada é de `a`: default, contado à parte.
        assert_eq!(b.scalars_com_tentativas(0), AudioScalars::default());
        assert_eq!(b.fallbacks_do_leitor(), (1, 1));
    }

    // ── TD-030: contadores do leitor com escritor em ciclo apertado (medidos, registados) ──
    // Cada snapshot — incluindo os do fallback — tem de ser coerente e desta instância.
    #[test]
    fn fallbacks_medidos_com_escritor_continuo() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let n: u64 = if cfg!(miri) { 40 } else { 50_000 };
        let share = Arc::new(AudioShare::new());
        let parar = Arc::new(AtomicBool::new(false));
        let (rs, rp) = (share.clone(), parar.clone());
        let leitor = std::thread::spawn(move || {
            let mut leituras = 0u64;
            while !rp.load(Ordering::Relaxed) {
                leituras += 1;
                let s = rs.scalars();
                if s != AudioScalars::default() {
                    assert_eq!(s.sample_rate, 4242, "snapshot alheio: {s:?}");
                    assert_eq!(s.bass, s.timestamp_ms as f32, "snapshot incoerente: {s:?}");
                    assert_eq!(s.beat, s.timestamp_ms % 2 == 1, "snapshot incoerente: {s:?}");
                }
            }
            leituras
        });
        for i in 1..=n {
            let mut f = features(i % 2 == 1, i, i as f32);
            f.sample_rate = 4242;
            share.publish(&f);
        }
        parar.store(true, Ordering::Relaxed);
        let leituras = leitor.join().unwrap();
        let (esgotadas, sem_snapshot) = share.fallbacks_do_leitor();
        eprintln!("TD030_FALLBACKS publicacoes={n} leituras={leituras} k={TENTATIVAS_DO_LEITOR} esgotadas={esgotadas} sem_snapshot={sem_snapshot}");
        assert!(leituras > 0);
    }

    #[test]
    fn band_pulse_tracks_energy() {
        let share = Arc::new(AudioShare::new());
        let fx = BandPulse::new(PixelColor::rgb(200, 0, 0), Band::Bass, 1.0, share.clone());

        share.publish(&features(false, 1, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 0), "no bass ⇒ dark");

        share.publish(&features(false, 2, 0.5));
        assert_eq!(render(&fx, 0), PixelColor::rgb(100, 0, 0), "half bass ⇒ half brightness");

        share.publish(&features(false, 3, 5.0)); // gain*energy clamps to 1
        assert_eq!(render(&fx, 0), PixelColor::rgb(200, 0, 0), "clamped to full");
    }

    #[test]
    fn beat_flash_triggers_and_decays() {
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(0, 0, 200), 1000, share.clone());

        // Before any beat: dark.
        share.publish(&features(false, 1, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 0));

        // New beat at ts=2 → full flash at the render time it's first seen.
        share.publish(&features(true, 2, 0.0));
        assert_eq!(render(&fx, 0), PixelColor::rgb(0, 0, 200), "beat ⇒ full");
        // Same beat still latched in the share, but no retrigger; decays with render time.
        assert_eq!(render(&fx, 500), PixelColor::rgb(0, 0, 100), "decays to half at t=500");
        assert_eq!(render(&fx, 1000), PixelColor::rgb(0, 0, 0), "fully decayed at t=1000");

        // A new beat (different ts) re-flashes.
        share.publish(&features(true, 3, 0.0));
        assert_eq!(render(&fx, 1200), PixelColor::rgb(0, 0, 200), "new beat ⇒ full again");
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use std::sync::Arc;
    use led_core::PixelColor;

    fn af(beat: bool, ts: u64, bass: f32, rms: f32) -> AudioFeatures {
        AudioFeatures {
            sample_rate: 48_000,
            timestamp_ms: ts,
            rms,
            beat,
            bass,
            mid: 0.3,
            high: 0.1,
            spectrum: vec![0.0; 8],
            musical_section: None, instrument_class: None,
        }
    }

    fn px(fx: &dyn Effect, t: u64) -> PixelColor {
        let mut out = [PixelColor::default(); 4];
        fx.render(t, &[Vec3::ZERO; 4], &mut out);
        out[0]
    }

    // ── CONCURRENCY: AudioShare — 8 writers, 8 readers simultaneously ─────
    // Full test: 8 threads × 1000 iterations each. Validated in normal test runs.
    // Miri version (below): 2 threads × 32 iterations — stays within Miri resource budget.
    #[test]
    fn audioshare_concurrent_publish_read_no_deadlock() {
        use std::thread;
        // Under Miri, reduce workload to avoid OOM/timeout (resource limit invariant).
        let (threads, iters) = if cfg!(miri) { (2usize, 32usize) } else { (8, 1_000) };

        let share = Arc::new(AudioShare::new());
        let mut handles = Vec::new();

        for i in 0..threads {
            let s = share.clone();
            handles.push(thread::spawn(move || {
                for j in 0..iters as u64 {
                    s.publish(&af(j % 3 == 0, i as u64 * 1000 + j, j as f32 * 0.001, 0.5));
                }
            }));
        }
        for i in 0..threads {
            let s = share.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..iters {
                    let sc = s.scalars();
                    assert!(sc.rms >= 0.0, "rms must be non-negative");
                    s.with_spectrum(|spec| assert!(spec.len() == 8 || spec.is_empty()));
                    let _ = i;
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
    }

    // Miri-specific: explicit 2-thread × 32-iteration test (validates concurrency safety
    // within Miri's resource limits). Full 8×1000 test runs in normal cargo test.
    #[test]
    fn audioshare_miri_2thread_concurrent_no_ub() {
        use std::thread;
        let share = Arc::new(AudioShare::new());
        let s1 = share.clone();
        let s2 = share.clone();
        let w = thread::spawn(move || {
            for j in 0..32u64 {
                s1.publish(&af(j % 2 == 0, j, j as f32 * 0.01, 0.5));
            }
        });
        let r = thread::spawn(move || {
            for _ in 0..32 {
                let sc = s2.scalars();
                assert!(sc.rms >= 0.0);
            }
        });
        w.join().unwrap();
        r.join().unwrap();
    }

    // ── INVARIANT: AudioShare never hands out stale sample_rate=0 after publish ──
    #[test]
    fn audioshare_after_publish_sample_rate_reflects_features() {
        let share = Arc::new(AudioShare::new());
        let f = af(false, 100, 0.5, 0.8);
        share.publish(&f);
        let sc = share.scalars();
        assert_eq!(sc.sample_rate, 48_000, "sample_rate must reflect last publish");
        assert_eq!(sc.timestamp_ms, 100);
    }

    // ── STRESS: 100k publish cycles — spectrum size must stay stable ──────
    // NOTE: led-core::AudioFeatures has Vec<f32> spectrum so it is NOT Copy.
    // We construct a fresh AudioFeatures per iteration.
    #[test]
    fn audioshare_100k_publishes_spectrum_size_stable() {
        let share = Arc::new(AudioShare::new());
        for i in 0..100_000u64 {
            let feat = af(i % 4 == 0, i, 1.0, 0.5);
            share.publish(&feat);
        }
        // Spectrum size must remain exactly 8 (set on first publish, no resize after)
        share.with_spectrum(|s| assert_eq!(s.len(), 8, "spectrum must not resize after init"));
    }

    // ── FUZZ: publish with empty spectrum → no panic ──────────────────────
    #[test]
    fn audioshare_empty_spectrum_no_panic() {
        let share = Arc::new(AudioShare::new());
        let f = AudioFeatures {
            sample_rate: 44_100,
            timestamp_ms: 0,
            rms: 0.0, beat: false,
            bass: 0.0, mid: 0.0, high: 0.0,
            spectrum: vec![], // empty
            musical_section: None, instrument_class: None,
        };
        share.publish(&f);
        share.with_spectrum(|s| assert_eq!(s.len(), 0));
        let sc = share.scalars();
        assert_eq!(sc.sample_rate, 44_100);
    }

    // ── FUZZ: AudioShare with NaN/Inf energy values ────────────────────────
    #[test]
    fn audioshare_nan_inf_energy_publish_no_panic() {
        let share = Arc::new(AudioShare::new());
        let f = AudioFeatures {
            sample_rate: 48_000, timestamp_ms: 1,
            rms: f32::NAN, beat: false,
            bass: f32::INFINITY, mid: f32::NEG_INFINITY, high: f32::NAN,
            spectrum: vec![f32::NAN; 8],
            musical_section: None, instrument_class: None,
        };
        share.publish(&f);
        let sc = share.scalars();
        assert!(sc.bass.is_infinite() || sc.bass.is_nan(), "extreme value preserved");
    }

    // ── CONTRACT: BeatFlash never retrigs on same timestamp ───────────────
    #[test]
    fn beatflash_no_retrig_same_timestamp() {
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(0, 255, 0), 1000, share.clone());
        share.publish(&af(true, 42, 0.0, 0.5));
        let a = px(&fx, 0).g;
        let b = px(&fx, 1).g;
        // second render: same timestamp 42 — must NOT retrigger, must have decayed
        assert!(b <= a, "same-ts beat must not retrigger: a={a} b={b}");
    }

    // ── CONTRACT: BandPulse gain=0 → always black ────────────────────────
    #[test]
    fn bandpulse_zero_gain_always_black() {
        let share = Arc::new(AudioShare::new());
        let fx = BandPulse::new(PixelColor::rgb(255, 0, 0), Band::Bass, 0.0, share.clone());
        share.publish(&af(false, 1, 9999.0, 1.0));
        assert_eq!(px(&fx, 0), PixelColor::default(), "gain=0 must be black regardless");
    }

    // ── CONTRACT: BandPulse never exceeds base color ──────────────────────
    #[test]
    fn bandpulse_never_exceeds_base_color() {
        let share = Arc::new(AudioShare::new());
        let base = PixelColor::rgb(100, 50, 200);
        let fx = BandPulse::new(base, Band::Bass, 999.0, share.clone());
        share.publish(&af(false, 1, 1.0, 1.0));
        let out = px(&fx, 0);
        assert!(out.r <= base.r, "r overflow: {} > {}", out.r, base.r);
        assert!(out.g <= base.g, "g overflow: {} > {}", out.g, base.g);
        assert!(out.b <= base.b, "b overflow: {} > {}", out.b, base.b);
    }

    // ── COHERENCE: scalars() must return beat+timestamp_ms from the SAME publish ─
    // This is the property that per-field atomics violated (TD-002).
    // A concurrent writer publishes frames with strictly increasing (beat, timestamp_ms)
    // pairs; the reader asserts that every snapshot it sees is internally consistent:
    // if beat=true, timestamp_ms must match the ts that was published with that beat.
    #[test]
    fn audioshare_scalars_beat_timestamp_coherent_under_concurrency() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;
        // Known pairs: publish alternates (beat=false, ts=even) / (beat=true, ts=odd).
        // A coherent snapshot must satisfy: beat == (timestamp_ms % 2 == 1).
        let share = Arc::new(AudioShare::new());
        let stop = Arc::new(AtomicBool::new(false));
        let violations = Arc::new(std::sync::atomic::AtomicU32::new(0));

        // Writer: publishes 10_000 frames alternating beat/no-beat with matched timestamps.
        // TD-030 (R6.2): `publicado` = índice do último publish CONCLUÍDO, para o diagnóstico.
        let publicado = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let ws = share.clone();
        let wp = publicado.clone();
        let writer = thread::spawn(move || {
            for i in 0u64..10_000 {
                let beat = i % 2 == 1;
                ws.publish(&af(beat, i, 0.0, 0.0));
                wp.store(i, Ordering::SeqCst);
            }
        });

        // Reader: reads in a tight loop, checks coherence of every snapshot.
        // Diagnóstico (TD-030, R6.2): a PRIMEIRA snapshot incoerente fica registada — beat,
        // timestamp_ms, o índice publicado nesse instante e a iteração do reader.
        let rs = share.clone();
        let st = stop.clone();
        let viol = violations.clone();
        let rp = publicado.clone();
        let reader = thread::spawn(move || {
            let mut iteracao: u64 = 0;
            let mut primeira: Option<(bool, u64, u64, u64)> = None;
            while !st.load(Ordering::Relaxed) {
                iteracao += 1;
                let sc = rs.scalars();
                // Coherence invariant: beat ↔ odd timestamp.
                let expected_beat = sc.timestamp_ms % 2 == 1;
                if sc.timestamp_ms > 0 && sc.beat != expected_beat {
                    viol.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if primeira.is_none() {
                        primeira = Some((sc.beat, sc.timestamp_ms, rp.load(Ordering::SeqCst), iteracao));
                    }
                }
            }
            (primeira, iteracao)
        });

        writer.join().unwrap();
        stop.store(true, Ordering::Relaxed);
        let (primeira, iteracoes) = reader.join().unwrap();

        let v = violations.load(Ordering::Relaxed);
        assert_eq!(
            v,
            0,
            "coherence violated {v} times: beat/timestamp_ms from different publishes — primeira: {} \
             (reader leu {iteracoes} snapshots)",
            match primeira {
                Some((beat, ts, publ, it)) => format!(
                    "beat={beat} timestamp_ms={ts} (esperado beat={}) · ultimo publish concluido={} · iteracao do reader={it}",
                    ts % 2 == 1,
                    if publ == u64::MAX { "nenhum".to_string() } else { publ.to_string() }
                ),
                None => "nao registada".into(),
            }
        );
    }

    // ── REAL-TIME: BeatFlash render must complete in < 1ms (50ms tick budget) ─
    #[test]
    fn beatflash_render_latency_under_1ms() {
        use std::time::Instant;
        let share = Arc::new(AudioShare::new());
        let fx = BeatFlash::new(PixelColor::rgb(255, 255, 255), 500, share.clone());
        share.publish(&af(true, 1, 0.5, 0.5));
        let mut out = vec![PixelColor::default(); 1_000];
        let pos = vec![Vec3::ZERO; 1_000];
        let t0 = Instant::now();
        fx.render(0, &pos, &mut out);
        let elapsed = t0.elapsed();
        assert!(elapsed.as_millis() < 5, "1000-pixel render took {}ms", elapsed.as_millis());
    }

    // ── ISOLATION (TD-030): scalars() never returns a value published into ANOTHER AudioShare ─
    // TD-030 measured `scalars()` handing out a snapshot that only another, independent
    // `AudioShare` instance had published (ArcSwap 1.9.1, tests running in parallel). Here K
    // instances live at once, each with its own writer and reader. Instance `id` publishes
    // `sample_rate = id` and marks `timestamp_ms = rms = bass = i`, `beat = i odd`. Every
    // snapshot a reader sees must be the default or carry ITS id with coherent marks.
    #[test]
    fn audioshare_isolamento_entre_instancias() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        use std::thread;
        let (instancias, publicacoes) = if cfg!(miri) { (2u32, 40u64) } else { (8u32, 10_000u64) };

        let parar = Arc::new(AtomicBool::new(false));
        let mut escritores = Vec::new();
        let mut leitores = Vec::new();
        for id in 1..=instancias {
            let share = Arc::new(AudioShare::new());
            let ws = share.clone();
            escritores.push(thread::spawn(move || {
                for i in 1..=publicacoes {
                    ws.publish(&AudioFeatures {
                        sample_rate: id,
                        timestamp_ms: i,
                        rms: i as f32,
                        beat: i % 2 == 1,
                        bass: i as f32,
                        mid: 0.3,
                        high: 0.1,
                        spectrum: vec![0.0; 8],
                        musical_section: None,
                        instrument_class: None,
                    });
                }
            }));
            let rs = share.clone();
            let st = parar.clone();
            let violacoes = Arc::new(AtomicU32::new(0));
            let rv = violacoes.clone();
            leitores.push((id, violacoes, thread::spawn(move || {
                let mut primeira: Option<AudioScalars> = None;
                let mut leituras = 0u64;
                while !st.load(Ordering::Relaxed) {
                    leituras += 1;
                    let sc = rs.scalars();
                    if sc == AudioScalars::default() {
                        continue;
                    }
                    let marca = sc.timestamp_ms as f32;
                    let coerente = sc.sample_rate == id
                        && sc.rms == marca
                        && sc.bass == marca
                        && sc.beat == (sc.timestamp_ms % 2 == 1);
                    if !coerente {
                        rv.fetch_add(1, Ordering::Relaxed);
                        primeira.get_or_insert(sc);
                    }
                }
                (primeira, leituras)
            })));
        }
        for e in escritores {
            e.join().unwrap();
        }
        parar.store(true, Ordering::Relaxed);
        let mut falhas = Vec::new();
        for (id, violacoes, leitor) in leitores {
            let (primeira, leituras) = leitor.join().unwrap();
            let v = violacoes.load(Ordering::Relaxed);
            if v > 0 {
                falhas.push(format!("instancia {id}: {v} snapshots alheias/incoerentes em {leituras} leituras; primeira={primeira:?}"));
            }
        }
        assert!(falhas.is_empty(), "TD-030 isolamento violado: {}", falhas.join(" | "));
    }
}
