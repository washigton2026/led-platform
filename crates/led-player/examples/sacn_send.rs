//! Minimal E1.31 sACN hardware validator — exercises the existing `SacnDevice`
//! against real hardware. NOT product code.
//!
//! Usage: sacn_send <show.lumyx> <ip>
//!
//! Sends the recording to <ip>:5568 via unicast E1.31, universes 1-N,
//! and reports played / failed counts.

use std::net::SocketAddr;

use led_core::{CompiledLayout, RgbOrder};
use led_hal::Hal;
use led_protocols::SacnDevice;
use led_player::{linear_assignments, ShowInfo, Speed};
use led_show_recorder::ShowReader;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: sacn_send <show.lumyx> <ip[:port]> [first_universe]");
        std::process::exit(2);
    }
    let path = &args[0];
    let addr_str = &args[1];
    let first_universe: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let use_multicast = args.iter().any(|a| a == "--multicast");
    let addr: SocketAddr = if addr_str.contains(':') {
        match addr_str.parse() {
            Ok(a) => a,
            Err(e) => { eprintln!("bad address '{addr_str}': {e}"); std::process::exit(2); }
        }
    } else {
        format!("{addr_str}:5568").parse().unwrap()
    };

    let reader = match ShowReader::open(path) {
        Ok(r) => r,
        Err(e) => { eprintln!("cannot open '{path}': {e:?}"); std::process::exit(1); }
    };
    let records = match reader.collect_all() {
        Ok(r) => r,
        Err(e) => { eprintln!("cannot read '{path}': {e:?}"); std::process::exit(1); }
    };

    let info = ShowInfo::from_records(&records);
    println!("{}", info.to_json());

    let px = info.pixel_count as usize;
    // CID: zeroed is valid for testing (RFC 8126 §4)
    let cid = [0u8; 16];
    let dev = if use_multicast {
        match SacnDevice::multicast(0, cid, "LUMYX-sacn-send") {
            Ok(d) => { println!("mode: MULTICAST (239.255.<hi>.<lo>:5568)"); d }
            Err(e) => { eprintln!("sacn multicast socket: {e}"); std::process::exit(1); }
        }
    } else {
        match SacnDevice::unicast(0, addr, cid, "LUMYX-sacn-send") {
            Ok(d) => d,
            Err(e) => { eprintln!("sacn socket: {e}"); std::process::exit(1); }
        }
    };

    // Linear mapping: 170px/universe, starting universe configurable
    let assigns = linear_assignments(px, 0, first_universe, RgbOrder::Rgb);
    let layout = CompiledLayout::compile(&assigns);
    let hal = Hal::new(layout, vec![dev]);

    println!("output: sACN {addr} unicast (universes {first_universe}..{})",
        first_universe as usize + px.div_ceil(170) - 1);

    match led_player::play_instrumented(&records, &hal, Speed::Factor(1.0), None) {
        Ok(r) => println!(
            r#"{{"played":{},"failed":{},"duration_ms":{},"hash":"{:#018x}"}}"#,
            r.frames_played, r.frames_failed, r.duration_ms, r.manifest_hash
        ),
        Err(e) => {
            eprintln!("playback error: {e:?}");
            std::process::exit(1);
        }
    }
}
