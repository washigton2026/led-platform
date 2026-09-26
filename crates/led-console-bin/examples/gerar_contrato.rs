//! Regenera o contrato TypeScript a partir do Rust (ADR-0027).
//!
//! ```sh
//! cargo run -p led-console-bin --example gerar_contrato
//! ```
//!
//! É a **única** forma legítima de o ficheiro mudar. O gate
//! `tests/contract_gate.rs` reprova qualquer edição manual.

fn main() -> std::io::Result<()> {
    let destino = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contract")
        .join("lumyx-contract.generated.ts");
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ts = led_console_bin::contract::gerar_typescript();
    std::fs::write(&destino, &ts)?;
    println!("contrato escrito: {} ({} bytes)", destino.display(), ts.len());
    Ok(())
}
