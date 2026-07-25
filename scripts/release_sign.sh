#!/usr/bin/env bash
# LUMYX release signing pipeline: build → SBOM → sign every artifact.
#
# Signing backends, in order:
#   1. cosign (keyless OIDC or $COSIGN_KEY) — when the binary is installed.
#   2. Built-in Ed25519 (led-show-recorder sign_file) — always available.
#
# Usage: ./scripts/release_sign.sh [key-file]
#   key-file: Ed25519 seed (created with `sign_file keygen` if absent).
set -euo pipefail
cd "$(dirname "$0")/.."

KEY="${1:-release/lumyx-release.key}"
OUT="release"
mkdir -p "$OUT"

echo "── build (release) ──────────────────────────────"
cargo build --release -p led-player -p led-demo

echo "── SBOM ─────────────────────────────────────────"
python3 scripts/generate_sbom.py --out "$OUT/sbom.cdx.json"

ARTIFACTS=(target/release/led-player target/release/led-demo "$OUT/sbom.cdx.json")

if [ ! -f "$KEY" ]; then
    echo "── keygen (first run) ───────────────────────────"
    cargo run --release -q -p led-show-recorder --example sign_file -- keygen "$KEY"
fi

echo "── sign ─────────────────────────────────────────"
for a in "${ARTIFACTS[@]}"; do
    cargo run --release -q -p led-show-recorder --example sign_file -- sign "$KEY" "$a"
done

if command -v cosign >/dev/null 2>&1; then
    echo "── cosign sign + SBOM attest ────────────────────"
    # Local key pair (non-interactive; keyless OIDC needs a browser). The
    # private key lives in release/ (gitignored), passphrase from env.
    export COSIGN_PASSWORD="${COSIGN_PASSWORD:-}"
    if [ ! -f "$OUT/cosign.key" ]; then
        (cd "$OUT" && cosign generate-key-pair)
    fi
    for a in target/release/led-player target/release/led-demo; do
        cosign sign-blob --yes --key "$OUT/cosign.key" "$a" \
            --new-bundle-format --bundle "$a.cosign.bundle"
        cosign attest-blob --yes --key "$OUT/cosign.key" \
            --predicate "$OUT/sbom.cdx.json" --type cyclonedx "$a" \
            --new-bundle-format --bundle "$a.sbom.bundle"
        # Verification is part of signing — an unverifiable signature is a bug.
        cosign verify-blob --key "$OUT/cosign.pub" \
            --new-bundle-format --bundle "$a.cosign.bundle" "$a"
        echo "cosign: $a signed + SBOM attested + verified"
    done
else
    echo "cosign not installed — Ed25519 sidecars written; install cosign to add"
    echo "attestations (brew install cosign) and re-run."
fi

echo "── verify (self-check) ──────────────────────────"
for a in "${ARTIFACTS[@]}"; do
    cargo run --release -q -p led-show-recorder --example sign_file -- \
        verify "$a" "$a.sig" "$a.pub"
done

echo "RELEASE SIGNED: ${#ARTIFACTS[@]} artifacts (+SBOM) — sidecars *.sig/*.pub"
