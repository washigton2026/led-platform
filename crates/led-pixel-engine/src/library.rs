//! Biblioteca de efeitos — os efeitos "de show" construídos sobre o contrato do
//! [`Effect`](crate::effect::Effect).
//!
//! `effect.rs` guarda o **trait** e as três primitivas (`SolidColor`, `Rainbow`, `Pulse`).
//! Este módulo guarda a **biblioteca**: o vocabulário que um operador espera encontrar.
//!
//! ## As três regras que todo efeito daqui obedece (ADR-0021)
//!
//! 1. **Função pura de `(time_ms, position, index)`.** Sem estado guardado — a assinatura
//!    `&self` já força isso, e é o que preserva o replay determinístico.
//! 2. **Aleatoriedade é hash, nunca fluxo.** Ver [`crate::noise`].
//! 3. **Parâmetro espacial é taxa por metro/por segundo, nunca coordenada normalizada.**
//!    O efeito não recebe as dimensões do rig — só a posição de cada pixel. `cycles_per_m`
//!    (idioma já usado por `Rainbow`) funciona sem conhecer o tamanho; `0..1` não.
//!    Onde a extensão é mesmo necessária (um cometa que dá a volta), ela é **parâmetro
//!    declarado** (`span_m`): quem monta o show sabe o tamanho do rig, o efeito não.
//!
//! ## Sobre valores não-finitos
//!
//! Posições podem chegar com `NaN` (dados importados). Nenhum efeito daqui entra em pânico
//! nem produz cor indefinida: a conversão final para `u8` satura, e [`crate::noise`] trata
//! não-finito explicitamente. Há teste negativo cobrindo isto.

use led_core::PixelColor;

use crate::color;
use crate::effect::{Effect, Vec3};
use crate::noise::{fbm, hash01};

/// Onda triangular em `[0, 1]`: 0 nas pontas, 1 no meio. Contínua e sem `powf`.
#[inline]
fn triangle(t: f32) -> f32 {
    let f = t.rem_euclid(1.0);
    1.0 - (2.0 * f - 1.0).abs()
}

/// Interpola duas cores. `t` fora de `[0,1]` é clampado.
#[inline]
fn lerp_color(a: PixelColor, b: PixelColor, t: f32) -> PixelColor {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t + 0.5) as u8;
    PixelColor::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

// ── Chase ──────────────────────────────────────────────────────────────────────

/// Um pulso que corre pelo eixo x, com cauda que se apaga atrás da cabeça.
///
/// O espaçamento é **por metro**, então o mesmo `Chase` funciona num rig de 2 m ou de 200 m
/// sem reconfigurar — é o mesmo idioma do `Rainbow::cycles_per_m`.
#[derive(Clone, Copy, Debug)]
pub struct Chase {
    pub color: PixelColor,
    /// Velocidade da cabeça, metros por segundo. Negativo corre no sentido oposto.
    pub speed_m_s: f32,
    /// Distância entre duas cabeças consecutivas, em metros.
    pub spacing_m: f32,
    /// Comprimento da cauda como fração de `spacing_m`, em `[0,1]`. 0 = só a cabeça.
    pub tail_frac: f32,
}

impl Effect for Chase {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let spacing = if self.spacing_m.abs() > f32::EPSILON { self.spacing_m } else { 1.0 };
        let tail = self.tail_frac.clamp(0.0, 1.0);
        for (i, p) in positions.iter().enumerate() {
            let phase = (p.x - self.speed_m_s * t) / spacing;
            // Distância ATRÁS da cabeça, em ciclos: 0 exatamente na cabeça.
            let d = (-phase).rem_euclid(1.0);
            let b = if tail > 0.0 && d < tail { 1.0 - d / tail } else { 0.0 };
            out[i] = color::scale(self.color, b);
        }
    }
}

// ── Twinkle ────────────────────────────────────────────────────────────────────

/// Cintilação esparsa: uma fração dos pixels pisca fora de fase; o resto fica no brilho base.
///
/// Quais pixels cintilam é decidido por **hash do índice** — estável entre frames (um pixel
/// não muda de papel a cada quadro) e sem guardar nada.
#[derive(Clone, Copy, Debug)]
pub struct Twinkle {
    pub color: PixelColor,
    /// Brilho dos pixels que não cintilam, em `[0,1]`.
    pub base: f32,
    /// Fração dos pixels que cintilam, em `[0,1]`.
    pub density: f32,
    /// Ciclos de cintilação por segundo.
    pub rate_hz: f32,
    pub seed: u64,
}

impl Effect for Twinkle {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let base = self.base.clamp(0.0, 1.0);
        let density = self.density.clamp(0.0, 1.0);
        // Único efeito da biblioteca indexado só pela ORDEM do pixel, não pela posição —
        // cintilação é sobre "qual lâmpada", não sobre "onde ela está".
        for (i, o) in out.iter_mut().enumerate() {
            let key = i as u64;
            let b = if hash01(key, self.seed) < density {
                // Fase própria por pixel: sem isso o rig inteiro piscaria junto.
                let phase = hash01(key, self.seed ^ 0xA5A5_A5A5);
                base + (1.0 - base) * triangle(t * self.rate_hz + phase)
            } else {
                base
            };
            *o = color::scale(self.color, b);
        }
    }
}

// ── Fire ───────────────────────────────────────────────────────────────────────

/// Fogo por **ruído fractal**, subindo no eixo y.
///
/// ⚠️ **Divergência algorítmica honesta:** o `Fire2012` clássico (FastLED) propaga calor
/// entre pixels vizinhos **de um frame para o outro** — é inerentemente com estado, e por
/// isso incompatível com a regra 1 do ADR-0021. Este efeito produz uma aparência de fogo
/// por ruído fractal deslizante, o que é **visualmente próximo mas não o mesmo algoritmo**.
/// O que se ganha em troca é replay determinístico e custo constante por pixel.
#[derive(Clone, Copy, Debug)]
pub struct Fire {
    /// Quão rápido as chamas sobem, em metros por segundo.
    pub speed_m_s: f32,
    /// Detalhe espacial: células de ruído por metro. Maior = chamas mais finas.
    pub cells_per_m: f32,
    /// Perda de calor por metro de altura. Maior = fogo mais baixo.
    pub cooling_per_m: f32,
    /// Variação lateral: quanto o x desloca o ruído. 0 = todas as colunas idênticas.
    pub lateral: f32,
    pub seed: u64,
}

/// Rampa de calor preto → vermelho → laranja → amarelo → branco, sem ramificação.
#[inline]
fn heat_color(h: f32) -> PixelColor {
    let h = h.clamp(0.0, 1.0) * 3.0;
    let ch = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    PixelColor::rgb(ch(h), ch(h - 1.0), ch(h - 2.0))
}

impl Effect for Fire {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        for (i, p) in positions.iter().enumerate() {
            let x = p.y * self.cells_per_m - t * self.speed_m_s * self.cells_per_m
                + p.x * self.lateral;
            let n = fbm(x, 4, self.seed);
            let heat = n - self.cooling_per_m * p.y;
            out[i] = heat_color(heat);
        }
    }
}

// ── ColorWash ──────────────────────────────────────────────────────────────────

/// Transição suave do rig inteiro entre duas cores. O efeito de fundo mais usado num show.
#[derive(Clone, Copy, Debug)]
pub struct ColorWash {
    pub a: PixelColor,
    pub b: PixelColor,
    /// Ciclos completos (a → b → a) por segundo.
    pub hz: f32,
}

impl Effect for ColorWash {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        out.fill(lerp_color(self.a, self.b, triangle(t * self.hz)));
    }
}

// ── Strobe ─────────────────────────────────────────────────────────────────────

/// Piscada dura ligado/desligado.
///
/// ⚠️ **Segurança de plateia.** Frequências entre [`Strobe::SEIZURE_RISK_HZ`] são a faixa
/// associada a convulsão fotossensível; várias jurisdições limitam estroboscópio em evento
/// público. Este efeito **não clampa em silêncio** — seguindo o precedente do ADR-0018
/// ("o validador declara, o guard bloqueia"), um comportamento surpreendente no palco é
/// pior que um parâmetro documentado. Use [`Strobe::is_in_seizure_risk_band`] para avisar
/// ou bloquear na camada que tem contexto para decidir.
#[derive(Clone, Copy, Debug)]
pub struct Strobe {
    pub color: PixelColor,
    /// Piscadas por segundo.
    pub hz: f32,
    /// Fração de cada período em que fica aceso, em `[0,1]`.
    pub duty: f32,
}

impl Strobe {
    /// Faixa de risco de convulsão fotossensível, em Hz (limites inclusivos).
    pub const SEIZURE_RISK_HZ: (f32, f32) = (3.0, 60.0);

    /// `true` se `hz` cai na faixa de risco. Consulta declarativa — não altera o render.
    pub fn is_in_seizure_risk_band(hz: f32) -> bool {
        let (lo, hi) = Self::SEIZURE_RISK_HZ;
        hz >= lo && hz <= hi
    }
}

impl Effect for Strobe {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let phase = (t * self.hz).rem_euclid(1.0);
        let on = phase < self.duty.clamp(0.0, 1.0);
        out.fill(if on { self.color } else { PixelColor::rgb(0, 0, 0) });
    }
}

// ── Meteor ─────────────────────────────────────────────────────────────────────

/// Cometa: uma cabeça brilhante atravessando o rig, deixando cauda que se desfaz.
///
/// Substitui a aproximação `Meteors → Rainbow` documentada em
/// `led-demo/examples/robot_sequence.rs` — o show real do rig usa este efeito.
///
/// `span_m` é declarado porque a volta do cometa depende do tamanho do rig, e o efeito não
/// conhece esse tamanho (regra 3 do ADR-0021). Quem monta o show conhece.
#[derive(Clone, Copy, Debug)]
pub struct Meteor {
    pub color: PixelColor,
    /// Extensão do percurso em x, em metros — o cometa dá a volta ao chegar no fim.
    pub span_m: f32,
    pub speed_m_s: f32,
    /// Comprimento da cauda em metros.
    pub tail_m: f32,
    /// Fração dos pixels da cauda que "esfarela" a cada instante, em `[0,1]`.
    pub sparkle: f32,
    pub seed: u64,
}

impl Effect for Meteor {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let span = if self.span_m > f32::EPSILON { self.span_m } else { 1.0 };
        let tail = self.tail_m.max(0.0);
        let head = (self.speed_m_s * t).rem_euclid(span);
        let sparkle = self.sparkle.clamp(0.0, 1.0);
        // A cauda esfarela em degraus de 50 ms: mais rápido vira ruído, mais lento congela.
        let bucket = time_ms / 50;
        for (i, p) in positions.iter().enumerate() {
            let d = (head - p.x).rem_euclid(span); // distância atrás da cabeça
            let mut b = if tail > 0.0 && d <= tail { 1.0 - d / tail } else { 0.0 };
            if b > 0.0 && hash01(i as u64, self.seed ^ bucket) < sparkle {
                b *= 0.25;
            }
            out[i] = color::scale(self.color, b);
        }
    }
}

// ── Lightning ──────────────────────────────────────────────────────────────────

/// Relâmpago: clarões do rig inteiro em instantes pseudo-aleatórios, com queda rápida.
///
/// Substitui a aproximação `Lightning → Pulse 6 Hz` documentada em
/// `led-demo/examples/robot_sequence.rs`. A diferença que se vê no palco: o `Pulse` é
/// periódico e previsível; um relâmpago **não** é.
#[derive(Clone, Copy, Debug)]
pub struct Lightning {
    pub color: PixelColor,
    /// Janela entre oportunidades de raio, em milissegundos.
    pub window_ms: u64,
    /// Fração das janelas que efetivamente têm raio, em `[0,1]`.
    pub probability: f32,
    /// Duração da queda de brilho de um raio, em milissegundos.
    pub decay_ms: u64,
    pub seed: u64,
}

impl Effect for Lightning {
    fn render(&self, time_ms: u64, _positions: &[Vec3], out: &mut [PixelColor]) {
        let window = self.window_ms.max(1);
        let decay = self.decay_ms.max(1);
        let bucket = time_ms / window;
        let roll = hash01(bucket, self.seed);
        let b = if roll < self.probability.clamp(0.0, 1.0) {
            let elapsed = time_ms % window;
            let fade = 1.0 - (elapsed as f32 / decay as f32);
            // Intensidade própria por raio: nem todo relâmpago tem a mesma força.
            let strength = 0.5 + 0.5 * hash01(bucket, self.seed ^ 0x5EED);
            (fade * strength).max(0.0)
        } else {
            0.0
        };
        out.fill(color::scale(self.color, b));
    }
}

// ── Ripple ─────────────────────────────────────────────────────────────────────

/// Ondas concêntricas partindo de um ponto, com atenuação pela distância.
///
/// O único efeito da biblioteca que usa as três coordenadas — útil para rigs em que os
/// pixels estão espalhados no espaço (o caso dos robôs) e não numa linha.
#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    pub color: PixelColor,
    pub center: Vec3,
    pub speed_m_s: f32,
    /// Distância entre duas cristas, em metros.
    pub wavelength_m: f32,
    /// Atenuação por metro de distância do centro. 0 = sem atenuação.
    pub falloff_per_m: f32,
}

impl Effect for Ripple {
    fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let t = time_ms as f32 / 1000.0;
        let lambda =
            if self.wavelength_m.abs() > f32::EPSILON { self.wavelength_m } else { 1.0 };
        for (i, p) in positions.iter().enumerate() {
            let (dx, dy, dz) =
                (p.x - self.center.x, p.y - self.center.y, p.z - self.center.z);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let crest = triangle((dist - self.speed_m_s * t) / lambda);
            let atten = 1.0 / (1.0 + self.falloff_per_m.max(0.0) * dist);
            out[i] = color::scale(self.color, crest * atten);
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize, step_m: f32) -> Vec<Vec3> {
        (0..n).map(|i| Vec3::new(i as f32 * step_m, 0.0, 0.0)).collect()
    }

    fn buf(n: usize) -> Vec<PixelColor> {
        vec![PixelColor::default(); n]
    }

    const BLACK: PixelColor = PixelColor { r: 0, g: 0, b: 0 };

    // ── A regra que rege todos: pureza ────────────────────────────────────────

    /// A propriedade de que o replay verificado por hash depende. Se um efeito guardasse
    /// estado, esta seria a primeira a quebrar.
    #[test]
    fn every_effect_is_a_pure_function_of_time() {
        let pos = line(64, 0.05);
        let effects: Vec<Box<dyn Effect>> = vec![
            Box::new(Chase {
                color: PixelColor::rgb(255, 0, 0),
                speed_m_s: 1.0,
                spacing_m: 0.5,
                tail_frac: 0.6,
            }),
            Box::new(Twinkle {
                color: PixelColor::rgb(255, 255, 255),
                base: 0.1,
                density: 0.3,
                rate_hz: 2.0,
                seed: 11,
            }),
            Box::new(Fire {
                speed_m_s: 0.8,
                cells_per_m: 6.0,
                cooling_per_m: 0.4,
                lateral: 3.0,
                seed: 4,
            }),
            Box::new(ColorWash {
                a: PixelColor::rgb(255, 0, 0),
                b: PixelColor::rgb(0, 0, 255),
                hz: 0.25,
            }),
            Box::new(Strobe { color: PixelColor::rgb(255, 255, 255), hz: 8.0, duty: 0.2 }),
            Box::new(Meteor {
                color: PixelColor::rgb(0, 200, 255),
                span_m: 3.2,
                speed_m_s: 2.0,
                tail_m: 0.5,
                sparkle: 0.3,
                seed: 9,
            }),
            Box::new(Lightning {
                color: PixelColor::rgb(255, 255, 255),
                window_ms: 400,
                probability: 0.5,
                decay_ms: 120,
                seed: 2,
            }),
            Box::new(Ripple {
                color: PixelColor::rgb(0, 255, 128),
                center: Vec3::new(1.6, 0.0, 0.0),
                speed_m_s: 1.5,
                wavelength_m: 0.8,
                falloff_per_m: 0.5,
            }),
        ];

        for (n, fx) in effects.iter().enumerate() {
            let (mut a, mut b) = (buf(64), buf(64));
            // Renderiza fora de ordem entre as duas passadas: se houvesse estado interno,
            // a ordem das chamadas mudaria o resultado.
            for t in [0u64, 137, 999, 4321] {
                fx.render(t, &pos, &mut a);
                fx.render(t + 1, &pos, &mut b); // "suja" qualquer estado hipotético
                fx.render(t, &pos, &mut b);
                assert_eq!(a, b, "efeito {n} não é puro em t={t}");
            }
        }
    }

    /// Controle negativo — posições sujas (`NaN`/`∞`) nunca podem virar pânico. Mesma
    /// classe do BUG-3 já pago por este repo.
    #[test]
    fn negative_control_non_finite_positions_do_not_panic() {
        let pos = vec![
            Vec3::new(f32::NAN, 0.0, 0.0),
            Vec3::new(f32::INFINITY, f32::NEG_INFINITY, f32::NAN),
            Vec3::ZERO,
        ];
        let mut out = buf(3);

        Chase { color: PixelColor::rgb(9, 9, 9), speed_m_s: 1.0, spacing_m: 0.3, tail_frac: 0.5 }
            .render(500, &pos, &mut out);
        Fire { speed_m_s: 1.0, cells_per_m: 5.0, cooling_per_m: 0.2, lateral: 1.0, seed: 1 }
            .render(500, &pos, &mut out);
        Meteor {
            color: PixelColor::rgb(9, 9, 9),
            span_m: 2.0,
            speed_m_s: 1.0,
            tail_m: 0.4,
            sparkle: 0.2,
            seed: 1,
        }
        .render(500, &pos, &mut out);
        Ripple {
            color: PixelColor::rgb(9, 9, 9),
            center: Vec3::ZERO,
            speed_m_s: 1.0,
            wavelength_m: 0.5,
            falloff_per_m: 0.3,
        }
        .render(500, &pos, &mut out);
        // Chegar aqui sem pânico é o teste; e o pixel são continua correto.
        assert_eq!(out.len(), 3);
    }

    // ── Chase ─────────────────────────────────────────────────────────────────

    #[test]
    fn chase_head_is_full_brightness_and_tail_trails_behind() {
        let fx = Chase {
            color: PixelColor::rgb(255, 255, 255),
            speed_m_s: 1.0,
            spacing_m: 1.0,
            tail_frac: 0.5,
        };
        // t = 1 s ⇒ cabeça em x = 1.0 m. Pixels a cada 0.1 m.
        let pos = line(21, 0.1);
        let mut out = buf(21);
        fx.render(1000, &pos, &mut out);

        assert_eq!(out[10], PixelColor::rgb(255, 255, 255), "x=1.0 é a cabeça");
        // Cauda ATRÁS (x menor) acende; à frente (x maior, ainda dentro do ciclo) apaga.
        assert!(out[9].r > 0, "x=0.9 está na cauda");
        assert_eq!(out[11], BLACK, "x=1.1 está à frente da cabeça");
        // Brilho decresce ao afastar da cabeça.
        assert!(out[9].r > out[8].r && out[8].r > out[7].r, "cauda tem que decair");
    }

    #[test]
    fn chase_with_zero_tail_lights_only_the_head() {
        let fx = Chase {
            color: PixelColor::rgb(255, 0, 0),
            speed_m_s: 0.0,
            spacing_m: 1.0,
            tail_frac: 0.0,
        };
        let pos = line(10, 0.1);
        let mut out = buf(10);
        fx.render(0, &pos, &mut out);
        assert!(out.iter().all(|&c| c == BLACK), "cauda 0 ⇒ nada aceso fora da cabeça exata");
    }

    // ── Twinkle ───────────────────────────────────────────────────────────────

    #[test]
    fn twinkle_density_bounds_are_exact() {
        let pos = line(200, 0.01);
        let mut out = buf(200);

        // density = 0 ⇒ ninguém cintila: todos exatamente no brilho base.
        Twinkle {
            color: PixelColor::rgb(200, 200, 200),
            base: 0.5,
            density: 0.0,
            rate_hz: 3.0,
            seed: 1,
        }
        .render(1234, &pos, &mut out);
        let base_color = color::scale(PixelColor::rgb(200, 200, 200), 0.5);
        assert!(out.iter().all(|&c| c == base_color), "density=0 tem que ser uniforme");

        // density = 1 ⇒ todos cintilam, e num instante genérico o rig NÃO é uniforme
        // (cada pixel tem fase própria).
        Twinkle {
            color: PixelColor::rgb(200, 200, 200),
            base: 0.0,
            density: 1.0,
            rate_hz: 3.0,
            seed: 1,
        }
        .render(1234, &pos, &mut out);
        assert!(out.iter().any(|&c| c != out[0]), "density=1 sem fases próprias = piscar junto");
    }

    #[test]
    fn twinkle_roles_are_stable_across_frames() {
        // Um pixel não pode trocar de "cintila / não cintila" a cada quadro — isso seria
        // ruído, não cintilação. O papel vem do hash do índice, então é fixo.
        let pos = line(100, 0.01);
        let fx = Twinkle {
            color: PixelColor::rgb(255, 255, 255),
            base: 0.0,
            density: 0.25,
            rate_hz: 0.0, // sem avanço temporal: só o papel decide
            seed: 77,
        };
        let (mut a, mut b) = (buf(100), buf(100));
        fx.render(0, &pos, &mut a);
        fx.render(60_000, &pos, &mut b);
        let lit_a: Vec<bool> = a.iter().map(|c| c.r > 0).collect();
        let lit_b: Vec<bool> = b.iter().map(|c| c.r > 0).collect();
        assert_eq!(lit_a, lit_b, "o conjunto de pixels cintilantes tem que ser estável");
    }

    // ── Fire ──────────────────────────────────────────────────────────────────

    #[test]
    fn fire_palette_walks_black_red_yellow_white() {
        assert_eq!(heat_color(0.0), BLACK);
        assert_eq!(heat_color(1.0 / 3.0), PixelColor::rgb(255, 0, 0), "1/3 = vermelho pleno");
        assert_eq!(heat_color(2.0 / 3.0), PixelColor::rgb(255, 255, 0), "2/3 = amarelo");
        assert_eq!(heat_color(1.0), PixelColor::rgb(255, 255, 255), "topo = branco");
        // Fora de faixa satura em vez de dar a volta.
        assert_eq!(heat_color(-5.0), BLACK);
        assert_eq!(heat_color(5.0), PixelColor::rgb(255, 255, 255));
    }

    #[test]
    fn fire_cools_with_height() {
        // Com resfriamento forte, o topo tem que estar mais escuro que a base.
        let fx = Fire {
            speed_m_s: 0.0,
            cells_per_m: 4.0,
            cooling_per_m: 1.0,
            lateral: 0.0,
            seed: 3,
        };
        let pos: Vec<Vec3> = (0..20).map(|i| Vec3::new(0.0, i as f32 * 0.1, 0.0)).collect();
        let mut out = buf(20);
        fx.render(0, &pos, &mut out);
        let sum = |c: PixelColor| c.r as u32 + c.g as u32 + c.b as u32;
        let low: u32 = out[..5].iter().map(|&c| sum(c)).sum();
        let high: u32 = out[15..].iter().map(|&c| sum(c)).sum();
        assert!(low > high, "base ({low}) tem que ser mais quente que topo ({high})");
    }

    // ── ColorWash ─────────────────────────────────────────────────────────────

    #[test]
    fn colorwash_hits_both_endpoints_and_is_uniform() {
        let (a, b) = (PixelColor::rgb(255, 0, 0), PixelColor::rgb(0, 0, 255));
        let fx = ColorWash { a, b, hz: 1.0 };
        let pos = line(8, 0.1);
        let mut out = buf(8);

        fx.render(0, &pos, &mut out); // triangle(0) = 0 ⇒ cor a
        assert!(out.iter().all(|&c| c == a), "t=0 tem que ser a cor A pura");

        fx.render(500, &pos, &mut out); // triangle(0.5) = 1 ⇒ cor b
        assert!(out.iter().all(|&c| c == b), "meio-ciclo tem que ser a cor B pura");
    }

    // ── Strobe ────────────────────────────────────────────────────────────────

    #[test]
    fn strobe_duty_cycle_is_honoured() {
        let fx = Strobe { color: PixelColor::rgb(255, 255, 255), hz: 10.0, duty: 0.25 };
        let pos = line(4, 0.1);
        let mut out = buf(4);
        // Período de 100 ms, aceso nos primeiros 25 ms.
        fx.render(0, &pos, &mut out);
        assert_eq!(out[0], PixelColor::rgb(255, 255, 255), "início do período: aceso");
        fx.render(20, &pos, &mut out);
        assert_eq!(out[0], PixelColor::rgb(255, 255, 255), "20 ms < 25 ms: aceso");
        fx.render(30, &pos, &mut out);
        assert_eq!(out[0], BLACK, "30 ms > 25 ms: apagado");
    }

    /// A faixa de risco é consultável — é isso que permite a camada de cima avisar o
    /// operador em vez de o efeito decidir sozinho (precedente do ADR-0018).
    #[test]
    fn strobe_seizure_band_is_queryable_and_inclusive() {
        assert!(Strobe::is_in_seizure_risk_band(3.0), "limite inferior é inclusivo");
        assert!(Strobe::is_in_seizure_risk_band(60.0), "limite superior é inclusivo");
        assert!(Strobe::is_in_seizure_risk_band(15.0));
        assert!(!Strobe::is_in_seizure_risk_band(2.0));
        assert!(!Strobe::is_in_seizure_risk_band(120.0));
    }

    // ── Meteor ────────────────────────────────────────────────────────────────

    #[test]
    fn meteor_head_moves_and_wraps_at_the_span() {
        let fx = Meteor {
            color: PixelColor::rgb(255, 255, 255),
            span_m: 1.0,
            speed_m_s: 1.0,
            tail_m: 0.3,
            sparkle: 0.0, // sem esfarelamento: geometria pura
            seed: 1,
        };
        let pos = line(11, 0.1);
        let mut out = buf(11);

        fx.render(0, &pos, &mut out);
        assert_eq!(out[0], PixelColor::rgb(255, 255, 255), "t=0: cabeça em x=0");

        // t = 0.5 s ⇒ cabeça em x = 0.5.
        fx.render(500, &pos, &mut out);
        assert_eq!(out[5], PixelColor::rgb(255, 255, 255), "cabeça andou para x=0.5");
        assert_eq!(out[8], BLACK, "à frente da cabeça está apagado");

        // t = 1.0 s ⇒ deu a volta, cabeça de novo em x=0.
        fx.render(1000, &pos, &mut out);
        assert_eq!(out[0], PixelColor::rgb(255, 255, 255), "a volta é no span, não no infinito");
    }

    #[test]
    fn meteor_sparkle_dims_but_never_brightens() {
        let base = Meteor {
            color: PixelColor::rgb(255, 255, 255),
            span_m: 2.0,
            speed_m_s: 1.0,
            tail_m: 1.0,
            sparkle: 0.0,
            seed: 5,
        };
        let sparkled = Meteor { sparkle: 1.0, ..base };
        let pos = line(20, 0.1);
        let (mut a, mut b) = (buf(20), buf(20));
        base.render(700, &pos, &mut a);
        sparkled.render(700, &pos, &mut b);
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert!(y.r <= x.r, "sparkle clareou o pixel {i}: {} > {}", y.r, x.r);
        }
        assert!(a != b, "sparkle=1 tem que mudar alguma coisa");
    }

    // ── Lightning ─────────────────────────────────────────────────────────────

    #[test]
    fn lightning_never_strikes_at_zero_probability_and_always_at_one() {
        let pos = line(4, 0.1);
        let mut out = buf(4);

        let never = Lightning {
            color: PixelColor::rgb(255, 255, 255),
            window_ms: 100,
            probability: 0.0,
            decay_ms: 50,
            seed: 1,
        };
        for t in (0..2000).step_by(7) {
            never.render(t, &pos, &mut out);
            assert_eq!(out[0], BLACK, "probability=0 não pode acender em t={t}");
        }

        // probability = 1 ⇒ toda janela tem raio; o início de cada janela acende.
        let always = Lightning { probability: 1.0, ..never };
        for bucket in 0..10u64 {
            always.render(bucket * 100, &pos, &mut out);
            assert!(out[0].r > 0, "probability=1 tem que acender no início da janela {bucket}");
        }
    }

    #[test]
    fn lightning_is_not_periodic_like_a_pulse() {
        // O ponto do efeito: substituir o `Pulse` justamente porque relâmpago não é
        // periódico. Janelas diferentes têm que ter brilhos diferentes.
        let fx = Lightning {
            color: PixelColor::rgb(255, 255, 255),
            window_ms: 200,
            probability: 0.5,
            decay_ms: 80,
            seed: 42,
        };
        let pos = line(2, 0.1);
        let mut out = buf(2);
        let mut starts = Vec::new();
        for bucket in 0..40u64 {
            fx.render(bucket * 200, &pos, &mut out);
            starts.push(out[0].r);
        }
        let lit = starts.iter().filter(|&&v| v > 0).count();
        assert!((8..=32).contains(&lit), "com p=0.5, {lit}/40 janelas acesas é implausível");
        let distinct: std::collections::BTreeSet<u8> = starts.iter().copied().collect();
        assert!(distinct.len() > 3, "brilho de raio não pode ser sempre o mesmo valor");
    }

    // ── Ripple ────────────────────────────────────────────────────────────────

    #[test]
    fn ripple_attenuates_with_distance_from_the_centre() {
        // Para medir SÓ a atenuação é preciso comparar pixels na MESMA fase da onda —
        // senão o termo da crista varia junto e a comparação não diz nada. Distâncias
        // separadas por um comprimento de onda exato têm fase idêntica; a única coisa que
        // difere entre elas é a distância.
        let fx = Ripple {
            color: PixelColor::rgb(255, 255, 255),
            center: Vec3::ZERO,
            speed_m_s: 0.0, // congelado: só a geometria importa
            wavelength_m: 1.0,
            falloff_per_m: 1.0,
        };
        // 0.5, 1.5, 2.5 m ⇒ fase 0.5 nas três (crista máxima), distâncias diferentes.
        let pos = vec![
            Vec3::new(0.5, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(2.5, 0.0, 0.0),
        ];
        let mut out = buf(3);
        fx.render(0, &pos, &mut out);
        assert!(
            out[0].r > out[1].r && out[1].r > out[2].r,
            "mesma fase, distâncias crescentes ⇒ brilho estritamente decrescente: {:?}",
            out.iter().map(|c| c.r).collect::<Vec<_>>()
        );

        // Controle: sem atenuação, os mesmos três pixels ficam idênticos.
        let flat = Ripple { falloff_per_m: 0.0, ..fx };
        flat.render(0, &pos, &mut out);
        assert_eq!(out[0], out[1], "falloff=0 ⇒ a distância não pode mudar o brilho");
        assert_eq!(out[1], out[2]);
    }

    #[test]
    fn ripple_uses_all_three_axes() {
        // Distâncias iguais em eixos diferentes ⇒ mesma cor. É o que prova que o efeito
        // usa distância 3-D e não só x.
        let fx = Ripple {
            color: PixelColor::rgb(255, 255, 255),
            center: Vec3::ZERO,
            speed_m_s: 1.0,
            wavelength_m: 0.7,
            falloff_per_m: 0.4,
        };
        let pos = vec![
            Vec3::new(0.6, 0.0, 0.0),
            Vec3::new(0.0, 0.6, 0.0),
            Vec3::new(0.0, 0.0, 0.6),
        ];
        let mut out = buf(3);
        fx.render(333, &pos, &mut out);
        assert_eq!(out[0], out[1], "x e y equidistantes têm que dar a mesma cor");
        assert_eq!(out[1], out[2], "z tem que contar igual");
    }
}
