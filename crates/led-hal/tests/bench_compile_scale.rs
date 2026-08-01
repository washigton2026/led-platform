//! M6 — medição do custo de `CompiledLayout::compile` em escala (achado da FASE 1).
//!
//! A revisão constitucional apontou `led-core/src/mapping.rs` como **candidato a O(n²)**:
//! para cada atribuição, `compile` faz uma busca linear no vetor de devices
//! (`per_device.iter().position(...)`) e outra na lista de universos daquele device
//! (`.contains(&a.universe)`). A segunda cresce com o número de universos, então o custo
//! tende a `O(n × universos)` — e, com um device único, `universos ≈ n / pixels_por_universo`,
//! o que dá crescimento **quadrático em n**.
//!
//! Isto é **medição, não correção**: nenhuma lógica de produção é alterada. O `compile` roda
//! **uma vez, no startup** — nunca no hot path —, então a pergunta que importa não é "é
//! quadrático?" mas "**quanto custa no maior rig plausível?**".
//!
//! Rode com:
//!   cargo test -p led-hal --test bench_compile_scale -- --ignored --nocapture
//!
//! Honestidade de ambiente: medido na máquina que roda a suíte (dev macOS, build debug), não
//! na appliance de produção. Serve para o **formato do crescimento** e a ordem de grandeza.

use std::time::Instant;

use led_hal::*;

/// Compila `n` pixels num único device com 170 px/universo e devolve o tempo em ms.
fn compile_ms(n: usize) -> f64 {
    let universes = n.div_ceil(170) as u16;
    let specs = [DeviceSpec { id: 0, universes }];
    let t = Instant::now();
    let layout = CompiledLayout::linear(n, &specs, RgbOrder::Grb);
    let elapsed = t.elapsed().as_secs_f64() * 1000.0;
    // Impede que o otimizador descarte o trabalho.
    assert!(layout.universe_count() > 0);
    elapsed
}

#[test]
#[ignore = "measurement bench (wall-clock) — run manually with --ignored --nocapture"]
fn bench_compiled_layout_scaling() {
    // Aquecimento (aloca/aquece o allocator antes de medir).
    let _ = compile_ms(1_000);

    let sizes = [1_000usize, 6_200, 25_000, 50_000, 100_000];
    let mut prev: Option<(usize, f64)> = None;

    eprintln!("── M6 · CompiledLayout::compile em escala (1 device, 170 px/universo) ──");
    eprintln!("  {:>8}  {:>10}  {:>8}  {:>26}", "pixels", "ms", "×n", "×tempo (2× = linear)");
    for n in sizes {
        let ms = compile_ms(n);
        match prev {
            None => eprintln!("  {n:>8}  {ms:>10.2}  {:>8}  {:>26}", "—", "—"),
            Some((pn, pms)) => {
                let n_ratio = n as f64 / pn as f64;
                let t_ratio = ms / pms.max(f64::MIN_POSITIVE);
                eprintln!("  {n:>8}  {ms:>10.2}  {n_ratio:>8.1}  {t_ratio:>26.1}");
            }
        }
        prev = Some((n, ms));
    }
    eprintln!("  Leitura: se ×tempo ≈ ×n, o custo é linear; se ≈ (×n)², é quadrático.");
    eprintln!("  Contexto: `compile` roda UMA vez no startup, nunca no hot path.");
    eprintln!("  Rig real do usuário: 6.200 px (5 robôs).");

    // Sanidade apenas — o entregável é a medição impressa, não um limiar.
    assert!(compile_ms(1_000) >= 0.0);
}

/// Guarda de regressão **falsificável** para o tamanho do rig real: o `compile` do rig de
/// 6.200 px precisa caber com folga no startup. Limite generoso (debug é ~10× mais lento que
/// release) — se um dia isto falhar, a otimização deixou de ser opcional.
#[test]
fn compiling_the_real_rig_stays_well_under_a_second() {
    let ms = compile_ms(6_200);
    assert!(
        ms < 1_000.0,
        "compile de 6.200 px levou {ms:.1} ms — o rig real não pode custar ~1 s de startup"
    );
}
