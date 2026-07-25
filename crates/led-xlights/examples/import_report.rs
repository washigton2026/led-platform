//! Import an xLights show folder and print the report + conflict gate result.
//!
//! ```text
//! cargo run -p led-xlights --example import_report -- "/path/to/show folder"
//! ```

use led_xlights::{apply_fixes_to_xml, import_show_dir, import_strings};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let do_fix = std::env::args().any(|a| a == "--fix");
    let report = match import_show_dir(std::path::Path::new(&dir)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("import failed: {e}");
            std::process::exit(1);
        }
    };

    println!("{}", report.to_json());
    println!();
    println!("controllers:");
    for c in &report.controllers {
        println!(
            "  {:<14} {:<16} {:<7} {} universes, {} channels",
            c.name,
            c.ip,
            c.protocol,
            c.universes.len(),
            c.channel_capacity()
        );
    }
    println!("models: {}   groups: {}   pixels: {}",
        report.models.len(), report.groups.len(), report.total_pixels());

    if report.conflicts.is_empty() {
        match report.assignments() {
            Ok(a) => println!("\nGATE: OK — {} physical assignments ready for CompiledLayout", a.len()),
            Err(e) => println!("\nGATE: FAILED — {e}"),
        }
    } else {
        println!("\nGATE: {} channel conflicts — layout is ambiguous:", report.conflicts.len());
        for c in report.conflicts.iter().take(20) {
            println!("  {c}");
        }
        if report.conflicts.len() > 20 {
            println!("  … and {} more", report.conflicts.len() - 20);
        }

        if do_fix {
            let fixes = report.propose_fix();
            println!("\nproposed fixes: {} models get new start channels", fixes.len());

            let base = std::path::Path::new(&dir);
            let networks = std::fs::read_to_string(base.join("xlights_networks.xml")).unwrap();
            let original = std::fs::read_to_string(base.join("xlights_rgbeffects.xml")).unwrap();
            let fixed_xml = apply_fixes_to_xml(&original, &fixes);

            // Prove the fix before writing anything.
            let recheck = import_strings(&networks, &fixed_xml);
            if !recheck.conflicts.is_empty() {
                eprintln!("FIX FAILED VERIFICATION: {} conflicts remain", recheck.conflicts.len());
                std::process::exit(1);
            }
            match recheck.assignments() {
                Ok(a) => println!("fixed layout verified: 0 conflicts, {} assignments", a.len()),
                Err(e) => {
                    eprintln!("FIX FAILED VERIFICATION: {e}");
                    std::process::exit(1);
                }
            }

            // Never overwrite the original — write a sibling file.
            let out_path = base.join("xlights_rgbeffects.LUMYX-FIXED.xml");
            std::fs::write(&out_path, &fixed_xml).unwrap();
            println!("wrote {}", out_path.display());
        } else {
            println!("\n(run with --fix to generate a conflict-free copy)");
            std::process::exit(1);
        }
    }
}
