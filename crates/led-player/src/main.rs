//! LUMYX show player CLI.
//!
//! ```text
//! led-player show.lumyx --info                    # timeline summary (no output)
//! led-player show.lumyx --verify <hex-hash>       # integrity check against a known hash
//! led-player show.lumyx                           # play to the simulator (real time)
//! led-player show.lumyx --artnet 192.168.2.156 --first-universe 1
//! led-player show.lumyx --ddp 192.168.2.156      # pixel-native, ~3× fewer packets
//! led-player show.lumyx --speed 2.0               # 2× speed
//! led-player show.lumyx --loop 0                  # burn-in: loop forever (Ctrl-C to stop)
//! led-player show.lumyx --metrics 9464            # Prometheus em 0.0.0.0:9464/metrics
//! ```
//!
//! Burn-in mode (`--loop N`, 0 = infinite) re-verifies the manifest hash on
//! every pass and aborts on the first integrity or delivery failure — run it
//! for 72h against real hardware to close the burn-in gate.

use std::net::{SocketAddr, ToSocketAddrs};
use std::process::ExitCode;

use led_core::{CompiledLayout, RgbOrder};
use led_hal::{Hal, SimulatorDevice};
use led_player::{linear_assignments, DdpOutput, ShowInfo, Speed};
use led_protocols::ArtNetDevice;
use led_show_recorder::replay::ReplayManifest;
use led_show_recorder::ShowReader;

fn usage() -> ExitCode {
    eprintln!(
        "usage: led-player <show.lumyx> [--info] [--verify <hex-hash>] \
         [--artnet <ip[:port]>] [--ddp <ip[:port]>] [--first-universe N] \
         [--speed X|max] [--loop N] [--discover] [--require-all] \
         [--profile <preset>] [--list-profiles]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else { return usage() };

    let mut info_only = false;
    let mut verify: Option<u64> = None;
    let mut verify_key: Option<[u8; 32]> = None;
    let mut artnet: Option<SocketAddr> = None;
    let mut ddp: Option<SocketAddr> = None;
    let mut first_universe: u16 = 1;
    let mut speed = Speed::Factor(1.0);
    let mut loops: Option<u64> = None; // Some(0) = infinite burn-in
    let mut metrics_port: Option<u16> = None;
    let mut discover = false;
    let mut require_all = false;
    let mut profile_name: Option<String> = None;
    let mut list_profiles = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--info" => info_only = true,
            "--verify" => {
                i += 1;
                let Some(h) = args.get(i) else { return usage() };
                let h = h.trim_start_matches("0x");
                match u64::from_str_radix(h, 16) {
                    Ok(v) => verify = Some(v),
                    Err(_) => return usage(),
                }
            }
            "--verify-key" => {
                i += 1;
                let Some(h) = args.get(i) else { return usage() };
                let h = h.trim();
                if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                    eprintln!("--verify-key must be 64 hex chars (an Ed25519 public key)");
                    return ExitCode::from(2);
                }
                let mut k = [0u8; 32];
                for (j, chunk) in h.as_bytes().chunks(2).enumerate() {
                    k[j] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
                }
                verify_key = Some(k);
            }
            "--artnet" => {
                i += 1;
                let Some(addr) = args.get(i) else { return usage() };
                let full = if addr.contains(':') { addr.clone() } else { format!("{addr}:6454") };
                match full.to_socket_addrs().ok().and_then(|mut a| a.next()) {
                    Some(sa) => artnet = Some(sa),
                    None => {
                        eprintln!("cannot resolve --artnet address '{addr}'");
                        return ExitCode::from(2);
                    }
                }
            }
            "--ddp" => {
                i += 1;
                let Some(addr) = args.get(i) else { return usage() };
                let full = if addr.contains(':') { addr.clone() } else { format!("{addr}:4048") };
                match full.to_socket_addrs().ok().and_then(|mut a| a.next()) {
                    Some(sa) => ddp = Some(sa),
                    None => {
                        eprintln!("cannot resolve --ddp address '{addr}'");
                        return ExitCode::from(2);
                    }
                }
            }
            "--first-universe" => {
                i += 1;
                let Some(v) = args.get(i).and_then(|v| v.parse().ok()) else { return usage() };
                first_universe = v;
            }
            "--speed" => {
                i += 1;
                let Some(v) = args.get(i) else { return usage() };
                speed = if v == "max" {
                    Speed::Max
                } else {
                    match v.parse::<f32>() {
                        Ok(f) if f > 0.0 => Speed::Factor(f),
                        _ => return usage(),
                    }
                };
            }
            "--loop" => {
                i += 1;
                let Some(v) = args.get(i).and_then(|v| v.parse().ok()) else { return usage() };
                loops = Some(v);
            }
            "--metrics" => {
                i += 1;
                let Some(v) = args.get(i).and_then(|v| v.parse().ok()) else { return usage() };
                metrics_port = Some(v);
            }
            "--discover" => discover = true,
            "--require-all" => { discover = true; require_all = true; }
            "--profile" => {
                i += 1;
                let Some(v) = args.get(i) else { return usage() };
                profile_name = Some(v.clone());
            }
            "--list-profiles" => list_profiles = true,
            _ => return usage(),
        }
        i += 1;
    }

    if list_profiles {
        let reg = led_hardware_profile::HardwareRegistry::with_builtin();
        println!("presets embutidos (ADR-0018):");
        for name in reg.names() {
            let p = reg.profile(name).expect("preset");
            println!(
                "  {name:<26} {} {} · {:?}/{:?} · {} px/uni · gamma {} · brightness {}",
                p.identity.vendor,
                p.identity.model,
                p.capabilities.protocol,
                p.capabilities.output_interface,
                p.limits.pixels_per_universe,
                p.calibration.gamma,
                p.calibration.brightness,
            );
        }
        return ExitCode::SUCCESS;
    }

    // Read the whole recording (shows are minutes, not hours — memory is fine
    // and it lets us verify the manifest before the first frame goes out).
    let reader = match ShowReader::open(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot open '{path}': {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let records = match reader.collect_all() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read '{path}': {e:?}");
            return ExitCode::FAILURE;
        }
    };

    let info = ShowInfo::from_records(&records);
    println!("{}", info.to_json());

    if let Some(expected) = verify {
        let manifest = ReplayManifest::from_records(&records);
        if manifest.aggregate_hash != expected {
            eprintln!(
                "VERIFY FAILED: recorded {:#018x} != expected {expected:#018x}",
                manifest.aggregate_hash
            );
            return ExitCode::FAILURE;
        }
        println!("verify: OK ({:#018x})", manifest.aggregate_hash);
    }

    // Authentic verification: pin the operator's key against the <show>.sig
    // sidecar. Rejects a re-signed tamper (embedded-key trust is not enough).
    if let Some(key) = verify_key {
        use led_show_recorder::signing::{verify_manifest_pinned, SignatureError, SignedManifest};
        let sidecar = format!("{path}.sig");
        let blob = match std::fs::read(&sidecar) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("--verify-key needs a signature sidecar '{sidecar}': {e}");
                return ExitCode::FAILURE;
            }
        };
        let signed = match SignedManifest::from_bytes(&blob) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("malformed sidecar '{sidecar}': {e:?}");
                return ExitCode::FAILURE;
            }
        };
        // The sidecar must describe THIS show, and be signed by the pinned key.
        let live = ReplayManifest::from_records(&records);
        if signed.manifest.aggregate_hash != live.aggregate_hash {
            eprintln!(
                "SIG VERIFY FAILED: sidecar covers {:#018x}, show is {:#018x}",
                signed.manifest.aggregate_hash, live.aggregate_hash
            );
            return ExitCode::FAILURE;
        }
        match verify_manifest_pinned(&signed, &key) {
            Ok(()) => println!("sig-verify: OK (authentic, pinned key)"),
            Err(SignatureError::UntrustedKey) => {
                eprintln!("SIG VERIFY FAILED: signed by a key that is NOT the pinned key (possible re-signed tamper)");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("SIG VERIFY FAILED: {e:?}");
                return ExitCode::FAILURE;
            }
        }
    }

    if info_only {
        return ExitCode::SUCCESS;
    }

    // Pre-show discovery: before the first frame, confirm the controller we are
    // about to drive actually answers ArtPoll. A dead/wrong-subnet/off-WiFi
    // controller answers nothing → the operator learns it now, not mid-show.
    if discover {
        let target_ip = ddp.or(artnet).map(|sa| sa.ip());
        match target_ip {
            Some(std::net::IpAddr::V4(ip)) => {
                use led_protocols::discover_controllers;
                use std::time::Duration;
                print!("discovery: probing {ip} (ArtPoll, 1.5s)… ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                match discover_controllers(&[ip], Duration::from_millis(1500)) {
                    Ok(res) if res.all_present() => println!("✅ respondeu"),
                    Ok(_) => {
                        println!("⚠ SEM resposta");
                        if require_all {
                            eprintln!("ABORT: --require-all e {ip} não respondeu ao ArtPoll (desligado? WiFi morto? subnet errada?)");
                            return ExitCode::FAILURE;
                        }
                        eprintln!("aviso: seguindo mesmo assim (sem --require-all); o palco pode ficar escuro");
                    }
                    Err(e) => {
                        eprintln!("discovery falhou (socket ArtPoll :6454 — precisa de porta livre): {e}");
                        if require_all {
                            return ExitCode::FAILURE;
                        }
                    }
                }
            }
            _ => eprintln!("--discover requer um alvo IPv4 (--artnet/--ddp)"),
        }
    }

    let px = info.pixel_count as usize;

    // Layout + calibração: de um HardwareProfile quando `--profile` é dado, senão o
    // comportamento histórico (RGB linear, 170 px/universo). O app é o ponto de composição —
    // é aqui que o descritor de design-time vira artefatos de runtime (ADR-0018/0019).
    let (layout, calibration) = match &profile_name {
        None => (
            CompiledLayout::compile(&linear_assignments(px, 0, first_universe, RgbOrder::Rgb)),
            led_hal::Calibration::new(),
        ),
        Some(name) => {
            use led_hardware_profile as hwp;
            let registry = hwp::HardwareRegistry::with_builtin();
            let Some(profile) = registry.profile(name) else {
                eprintln!("profile desconhecido: '{name}' — use --list-profiles");
                return ExitCode::FAILURE;
            };

            // O que ESTE binário sabe executar, passado como dado (o crate do profile é leaf).
            let available = hwp::Available {
                interfaces: &[hwp::OutputInterface::Ethernet, hwp::OutputInterface::WiFi],
                protocols: &[hwp::Protocol::ArtNet, hwp::Protocol::Ddp],
            };
            let report = hwp::validate(&profile, &available);
            for w in report.warnings() {
                eprintln!("⚠ profile '{name}': {w:?}");
            }
            if report.has_errors() {
                for e in report.errors() {
                    eprintln!("✖ profile '{name}': {e:?}");
                }
                return ExitCode::FAILURE;
            }

            let layout = match hwp::compile_layout(&profile, px as u32, 0, first_universe) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("✖ profile '{name}' não compila para {px} px: {e:?}");
                    return ExitCode::FAILURE;
                }
            };
            let mut cal = led_hal::Calibration::new();
            cal.set(0, profile.calibration.gamma, profile.calibration.brightness);
            println!(
                "profile: {name} ({} {}) · {:?} · {} px/uni · gamma {} · brightness {}",
                profile.identity.vendor,
                profile.identity.model,
                profile.capabilities.color,
                profile.limits.pixels_per_universe,
                profile.calibration.gamma,
                profile.calibration.brightness,
            );
            (layout, cal)
        }
    };

    // Observability: attach a MetricsEmitter to the Hal and expose /metrics.
    let emitter = std::sync::Arc::new(led_hal::MetricsEmitter::new("led-player"));
    let _metrics_server = match metrics_port {
        Some(port) => {
            let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
            match led_hal::serve_metrics(vec![emitter.clone()], addr) {
                Ok(s) => {
                    println!("metrics: http://{}/metrics", s.addr);
                    Some(s)
                }
                Err(e) => {
                    eprintln!("metrics bind failed: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => None,
    };
    let output: Box<dyn led_core::ProtocolOutput> = if let Some(dest) = ddp {
        // Pixel-native DDP: 487 px/datagram, no universe mapping (WLED-friendly).
        match DdpOutput::new(dest, px) {
            Ok(o) => {
                println!("output: DDP {dest} (pixel-native)");
                if !calibration.is_empty() {
                    eprintln!(
                        "⚠ calibração do profile NÃO se aplica no caminho DDP pixel-nativo: \
                         ele bypassa o HAL por design (ADR-0003), e a calibração vive no HAL \
                         (ADR-0019). Use --artnet para saída calibrada."
                    );
                }
                Box::new(o)
            }
            Err(e) => {
                eprintln!("ddp socket: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else if let Some(dest) = artnet {
        // Real hardware: linear mapping, universes from --first-universe (WLED
        // rigs number from 1), 170 px per universe.
        let dev = match ArtNetDevice::unicast(0, dest) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("artnet socket: {e}");
                return ExitCode::FAILURE;
            }
        };
        // Product-red-team guard against the silent --first-universe footgun:
        // universe 0 is almost always a mistake (WLED/xLights number from 1),
        // and a huge start hints at a typo. Warn loudly; do not block (some
        // rigs legitimately use high universes).
        let last_universe = first_universe as u32 + (px as u32).div_ceil(170).saturating_sub(1);
        if first_universe == 0 {
            eprintln!("⚠ --first-universe 0: WLED/xLights number universes from 1 — did you mean 1?");
        }
        if last_universe > 32767 {
            eprintln!("⚠ universes {first_universe}..{last_universe} exceed the 15-bit Art-Net range — likely a typo");
        }
        println!("output: ArtNet {dest} (universes {first_universe}..{last_universe})");
        Box::new(Hal::new(layout, vec![dev]).with_calibration(calibration))
    } else {
        let sim = SimulatorDevice::new(0, layout.device_universes(0));
        println!("output: simulator");
        Box::new(Hal::new(layout, vec![sim]).with_calibration(calibration))
    };

    // Burn-in: loop the show, re-verifying integrity every pass. Abort on the
    // first hash mismatch or delivery failure — that IS the burn-in signal.
    let total_loops = loops.unwrap_or(1);
    let mut pass: u64 = 0;
    loop {
        pass += 1;
        match led_player::play_instrumented(&records, output.as_ref(), speed, Some(&emitter)) {
            Ok(r) => {
                println!(
                    r#"{{"pass":{pass},"played":{},"failed":{},"duration_ms":{},"hash":"{:#018x}"}}"#,
                    r.frames_played, r.frames_failed, r.duration_ms, r.manifest_hash
                );
                if r.manifest_hash != info.manifest_hash {
                    eprintln!("BURN-IN ABORT: hash drift on pass {pass}");
                    return ExitCode::FAILURE;
                }
                if r.frames_failed > 0 {
                    eprintln!("BURN-IN ABORT: {} failed frames on pass {pass}", r.frames_failed);
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                eprintln!("playback error on pass {pass}: {e:?}");
                return ExitCode::FAILURE;
            }
        }
        if total_loops != 0 && pass >= total_loops {
            break;
        }
    }
    ExitCode::SUCCESS
}
