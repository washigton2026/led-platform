//! Sign / verify arbitrary files with the platform's Ed25519 identity.
//!
//! ```text
//! sign_file keygen  <key-file>                    # new identity (seed, 0600)
//! sign_file sign    <key-file> <artifact>         # → <artifact>.sig (+ .pub)
//! sign_file verify  <artifact> <artifact>.sig <pubkey-file>
//! ```

use led_show_recorder::signing::{verify_bytes, ShowSigner};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = args.get(1).expect("keygen <key-file>");
            // The seed file IS the identity — 32 bytes of OS entropy, mode 0600.
            let mut seed = [0u8; 32];
            {
                use std::io::Read;
                std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut seed).unwrap();
            }
            let signer = ShowSigner::from_seed(seed);
            std::fs::write(path, hex(&seed)).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            std::fs::write(format!("{path}.pub"), hex(&signer.public_key())).unwrap();
            println!("keygen: {path} (+.pub) public={}", hex(&signer.public_key()));
        }
        Some("sign") => {
            let key_path = args.get(1).expect("sign <key-file> <artifact>");
            let artifact = args.get(2).expect("sign <key-file> <artifact>");
            let seed = unhex::<32>(&std::fs::read_to_string(key_path).unwrap());
            let signer = ShowSigner::from_seed(seed);
            let data = std::fs::read(artifact).unwrap();
            let sig = signer.sign_bytes(&data);
            std::fs::write(format!("{artifact}.sig"), hex(&sig)).unwrap();
            std::fs::write(format!("{artifact}.pub"), hex(&signer.public_key())).unwrap();
            println!("signed: {artifact}.sig (ed25519, {} bytes covered)", data.len());
        }
        Some("verify") => {
            let artifact = args.get(1).expect("verify <artifact> <sig> <pub>");
            let sig_path = args.get(2).expect("verify <artifact> <sig> <pub>");
            let pub_path = args.get(3).expect("verify <artifact> <sig> <pub>");
            let data = std::fs::read(artifact).unwrap();
            let sig = unhex::<64>(&std::fs::read_to_string(sig_path).unwrap());
            let pk = unhex::<32>(&std::fs::read_to_string(pub_path).unwrap());
            match verify_bytes(&data, &sig, &pk) {
                Ok(()) => println!("verify: OK"),
                Err(e) => {
                    eprintln!("verify: FAILED ({e:?})");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: sign_file keygen|sign|verify …");
            std::process::exit(2);
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> [u8; N] {
    let s = s.trim();
    let mut out = [0u8; N];
    for (i, chunk) in s.as_bytes().chunks(2).take(N).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
}
