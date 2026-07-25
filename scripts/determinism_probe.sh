#!/usr/bin/env bash
# LUMYX cross-platform determinism probe.
#
# Run this ON EACH target platform (Linux x86_64/arm64, macOS, Windows via
# Git-Bash/WSL) and archive the output as certification evidence:
#
#   ./scripts/determinism_probe.sh | tee "determinism-$(uname -s)-$(uname -m).txt"
#
# Verdict semantics (see integration-tests/tests/determinism_vector.rs):
#   - intent hash:  integer math — MUST match the golden on every platform.
#   - plasma hash:  f32 trig — measures libm divergence; a mismatch is a
#                   FINDING to record (mitigation: table-based trig), not
#                   automatically a failure of the platform.
set -u

echo "== LUMYX determinism probe =="
echo "platform: $(uname -s) $(uname -m)"
echo "rustc:    $(rustc --version 2>/dev/null || echo 'not installed')"
echo "date:     $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo

cd "$(dirname "$0")/.."
if cargo test -p integration-tests --test determinism_vector 2>&1 \
    | tee /dev/stderr | grep -q "test result: ok. 3 passed"; then
    echo
    echo "VERDICT: MATCH — bit-identical with the reference platform (macOS arm64)"
    exit 0
else
    echo
    echo "VERDICT: DIVERGENCE — record observed hashes above in docs/determinism-findings.md"
    exit 1
fi
