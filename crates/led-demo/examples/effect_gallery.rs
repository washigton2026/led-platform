//! Galeria visual da biblioteca de efeitos (ADR-0021).
//!
//! Efeito é coisa que se **vê**. Teste de unidade prova geometria, monotonia, pureza e
//! ausência de alocação — nenhum deles prova que fica bonito na fita. Este exemplo renderiza
//! cada efeito numa faixa horizontal, empilhadas, e escreve `effect_gallery.gif`.
//!
//! ```text
//! cargo run --release -p led-demo --example effect_gallery
//! ```
//!
//! **Não toca em nenhum artefato de show.** `robot_sequence.lumyx` / `show.lumyx` e seus
//! hashes de verificação ficam intocados — este exemplo só escreve o próprio GIF.

use std::fs::File;

use led_core::PixelColor;
use led_pixel_engine::{
    Chase, ColorWash, ComputeEffect, Effect, Fire, Lightning, Meteor, Plasma, Pulse, Rainbow,
    Ripple, SolidColor, Strobe, Twinkle, Vec3,
};

/// Pixels por faixa (uma "fita" horizontal por efeito).
const STRIP: usize = 160;
/// Altura em pixels de imagem de cada faixa.
const ROW_H: usize = 14;
/// Escala de mundo: a faixa inteira mede 4 m — números por-metro ficam legíveis.
const SPAN_M: f32 = 4.0;
const FPS: u64 = 25;
const SECONDS: u64 = 8;

fn main() {
    let step = SPAN_M / STRIP as f32;
    // Faixa reta em x; y varia levemente para o `Fire` (que sobe em y) ter o que subir.
    let positions: Vec<Vec3> = (0..STRIP).map(|i| Vec3::new(i as f32 * step, 0.0, 0.0)).collect();
    // O Fire precisa de altura: uma segunda geometria, vertical, só para ele.
    let vertical: Vec<Vec3> = (0..STRIP).map(|i| Vec3::new(0.0, i as f32 * step, 0.0)).collect();

    let white = PixelColor::rgb(255, 255, 255);
    let entries: Vec<(&str, Box<dyn Effect>, bool)> = vec![
        ("SolidColor", Box::new(SolidColor(PixelColor::rgb(0, 90, 160))), false),
        ("Rainbow", Box::new(Rainbow { speed_hz: 0.25, cycles_per_m: 0.5 }), false),
        ("Pulse", Box::new(Pulse { color: PixelColor::rgb(255, 40, 0), hz: 0.8 }), false),
        (
            "Chase",
            Box::new(Chase { color: white, speed_m_s: 1.5, spacing_m: 1.0, tail_frac: 0.55 }),
            false,
        ),
        (
            "Twinkle",
            Box::new(Twinkle {
                color: PixelColor::rgb(255, 240, 200),
                base: 0.06,
                density: 0.28,
                rate_hz: 1.6,
                seed: 0x00C0_FFEE,
            }),
            false,
        ),
        (
            "Fire",
            Box::new(Fire {
                speed_m_s: 1.1,
                cells_per_m: 5.0,
                cooling_per_m: 0.22,
                lateral: 0.0,
                seed: 0x0000_F12E,
            }),
            true, // usa a geometria vertical
        ),
        (
            "ColorWash",
            Box::new(ColorWash {
                a: PixelColor::rgb(200, 0, 80),
                b: PixelColor::rgb(0, 120, 200),
                hz: 0.2,
            }),
            false,
        ),
        ("Strobe", Box::new(Strobe { color: white, hz: 2.5, duty: 0.18 }), false),
        (
            "Meteor",
            Box::new(Meteor {
                color: PixelColor::rgb(120, 210, 255),
                span_m: SPAN_M,
                speed_m_s: 1.2,
                tail_m: 0.9,
                sparkle: 0.3,
                seed: 0x00C0_7EA1,
            }),
            false,
        ),
        (
            "Lightning",
            Box::new(Lightning {
                color: white,
                window_ms: 420,
                probability: 0.5,
                decay_ms: 90,
                seed: 0x0000_B017,
            }),
            false,
        ),
        (
            "Ripple",
            Box::new(Ripple {
                color: PixelColor::rgb(60, 255, 180),
                center: Vec3::new(SPAN_M / 2.0, 0.0, 0.0),
                speed_m_s: 0.9,
                wavelength_m: 0.7,
                falloff_per_m: 0.35,
            }),
            false,
        ),
        (
            "Plasma",
            Box::new(ComputeEffect::new(Plasma { scale: 3.0, speed: 0.7 })),
            false,
        ),
    ];

    let img_w = STRIP as u16;
    let img_h = (entries.len() * ROW_H) as u16;
    let mut file = File::create("effect_gallery.gif").expect("create effect_gallery.gif");
    let mut enc = gif::Encoder::new(&mut file, img_w, img_h, &[]).expect("gif encoder");
    enc.set_repeat(gif::Repeat::Infinite).expect("repeat");

    let frame_ms = 1000 / FPS;
    let frames = SECONDS * FPS;
    let mut strip = vec![PixelColor::default(); STRIP];
    let mut rgb = vec![0u8; img_w as usize * img_h as usize * 3];

    for f in 0..frames {
        let t = f * frame_ms;
        for (row, (_, fx, vert)) in entries.iter().enumerate() {
            let pos = if *vert { &vertical } else { &positions };
            fx.render(t, pos, &mut strip);
            for y in 0..ROW_H {
                // 1 px de separação escura entre faixas para a galeria ficar legível.
                let dim = y == ROW_H - 1;
                for (x, c) in strip.iter().enumerate() {
                    let o = ((row * ROW_H + y) * img_w as usize + x) * 3;
                    let (r, g, b) = if dim { (0, 0, 0) } else { (c.r, c.g, c.b) };
                    rgb[o] = r;
                    rgb[o + 1] = g;
                    rgb[o + 2] = b;
                }
            }
        }
        let mut gf = gif::Frame::from_rgb_speed(img_w, img_h, &rgb, 10);
        gf.delay = (100 / FPS) as u16;
        enc.write_frame(&gf).expect("write frame");
    }

    println!("effect_gallery.gif — {} efeitos × {frames} frames ({STRIP} px/faixa, {SPAN_M} m)", entries.len());
    for (i, (name, _, vert)) in entries.iter().enumerate() {
        println!("  faixa {:>2}: {name}{}", i + 1, if *vert { "  (geometria vertical)" } else { "" });
    }
}
