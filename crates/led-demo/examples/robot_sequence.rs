//! The user's OWN show design, played by LUMYX.
//!
//! Reads the real xLights sequence (`__.xsq`: Lightning → Life → Meteors
//! cascading across the 5 robots, grand finale on all of them) and the real
//! layout (FIXED), maps each xLights effect to a LUMYX effect, renders the
//! full show, records it, verifies the replay, and writes a GIF excerpt.
//!
//! ```text
//! cargo run --release -p led-demo --example robot_sequence -- "<show dir>"
//! ```
//!
//! Effect mapping (timing/targeting was always exact; the *look* is now native):
//! | xLights   | LUMYX                        | antes desta rodada          |
//! |-----------|------------------------------|------------------------------|
//! | Lightning | `Lightning` (clarões aperiódicos) | `Pulse` 6 Hz (periódico) |
//! | Life      | `Plasma` (compute kernel)    | igual — já era adequado      |
//! | Meteors   | `Meteor` (cabeça + cauda)    | `Rainbow` (varredura de matiz) |
//!
//! As duas substituições fecham aproximações que estavam **documentadas como tais**: um
//! relâmpago não é periódico (o `Pulse` era), e um cometa tem cabeça e cauda (o `Rainbow`
//! não tinha nenhuma das duas).

use std::io::Cursor;
use std::path::Path;

use led_core::{CompiledLayout, LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{Hal, SimulatorDevice};
use led_pixel_engine::{ComputeEffect, Effect, Lightning, Meteor, Plasma, Pulse, Vec3};
use led_show_recorder::replay::ReplayManifest;
use led_show_recorder::{finalise_seekable, ShowReader, ShowRecord, ShowWriter};
use led_xlights::{import_strings, parse_sequence_file};

const FPS: u64 = 40;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/gabrielabambam/Desktop/meu show robô".into());
    let base = Path::new(&dir);

    // ── Layout (gated) + sequence (the user's design) ─────────────────────
    let networks = std::fs::read_to_string(base.join("xlights_networks.xml")).expect("networks");
    let rgbeffects = std::fs::read_to_string(base.join("xlights_rgbeffects.LUMYX-FIXED.xml"))
        .expect("run led-xlights --fix first");
    let report = import_strings(&networks, &rgbeffects);
    let assigns = report.assignments().expect("gate");
    let seq = parse_sequence_file(&base.join("__.xsq")).expect("__.xsq");
    println!("layout: {} px | sequence: {} spans over {}ms",
        assigns.len(), seq.spans.len(), seq.duration_ms);

    let positions: Vec<Vec3> = report
        .models
        .iter()
        .flat_map(|m| m.pixel_positions())
        .map(|(x, y, z)| Vec3::new(x, y, z))
        .collect();
    let px = positions.len();

    // Extensão real do rig no eixo x. O EFEITO não conhece o tamanho do rig (regra 3 do
    // ADR-0021) — quem monta o show conhece, e é aqui que ele é medido, uma vez, do layout.
    let x_span = {
        let (lo, hi) = positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
        (hi - lo).max(1.0)
    };
    println!("rig: extensão em x = {x_span:.1} (unidades do layout xLights)");

    // Resolve each span's target pixels once (groups → ranges).
    struct ActiveSpan {
        start_ms: u64,
        end_ms: u64,
        effect: Box<dyn Effect>,
        ranges: Vec<std::ops::Range<usize>>,
    }
    let spans: Vec<ActiveSpan> = seq
        .spans
        .iter()
        .filter_map(|s| {
            let ranges = report.pixels_for_group(&s.element);
            if ranges.is_empty() {
                eprintln!("  (skip '{}' — no pixels for element '{}')", s.effect, s.element);
                return None;
            }
            let effect: Box<dyn Effect> = match s.effect.as_str() {
                // Clarões aperiódicos: ~2,8 por segundo de oportunidade, 55 % efetivam,
                // queda de 70 ms. O `seed` fixo mantém o show reproduzível quadro a quadro.
                "Lightning" => Box::new(Lightning {
                    color: PixelColor::rgb(255, 255, 255),
                    window_ms: 360,
                    probability: 0.55,
                    decay_ms: 70,
                    seed: 0x116E_1146, // fixo: o show tem que replayar igual
                }),
                "Life" => Box::new(ComputeEffect::new(Plasma { scale: 0.008, speed: 0.6 })),
                // Cauda de 12 % do rig, 2 voltas por show-span; o esfarelamento é o que
                // distingue um cometa de um simples pulso deslizante.
                "Meteors" => Box::new(Meteor {
                    color: PixelColor::rgb(120, 200, 255),
                    span_m: x_span,
                    speed_m_s: x_span / 4.0,
                    tail_m: x_span * 0.12,
                    sparkle: 0.3,
                    seed: 0x3E7E_0800,   // fixo: idem
                }),
                _ => Box::new(Pulse { color: PixelColor::rgb(128, 0, 255), hz: 1.0 }),
            };
            Some(ActiveSpan { start_ms: s.start_ms, end_ms: s.end_ms, effect, ranges })
        })
        .collect();
    println!("mapped: {} spans onto pixel ranges", spans.len());

    // ── Output: the 5 simulated controllers ───────────────────────────────
    let layout = CompiledLayout::compile(&assigns);
    let sims: Vec<_> = (0..report.controllers.len() as u16)
        .map(|id| SimulatorDevice::new(id, layout.device_universes(id)))
        .collect();
    let hal = Hal::new(
        CompiledLayout::compile(&assigns),
        sims.iter().map(|s| s.clone() as _).collect(),
    );

    // ── Render the whole show ──────────────────────────────────────────────
    let frame_ms = 1000 / FPS;
    let frames = seq.duration_ms / frame_ms;
    let mut backing = Cursor::new(Vec::<u8>::new());
    let mut records: Vec<ShowRecord> = Vec::with_capacity(frames as usize);
    {
        let mut writer = ShowWriter::new(&mut backing, px as u32).expect("writer");
        let mut buf = vec![PixelColor::default(); px];
        let mut sub_buf: Vec<PixelColor> = Vec::new();
        let mut sub_pos: Vec<Vec3> = Vec::new();

        for f in 0..frames {
            let t = f * frame_ms;
            buf.fill(PixelColor::default());

            for span in &spans {
                if t < span.start_ms || t >= span.end_ms {
                    continue;
                }
                let local_t = t - span.start_ms;
                for r in &span.ranges {
                    // Render the effect on the subset, then scatter back.
                    sub_pos.clear();
                    sub_pos.extend_from_slice(&positions[r.clone()]);
                    sub_buf.clear();
                    sub_buf.resize(r.len(), PixelColor::default());
                    span.effect.render(local_t, &sub_pos, &mut sub_buf);
                    buf[r.clone()].copy_from_slice(&sub_buf);
                }
            }

            hal.send_frame(&LogicalFrame::new(buf.clone(), t)).expect("send");
            let rec = ShowRecord { timestamp_ms: t, pixels: buf.clone(), audio: None };
            writer.write_frame(&rec).expect("record");
            records.push(rec);
        }
        finalise_seekable(&mut writer).expect("finalise");
    }
    println!("rendered: {} frames × {} px", frames, px);

    // ── Persist + verify ───────────────────────────────────────────────────
    let data = backing.into_inner();
    std::fs::write("robot_sequence.lumyx", &data).expect("write");
    let manifest = ReplayManifest::from_records(&records);
    let replayed = ShowReader::new(Cursor::new(data)).unwrap().collect_all().unwrap();
    assert_eq!(
        ReplayManifest::from_records(&replayed).aggregate_hash,
        manifest.aggregate_hash,
        "replay must match"
    );
    println!("replay: VERIFIED — {:#018x} → robot_sequence.lumyx", manifest.aggregate_hash);

    // O hash do show gravado é PARÂMETRO DE PALCO, não só um número de log:
    // `docs/runbooks/show-startup.md` manda o operador rodar
    //     led-player robot_sequence.lumyx --verify <hash>
    // com um hash fixo. Regerar o show muda esse hash e o runbook para de bater — em
    // silêncio, na hora errada. Este aviso existe para que a atualização seja consciente.
    const RUNBOOK_HASH: u64 = 0xd8f1_479f_f364_5e1e;
    if manifest.aggregate_hash != RUNBOOK_HASH {
        println!(
            "\n⚠  ATENÇÃO — o hash mudou: runbook {RUNBOOK_HASH:#018x} → agora {:#018x}\n\
             ⚠  Atualize `docs/runbooks/show-startup.md` (--verify) ANTES do próximo show,\n\
             ⚠  ou o gate de integridade vai reprovar um show que na verdade está correto.",
            manifest.aggregate_hash
        );
    }

    // ── GIF excerpt: the Lightning cascade (0–31s) at 10 fps ─────────────
    let excerpt: Vec<&ShowRecord> = records
        .iter()
        .filter(|r| r.timestamp_ms < 31_000 && r.timestamp_ms % 100 == 0)
        .collect();
    write_gif(&excerpt, &positions, "robot_sequence.gif");
    println!("preview: robot_sequence.gif ({} frames — Lightning cascade r1→r5)", excerpt.len());
}

fn write_gif(records: &[&ShowRecord], positions: &[Vec3], path: &str) {
    const W: u16 = 640;
    const H: u16 = 360;
    let (min_x, max_x) = positions.iter().map(|p| p.x).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let (min_y, max_y) = positions.iter().map(|p| p.y).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let scale = ((W as f32 - 20.0) / (max_x - min_x).max(1.0))
        .min((H as f32 - 20.0) / (max_y - min_y).max(1.0));

    let file = std::fs::File::create(path).expect("gif");
    let mut enc = gif::Encoder::new(file, W, H, &[]).expect("enc");
    enc.set_repeat(gif::Repeat::Infinite).expect("repeat");
    for rec in records {
        let mut rgb = vec![0u8; W as usize * H as usize * 3];
        for (pos, px) in positions.iter().zip(&rec.pixels) {
            let x = (10.0 + (pos.x - min_x) * scale) as usize;
            let y = (H as f32 - 10.0 - (pos.y - min_y) * scale) as usize;
            for dy in 0..2usize {
                for dx in 0..2usize {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < W as usize && yy < H as usize {
                        let i = (yy * W as usize + xx) * 3;
                        rgb[i] = px.r;
                        rgb[i + 1] = px.g;
                        rgb[i + 2] = px.b;
                    }
                }
            }
        }
        let mut frame = gif::Frame::from_rgb(W, H, &rgb);
        frame.delay = 10; // 100ms → 10 fps
        enc.write_frame(&frame).expect("frame");
    }
}
