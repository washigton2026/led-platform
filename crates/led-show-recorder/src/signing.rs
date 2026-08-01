//! Ed25519 signing for replays and show snapshots.
//!
//! A signed [`ReplayManifest`] proves that a `.lumyx` file (a) is the exact
//! pixel stream the operator approved and (b) has not been altered between
//! the studio and the show machine. Verification is offline: only the 32-byte
//! public key travels to the venue.
//!
//! ## Invariants (lumyx-security-architect)
//! - Signatures cover a **canonical byte encoding** of the manifest (version,
//!   counts, aggregate hash, every frame hash) — not a debug format.
//! - Key seeds come from the OS (`/dev/urandom`), never from a PRNG we roll.
//! - Ed25519 signatures are deterministic: same key + same manifest ⇒ same
//!   signature (replays stay reproducible end-to-end).
//! - Verification failure is a value (`SignatureError`), never a panic.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::replay::ReplayManifest;

/// Version tag included in the canonical signing bytes.
const SIGNING_VERSION: u8 = 1;

/// Why a signed manifest failed to verify.
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureError {
    /// The signature does not match the manifest bytes + public key.
    BadSignature,
    /// The blob is too short / malformed.
    Malformed,
    /// The embedded public key is not a valid Ed25519 point.
    BadPublicKey,
    /// The embedded key is valid but is NOT the pinned/trusted key
    /// (a re-signed tamper). Only from [`verify_manifest_pinned`].
    UntrustedKey,
}

/// A manifest plus the signature that vouches for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SignedManifest {
    pub manifest: ReplayManifest,
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

/// Canonical byte encoding of a manifest — what actually gets signed.
fn canonical_bytes(m: &ReplayManifest) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + 4 + 8 + m.frame_hashes.len() * 8);
    out.push(SIGNING_VERSION);
    out.extend_from_slice(&(m.frame_count as u64).to_le_bytes());
    out.extend_from_slice(&m.pixel_count.to_le_bytes());
    out.extend_from_slice(&m.aggregate_hash.to_le_bytes());
    for h in &m.frame_hashes {
        out.extend_from_slice(&h.to_le_bytes());
    }
    out
}

/// A signing identity (keep the seed private; ship only [`ShowSigner::public_key`]).
pub struct ShowSigner {
    key: SigningKey,
}

impl ShowSigner {
    /// Deterministic identity from a 32-byte seed (tests, HSM-escrowed seeds).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { key: SigningKey::from_bytes(&seed) }
    }

    /// Fresh identity from OS entropy (`/dev/urandom`). std-only, no RNG crate.
    pub fn generate() -> std::io::Result<Self> {
        use std::io::Read;
        let mut seed = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
        Ok(Self::from_seed(seed))
    }

    /// The 32-byte public half — safe to distribute.
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Sign a replay manifest.
    pub fn sign_manifest(&self, manifest: &ReplayManifest) -> SignedManifest {
        let sig = self.key.sign(&canonical_bytes(manifest));
        SignedManifest {
            manifest: manifest.clone(),
            public_key: self.public_key(),
            signature: sig.to_bytes(),
        }
    }

    /// Sign arbitrary snapshot bytes (a whole `.lumyx` file, a config blob…).
    pub fn sign_bytes(&self, data: &[u8]) -> [u8; 64] {
        self.key.sign(data).to_bytes()
    }
}

/// Verify a signed manifest against its **embedded** public key.
///
/// ⚠️ This proves INTEGRITY + internal consistency ONLY, not AUTHENTICITY: the
/// signature is checked against the key carried in the same blob, so an attacker
/// who alters the manifest can re-sign with their own key, embed their own public
/// key, and this returns `Ok`. (Proven by `redteam_resigned_tamper_defeats_unpinned`.)
///
/// Use this only when the blob's origin is already trusted (e.g. a local file you
/// just wrote). To verify a manifest that crossed a trust boundary — the studio→
/// venue path this signing exists for — use [`verify_manifest_pinned`] with the
/// operator's known public key.
#[deprecated(
    since = "0.1.0",
    note = "prova INTEGRIDADE, não AUTENTICIDADE: um atacante pode re-assinar um manifest \
            adulterado com a própria chave e esta função retorna Ok (provado por \
            redteam_resigned_tamper_defeats_unpinned_verify). Em qualquer fronteira de \
            confiança use verify_manifest_pinned com a chave confiada out-of-band (ADR-0004). \
            Mantida para o uso local legítimo: verificar um arquivo que você mesmo acabou de escrever."
)]
pub fn verify_manifest(signed: &SignedManifest) -> Result<(), SignatureError> {
    let vk = VerifyingKey::from_bytes(&signed.public_key)
        .map_err(|_| SignatureError::BadPublicKey)?;
    let sig = Signature::from_bytes(&signed.signature);
    vk.verify(&canonical_bytes(&signed.manifest), &sig)
        .map_err(|_| SignatureError::BadSignature)
}

/// Verify a signed manifest against a **pinned** (pre-trusted) public key.
///
/// This is the authentic-verification path: it rejects the blob unless it was
/// signed by exactly `trusted_key`. A re-signed tamper (attacker's key embedded)
/// is rejected because the embedded key ≠ the pinned key — closing the hole in
/// [`verify_manifest`]. The pinned key travels to the venue out-of-band (the
/// 32-byte `ShowSigner::public_key`), never inside the show file.
pub fn verify_manifest_pinned(
    signed: &SignedManifest,
    trusted_key: &[u8; 32],
) -> Result<(), SignatureError> {
    // Constant-time-ish identity check first: the embedded key must BE the
    // trusted key, not merely a valid Ed25519 point.
    if &signed.public_key != trusted_key {
        return Err(SignatureError::UntrustedKey);
    }
    let vk = VerifyingKey::from_bytes(trusted_key).map_err(|_| SignatureError::BadPublicKey)?;
    let sig = Signature::from_bytes(&signed.signature);
    vk.verify(&canonical_bytes(&signed.manifest), &sig)
        .map_err(|_| SignatureError::BadSignature)
}

/// Verify detached snapshot bytes.
pub fn verify_bytes(
    data: &[u8],
    signature: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<(), SignatureError> {
    let vk = VerifyingKey::from_bytes(public_key).map_err(|_| SignatureError::BadPublicKey)?;
    vk.verify(data, &Signature::from_bytes(signature))
        .map_err(|_| SignatureError::BadSignature)
}

impl SignedManifest {
    /// Sidecar serialization (`show.lumyx.sig`):
    /// `[1B version][32B pubkey][64B sig][8B frames][4B pixels][8B agg][8B × frame hashes]`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(SIGNING_VERSION);
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&(self.manifest.frame_count as u64).to_le_bytes());
        out.extend_from_slice(&self.manifest.pixel_count.to_le_bytes());
        out.extend_from_slice(&self.manifest.aggregate_hash.to_le_bytes());
        for h in &self.manifest.frame_hashes {
            out.extend_from_slice(&h.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, SignatureError> {
        const FIXED: usize = 1 + 32 + 64 + 8 + 4 + 8;
        if b.len() < FIXED || b[0] != SIGNING_VERSION {
            return Err(SignatureError::Malformed);
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&b[1..33]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&b[33..97]);
        let frame_count = u64::from_le_bytes(b[97..105].try_into().unwrap()) as usize;
        let pixel_count = u32::from_le_bytes(b[105..109].try_into().unwrap());
        let aggregate_hash = u64::from_le_bytes(b[109..117].try_into().unwrap());
        let rest = &b[117..];
        if rest.len() != frame_count * 8 {
            return Err(SignatureError::Malformed);
        }
        let frame_hashes = rest
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Ok(Self {
            manifest: ReplayManifest { frame_count, aggregate_hash, frame_hashes, pixel_count },
            public_key: pk,
            signature: sig,
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Estes testes exercitam `verify_manifest` DE PROPÓSITO — inclusive o teste de red-team
    // que documenta o buraco que motivou a depreciação (ADR-0004). Silenciar o aviso aqui é
    // correto; silenciá-lo em código de produção não seria.
    #![allow(deprecated)]

    use super::*;
    use crate::ShowRecord;
    use led_core::PixelColor;

    fn manifest() -> ReplayManifest {
        let records: Vec<ShowRecord> = (0..10u64)
            .map(|i| ShowRecord {
                timestamp_ms: i * 33,
                pixels: vec![PixelColor::rgb(i as u8, 0, 0); 4],
                audio: None,
            })
            .collect();
        ReplayManifest::from_records(&records)
    }

    #[test]
    fn sign_verify_roundtrip() {
        let signer = ShowSigner::from_seed([7u8; 32]);
        let signed = signer.sign_manifest(&manifest());
        assert_eq!(verify_manifest(&signed), Ok(()));
    }

    #[test]
    fn tampered_aggregate_hash_fails() {
        let signer = ShowSigner::from_seed([7u8; 32]);
        let mut signed = signer.sign_manifest(&manifest());
        signed.manifest.aggregate_hash ^= 1;
        assert_eq!(verify_manifest(&signed), Err(SignatureError::BadSignature));
    }

    #[test]
    fn tampered_frame_hash_fails() {
        let signer = ShowSigner::from_seed([7u8; 32]);
        let mut signed = signer.sign_manifest(&manifest());
        signed.manifest.frame_hashes[3] ^= 0xFF;
        assert_eq!(verify_manifest(&signed), Err(SignatureError::BadSignature));
    }

    #[test]
    fn wrong_key_fails() {
        let signer = ShowSigner::from_seed([7u8; 32]);
        let other = ShowSigner::from_seed([8u8; 32]);
        let mut signed = signer.sign_manifest(&manifest());
        signed.public_key = other.public_key();
        assert_eq!(verify_manifest(&signed), Err(SignatureError::BadSignature));
    }

    #[test]
    fn signature_is_deterministic() {
        let signer = ShowSigner::from_seed([9u8; 32]);
        let a = signer.sign_manifest(&manifest());
        let b = signer.sign_manifest(&manifest());
        assert_eq!(a.signature, b.signature, "Ed25519 is deterministic — replays stay reproducible");
    }

    #[test]
    fn sidecar_bytes_roundtrip_and_still_verify() {
        let signer = ShowSigner::from_seed([11u8; 32]);
        let signed = signer.sign_manifest(&manifest());
        let blob = signed.to_bytes();
        let restored = SignedManifest::from_bytes(&blob).expect("parses");
        assert_eq!(restored, signed);
        assert_eq!(verify_manifest(&restored), Ok(()));
    }

    #[test]
    fn malformed_sidecar_is_rejected() {
        assert_eq!(SignedManifest::from_bytes(&[1, 2, 3]), Err(SignatureError::Malformed));
        let signer = ShowSigner::from_seed([1u8; 32]);
        let mut blob = signer.sign_manifest(&manifest()).to_bytes();
        blob.truncate(blob.len() - 4); // frame hash table cut short
        assert_eq!(SignedManifest::from_bytes(&blob), Err(SignatureError::Malformed));
    }

    #[test]
    fn snapshot_bytes_sign_and_verify() {
        let signer = ShowSigner::from_seed([5u8; 32]);
        let data = b"the .lumyx file bytes";
        let sig = signer.sign_bytes(data);
        assert_eq!(verify_bytes(data, &sig, &signer.public_key()), Ok(()));
        let mut tampered = data.to_vec();
        tampered[0] ^= 1;
        assert_eq!(
            verify_bytes(&tampered, &sig, &signer.public_key()),
            Err(SignatureError::BadSignature)
        );
    }

    // ── Red Team: signature authenticity ──────────────────────────────────

    /// PROOF-OF-EXPLOIT (security-red-team, "como quebrar isso?"): the unpinned
    /// `verify_manifest` accepts a fully re-signed tamper. This test PASSES,
    /// documenting the vulnerability — it is the reason `verify_manifest_pinned`
    /// exists. It must keep passing (the flaw in the unpinned path is inherent;
    /// the mitigation is to not use it across a trust boundary).
    #[test]
    fn redteam_resigned_tamper_defeats_unpinned_verify() {
        let studio = ShowSigner::from_seed([1u8; 32]);
        let legit = studio.sign_manifest(&manifest());

        // Attacker alters the show and re-signs with THEIR OWN key, embedding
        // their own public key in the blob.
        let attacker = ShowSigner::from_seed([0x66u8; 32]);
        let mut forged = legit.clone();
        forged.manifest.aggregate_hash ^= 0xDEAD_BEEF; // tamper the pixels' hash
        let sig = attacker.key.sign(&canonical_bytes(&forged.manifest));
        forged.signature = sig.to_bytes();
        forged.public_key = attacker.public_key();

        // Unpinned verify is fooled — internal consistency holds.
        assert_eq!(verify_manifest(&forged), Ok(()),
            "unpinned verify trusts the embedded key → tamper undetected");
        assert_ne!(forged.public_key, studio.public_key(),
            "but it was NOT the studio's key");
    }

    /// The mitigation: pinned verify rejects the same re-signed tamper.
    /// NEGATIVE TEST — if this ever returns Ok, the fix has regressed.
    #[test]
    fn pinned_verify_rejects_resigned_tamper() {
        let studio = ShowSigner::from_seed([1u8; 32]);
        let trusted = studio.public_key(); // travels out-of-band to the venue

        let attacker = ShowSigner::from_seed([0x66u8; 32]);
        let mut forged = studio.sign_manifest(&manifest());
        forged.manifest.aggregate_hash ^= 0xDEAD_BEEF;
        forged.signature = attacker.key.sign(&canonical_bytes(&forged.manifest)).to_bytes();
        forged.public_key = attacker.public_key();

        assert_eq!(verify_manifest_pinned(&forged, &trusted), Err(SignatureError::UntrustedKey),
            "pinned verify must reject a key that is not the trusted one");
    }

    #[test]
    fn pinned_verify_accepts_legitimate_and_rejects_content_tamper() {
        let studio = ShowSigner::from_seed([1u8; 32]);
        let trusted = studio.public_key();
        let signed = studio.sign_manifest(&manifest());

        // Legitimate: right key, untouched content → Ok.
        assert_eq!(verify_manifest_pinned(&signed, &trusted), Ok(()));

        // Right key claimed but content tampered WITHOUT re-sign → BadSignature.
        let mut tampered = signed.clone();
        tampered.manifest.frame_hashes[0] ^= 1;
        assert_eq!(verify_manifest_pinned(&tampered, &trusted),
            Err(SignatureError::BadSignature));
    }

    #[test]
    fn generate_produces_distinct_working_identities() {
        let a = ShowSigner::generate().expect("os entropy");
        let b = ShowSigner::generate().expect("os entropy");
        assert_ne!(a.public_key(), b.public_key(), "distinct keys");
        let signed = a.sign_manifest(&manifest());
        assert_eq!(verify_manifest(&signed), Ok(()));
    }
}
