//! Sign a `.lumyx` show into a `<show>.sig` sidecar (SignedManifest).
//!
//! ```text
//! sign_show <show.lumyx> <key-hex-or-file>     # → <show.lumyx>.sig  (+ prints pubkey)
//! ```
//!
//! The venue verifies with `led-player --verify-key <pubkey-hex>`, which pins
//! the key out-of-band — closing the red-team finding that unpinned verify
//! trusts the key embedded in the blob.

use led_show_recorder::replay::ReplayManifest;
use led_show_recorder::signing::ShowSigner;
use led_show_recorder::ShowReader;

fn main() {
    let show = std::env::args().nth(1).expect("usage: sign_show <show.lumyx> <key>");
    let key_arg = std::env::args().nth(2).expect("usage: sign_show <show.lumyx> <key>");

    // Key: 64 hex chars directly, or a file containing them.
    let hex = if key_arg.len() == 64 && key_arg.chars().all(|c| c.is_ascii_hexdigit()) {
        key_arg
    } else {
        std::fs::read_to_string(&key_arg).expect("read key file").trim().to_string()
    };
    let mut seed = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).take(32).enumerate() {
        seed[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).expect("hex seed");
    }
    let signer = ShowSigner::from_seed(seed);

    let records = ShowReader::open(&show).expect("open show").collect_all().expect("read show");
    let manifest = ReplayManifest::from_records(&records);
    let signed = signer.sign_manifest(&manifest);

    let sidecar = format!("{show}.sig");
    std::fs::write(&sidecar, signed.to_bytes()).expect("write sidecar");
    let pubhex: String = signer.public_key().iter().map(|b| format!("{b:02x}")).collect();
    println!("signed: {sidecar}");
    println!("hash:   {:#018x}", manifest.aggregate_hash);
    println!("pubkey: {pubhex}");
    println!("verify at venue: led-player {show} --verify-key {pubhex}");
}
