//! Protótipo Leptos (CSR) para o spike de stack (ADR-0016). Mesma tela do protótipo React.
//!
//! ⚠️ NÃO compilado no ambiente do agente (faltam trunk + target wasm32). Rode você:
//!   rustup target add wasm32-unknown-unknown && cargo install trunk && trunk serve
//! Pequenos ajustes de API do Leptos 0.6 podem ser necessários ao rodar — isso é trabalho
//! normal de spike; nada aqui foi "medido" nem carimbado pelo agente.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[derive(Clone, Copy, PartialEq)]
enum Health {
    Ok,
    Warning,
    Critical,
}

/// Status colour-blind-safe: símbolo + palavra + cor (nunca só cor).
fn status(h: Health) -> (&'static str, &'static str, &'static str) {
    match h {
        Health::Ok => ("✓", "OK", "#2e7d32"),
        Health::Warning => ("▲", "Atenção", "#b26a00"),
        Health::Critical => ("■", "Crítico", "#c62828"),
    }
}

// Fixture com o shape real de crates/led-readmodel: (id, connected, frames_sent, last_send_ms).
const DEVICES: &[(u16, bool, u64, u64)] = &[(0, true, 42, 0), (1, true, 41, 800)];

#[component]
fn App() -> impl IntoView {
    let (health, set_health) = create_signal(Health::Ok);
    let cycle = move |_| {
        set_health.update(|h| {
            *h = match *h {
                Health::Ok => Health::Warning,
                Health::Warning => Health::Critical,
                Health::Critical => Health::Ok,
            }
        })
    };

    view! {
        <main style="max-width:760px;margin:2rem auto;padding:0 1rem;color:#eee;background:#111;font-family:system-ui,sans-serif;line-height:1.5">
            <h1>"LUMYX — Console (spike Leptos)"</h1>

            // Live region: o leitor de tela deve anunciar a mudança de status.
            <div role="status" aria-live="polite"
                 style=move || format!(
                     "display:flex;gap:10px;align-items:center;padding:10px 14px;border:2px solid {};border-radius:8px;margin:12px 0",
                     status(health.get()).2)>
                <span aria-hidden="true"
                      style=move || format!("font-size:22px;color:{}", status(health.get()).2)>
                    {move || status(health.get()).0}
                </span>
                <strong>{move || format!("Saúde do output: {}", status(health.get()).1)}</strong>
            </div>

            <button on:click=cycle
                    style="padding:8px 14px;font-size:15px;cursor:pointer;margin:4px 0 16px">
                "Ciclar status (testa o anúncio do leitor de tela)"
            </button>

            <section aria-labelledby="dev-h">
                <h2 id="dev-h">"Controladores"</h2>
                <table style="width:100%;border-collapse:collapse">
                    <caption style="text-align:left;font-style:italic;margin-bottom:6px">"Status por controlador"</caption>
                    <thead>
                        <tr>
                            <th scope="col">"ID"</th>
                            <th scope="col">"Conectado"</th>
                            <th scope="col">"Frames"</th>
                            <th scope="col">"Último envio (ms)"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {DEVICES.iter().map(|&(id, conn, frames, last)| view! {
                            <tr>
                                <td>{id}</td>
                                <td>{if conn { "sim" } else { "não" }}</td>
                                <td>{frames}</td>
                                <td>{last}</td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </section>

            <section aria-labelledby="pv-h">
                <h2 id="pv-h">"Preview"</h2>
                <Preview/>
            </section>
        </main>
    }
}

#[component]
fn Preview() -> impl IntoView {
    let canvas_ref = create_node_ref::<html::Canvas>();

    canvas_ref.on_load(move |canvas| {
        let canvas: web_sys::HtmlCanvasElement = canvas.into();
        canvas.set_width(640);
        canvas.set_height(240);
        let ctx = canvas
            .get_context("2d").unwrap().unwrap()
            .dyn_into::<web_sys::CanvasRenderingContext2d>().unwrap();

        // 10k pontos animados. (A verbosidade do loop RAF em Rust/wasm é dado do eixo DX.)
        let mut pts: Vec<(f64, f64, f64, f64)> = (0..10_000u64)
            .map(|i| {
                let r = |k: u64| (i.wrapping_mul(k).wrapping_add(k) % 997) as f64 / 997.0;
                (r(31), r(57), (r(13) - 0.5) * 0.004, (r(17) - 0.5) * 0.004)
            })
            .collect();

        let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
        let g = f.clone();
        *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            let (w, h) = (640.0, 240.0);
            ctx.set_fill_style(&JsValue::from_str("#0b0b0b"));
            ctx.fill_rect(0.0, 0.0, w, h);
            ctx.set_fill_style(&JsValue::from_str("#4fc3f7"));
            for p in pts.iter_mut() {
                p.0 += p.2;
                p.1 += p.3;
                if p.0 < 0.0 || p.0 > 1.0 { p.2 = -p.2; }
                if p.1 < 0.0 || p.1 > 1.0 { p.3 = -p.3; }
                ctx.fill_rect(p.0 * w, p.1 * h, 2.0, 2.0);
            }
            raf(f.borrow().as_ref().unwrap());
        }) as Box<dyn FnMut()>));
        raf(g.borrow().as_ref().unwrap());
    });

    view! {
        <p>"WebGPU: verifique navigator.gpu no browser · preview 2D de 10k pontos"</p>
        <canvas node_ref=canvas_ref role="img" aria-label="Preview animado de 10 mil pixels"
                style="width:100%;max-width:640px;border:1px solid #444;background:#0b0b0b"></canvas>
    }
}

fn raf(f: &Closure<dyn FnMut()>) {
    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .unwrap();
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
