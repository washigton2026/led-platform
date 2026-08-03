//! Deterministic pseudo-randomness for effects — **stateless by construction**.
//!
//! ## Por que não um PRNG
//!
//! [`Effect::render`](crate::effect::Effect::render) recebe `&self`. Um efeito **não pode**
//! guardar estado, e isso não é acidente: é o que torna o render determinístico e replayável
//! (`mesmo tempo ⇒ mesmo frame`). Um `Rng` com estado avançaria a cada chamada, e dois
//! renders do mesmo `time_ms` divergiriam — quebrando o replay verificado por hash que é
//! uma das garantias centrais do LUMYX.
//!
//! Logo: aleatoriedade em efeitos é **hash de coordenadas**, nunca um fluxo.
//! `aleatório(pixel, tempo)` em vez de `próximo()`.
//!
//! ## Relação com o SplitMix64 do workspace
//!
//! [`mix64`] é o **finalizador** do SplitMix64 — as mesmas duas rodadas xor-shift-multiply
//! usadas em `led_sequencer::show_intent` e `led_hal::chaos`. A diferença é essencial e é o
//! motivo de isto não ser uma terceira cópia da mesma coisa: **aqueles são geradores com
//! estado** (`fn(&mut u64) -> u64`, avançam um fluxo); este é uma **função pura**
//! (`fn(u64) -> u64`, sem estado). Compartilham as constantes porque o finalizador do
//! SplitMix64 é um misturador de boa qualidade e já testado — não porque façam o mesmo.
//!
//! ## NaN
//!
//! Posições vêm de dados do usuário e **podem ser NaN** — este repo já pagou por isso uma
//! vez (BUG-3: `smoothstep(NaN)` propagou NaN até a posição de drones, achado CRITICAL de
//! segurança). Toda função aqui que aceita `f32` devolve um valor finito para **qualquer**
//! entrada, inclusive `NaN` e `±∞`, e há teste provando isso.

/// Misturador 64-bit **sem estado**: o finalizador do SplitMix64.
///
/// Avalanche boa o bastante para índices sequenciais (`0, 1, 2, …`) produzirem saídas
/// descorrelacionadas — que é exatamente o caso de uso em efeitos, onde a "semente" é o
/// índice do pixel.
#[inline]
pub const fn mix64(x: u64) -> u64 {
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// Valor determinístico em `[0, 1)` para o par `(key, seed)`.
///
/// Usa os 24 bits altos do hash: 24 bits são exatamente representáveis em `f32`, então a
/// conversão não perde nem inventa precisão.
#[inline]
pub fn hash01(key: u64, seed: u64) -> f32 {
    const SCALE: f32 = 1.0 / (1u32 << 24) as f32;
    (mix64(key ^ mix64(seed)) >> 40) as f32 * SCALE
}

/// Ruído de valor 1-D, suave: interpola pontos de rede com `smoothstep`.
///
/// `x` está em **unidades de rede** — um passo inteiro de `x` é um ponto novo. Multiplique
/// a coordenada pela frequência espacial desejada antes de chamar.
///
/// Devolve `[0, 1]`. Para `x` não-finito devolve `0.5` (o valor neutro do intervalo) em vez
/// de propagar `NaN` — ver a nota de NaN no topo do módulo.
#[inline]
pub fn value_noise(x: f32, seed: u64) -> f32 {
    if !x.is_finite() {
        return 0.5;
    }
    let i = x.floor();
    let f = x - i;
    // `i` cabe em i64 porque já sabemos que x é finito; o wrap para u64 é determinístico.
    let cell = i as i64 as u64;
    let a = hash01(cell, seed);
    let b = hash01(cell.wrapping_add(1), seed);
    let u = f * f * (3.0 - 2.0 * f); // smoothstep — derivada zero nas bordas, sem "costura"
    a + (b - a) * u
}

/// Ruído fractal: soma oitavas de [`value_noise`] com amplitude decrescente.
///
/// É o que dá a textura irregular de fogo/fumaça sem armazenar mapa de calor nenhum.
/// Devolve `[0, 1]`.
#[inline]
pub fn fbm(x: f32, octaves: u32, seed: u64) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves.max(1) {
        sum += value_noise(x * freq, seed ^ (o as u64).wrapping_mul(0x9e37_79b9)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix64_decorrelates_sequential_keys() {
        // O caso de uso real: índices de pixel 0,1,2,… precisam parecer independentes.
        // Se o misturador fosse fraco, os bits altos de chaves vizinhas ficariam colados.
        let vals: Vec<f32> = (0..64).map(|i| hash01(i, 7)).collect();
        assert!(vals.iter().all(|v| (0.0..1.0).contains(v)), "sempre em [0,1)");

        // Metade acima, metade abaixo de 0.5 — dentro de uma folga generosa. Um gerador
        // quebrado (ex.: identidade) falharia isto de forma óbvia.
        let above = vals.iter().filter(|&&v| v >= 0.5).count();
        assert!((16..=48).contains(&above), "distribuição enviesada: {above}/64 acima de 0.5");
    }

    #[test]
    fn hash01_is_pure() {
        assert_eq!(hash01(42, 1), hash01(42, 1), "mesma entrada ⇒ mesma saída, sempre");
        assert_ne!(hash01(42, 1), hash01(42, 2), "a semente separa fluxos");
        assert_ne!(hash01(42, 1), hash01(43, 1), "a chave separa pixels");
    }

    #[test]
    fn value_noise_is_continuous_across_a_lattice_boundary() {
        // Uma costura visível na rede seria um artefato óptico real na fita. Amostramos
        // ao redor de um inteiro: o salto tem que ser pequeno.
        let left = value_noise(2.0 - 1e-4, 9);
        let right = value_noise(2.0 + 1e-4, 9);
        assert!((left - right).abs() < 1e-3, "costura em x=2: {left} vs {right}");
    }

    #[test]
    fn value_noise_stays_in_range() {
        for i in 0..500 {
            let v = value_noise(i as f32 * 0.37 - 90.0, 3);
            assert!((0.0..=1.0).contains(&v), "fora de faixa em i={i}: {v}");
        }
    }

    /// Controle negativo — a classe de falha que este repo já pagou uma vez (BUG-3:
    /// `smoothstep(NaN)` propagou NaN até a posição de drones). Entrada suja **nunca** pode
    /// sair como NaN de uma função de ruído.
    #[test]
    fn negative_control_non_finite_input_never_produces_nan() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(value_noise(x, 1).is_finite(), "value_noise({x}) não é finito");
            assert!(fbm(x, 4, 1).is_finite(), "fbm({x}) não é finito");
        }
    }

    #[test]
    fn fbm_stays_in_range_and_adds_detail() {
        for i in 0..200 {
            let v = fbm(i as f32 * 0.13, 4, 5);
            assert!((0.0..=1.0).contains(&v), "fbm fora de faixa: {v}");
        }
        // Mais oitavas ⇒ o sinal muda (detalhe novo), não é uma cópia da primeira oitava.
        assert_ne!(fbm(3.7, 1, 5), fbm(3.7, 4, 5));
    }
}
