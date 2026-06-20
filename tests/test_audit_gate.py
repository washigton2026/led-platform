#!/usr/bin/env python3
"""
Tests for scripts/audit_gate.py — KB-012 self-verification.

The gate must pass its own criterion: it must have a negative control.
If test_gate_rejects_bad_ledger PASSES, the gate is functioning.
If it FAILS, the gate itself is broken (would let bad closures through).

Run:
  python3 tests/test_audit_gate.py
  # or:
  python3 -m pytest tests/test_audit_gate.py -v
"""

import sys
import textwrap
import tempfile
import os
from pathlib import Path

# Add scripts/ to path so we can import audit_gate
sys.path.insert(0, str(Path(__file__).parent.parent / 'scripts'))
import audit_gate


WORKSPACE = Path(__file__).parent.parent

# ── Helpers ───────────────────────────────────────────────────────────────────

def make_ledger(content: str) -> Path:
    """Write a temp ledger file and return its path."""
    tmp = tempfile.NamedTemporaryFile(mode='w', suffix='.md', delete=False)
    tmp.write(content)
    tmp.flush()
    return Path(tmp.name)


def run_gate(ledger_path: Path, workspace: Path = WORKSPACE) -> tuple[int, list]:
    """Run the gate and return (exit_code, findings)."""
    g = audit_gate.Gate(workspace)
    tds = audit_gate.parse_ledger(ledger_path)
    exit_code = g.run(tds)
    return exit_code, g.findings


# ── Negative control: bad ledger MUST be rejected ─────────────────────────────

def test_gate_rejects_bad_ledger_a_no_evidence():
    """TD-BAD-A: closed with no evidence_ref → MUST exit 1."""
    ledger = WORKSPACE / 'tests/fixtures/ledger_bad.md'
    g = audit_gate.Gate(WORKSPACE)
    tds = audit_gate.parse_ledger(ledger)
    td_a = next((t for t in tds if t.get('td_id') == 'TD-BAD-A'), None)
    assert td_a is not None, "TD-BAD-A not found in bad ledger fixture"
    g.check(td_a)
    criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
    assert len(criticals) >= 1, (
        "GATE BROKEN: TD-BAD-A (closed, no evidence_ref) was not rejected. "
        "The gate would allow unsubstantiated closures through."
    )
    assert any('evidence_ref' in f[2] or 'negative_control' in f[2]
               for f in criticals), \
        f"Critical finding should mention missing fields, got: {criticals}"
    print("✅ test_gate_rejects_bad_ledger_a_no_evidence: PASS")


def test_gate_rejects_bad_ledger_b_zero_passed():
    """TD-BAD-B: evidence with 0 passed → MUST exit 1."""
    ledger = WORKSPACE / 'tests/fixtures/ledger_bad.md'
    g = audit_gate.Gate(WORKSPACE)
    tds = audit_gate.parse_ledger(ledger)
    td_b = next((t for t in tds if t.get('td_id') == 'TD-BAD-B'), None)
    assert td_b is not None, "TD-BAD-B not found in bad ledger fixture"
    g.check(td_b)
    criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
    assert len(criticals) >= 1, (
        "GATE BROKEN: TD-BAD-B (evidence with 0 passed) was not rejected. "
        "The Miri N=0 pattern would slip through."
    )
    assert any('0' in f[2] or 'zero' in f[2].lower() or 'N=0' in f[2]
               or 'nothing' in f[2]
               for f in criticals), \
        f"Critical should mention zero-passed, got: {criticals}"
    print("✅ test_gate_rejects_bad_ledger_b_zero_passed: PASS")


def test_gate_rejects_bad_ledger_c_empty_negative_control():
    """TD-BAD-C: negative_control empty → MUST exit 1."""
    ledger = WORKSPACE / 'tests/fixtures/ledger_bad.md'
    g = audit_gate.Gate(WORKSPACE)
    tds = audit_gate.parse_ledger(ledger)
    td_c = next((t for t in tds if t.get('td_id') == 'TD-BAD-C'), None)
    assert td_c is not None, "TD-BAD-C not found in bad ledger fixture"
    g.check(td_c)
    criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
    assert len(criticals) >= 1, (
        "GATE BROKEN: TD-BAD-C (empty negative_control) was not rejected. "
        "Non-falsifiable gates would be accepted."
    )
    print("✅ test_gate_rejects_bad_ledger_c_empty_negative_control: PASS")


def test_gate_rejects_full_bad_ledger_exit_1():
    """Running the gate on the full bad ledger must return exit code 1."""
    ledger = WORKSPACE / 'tests/fixtures/ledger_bad.md'
    exit_code, findings = run_gate(ledger)
    criticals = [f for f in findings if f[0] == audit_gate.CRITICAL]
    assert exit_code == 1, (
        f"GATE BROKEN: bad ledger returned exit {exit_code}, expected 1. "
        f"Criticals found: {criticals}"
    )
    assert len(criticals) >= 3, (
        f"Expected ≥3 Criticals (one per bad TD), got {len(criticals)}: {criticals}"
    )
    print(f"✅ test_gate_rejects_full_bad_ledger_exit_1: PASS ({len(criticals)} criticals)")


# ── Positive control: real ledger MUST pass ────────────────────────────────────

def test_gate_accepts_good_ledger():
    """The real technical-debt-ledger.md must pass the gate (exit 0)."""
    ledger = WORKSPACE / 'docs' / 'technical-debt-ledger.md'
    if not ledger.exists():
        print("⚠️  test_gate_accepts_good_ledger: SKIPPED (ledger not found)")
        return
    exit_code, findings = run_gate(ledger)
    criticals = [f for f in findings if f[0] == audit_gate.CRITICAL]
    assert exit_code == 0, (
        f"Real ledger failed the gate with {len(criticals)} Criticals:\n"
        + '\n'.join(f"  [{f[0]}] {f[1]}: {f[2]}" for f in criticals)
        + "\nFix these TDs before merging."
    )
    print(f"✅ test_gate_accepts_good_ledger: PASS (exit 0, {len(findings)} findings)")


# ── Unit tests for helpers ─────────────────────────────────────────────────────

def test_extract_passed_count():
    assert audit_gate.extract_passed_count("test result: ok. 5 passed; 0 failed") == 5
    assert audit_gate.extract_passed_count("test result: ok. 0 passed; 0 failed") == 0
    assert audit_gate.extract_passed_count("no result here") == -1
    assert audit_gate.extract_passed_count("12 passed; 0 failed; 1 ignored") == 12
    print("✅ test_extract_passed_count: PASS")


def test_evidence_git_hash():
    assert audit_gate.evidence_git_hash("# git-hash: abc1234\nresult") == "abc1234"
    assert audit_gate.evidence_git_hash("no hash here") is None
    print("✅ test_evidence_git_hash: PASS")


def test_pending_verification_within_deadline_is_ok():
    """pending-verification with future review_by must NOT be Critical."""
    future = "2099-12-31"
    ledger_text = textwrap.dedent(f"""
    ```yaml
    td_id:      TD-PENDING-OK
    status:     pending-verification
    review_by:  {future}
    pending_gate: Miri concurrency test
    ```
    """)
    tmp = make_ledger(ledger_text)
    try:
        g = audit_gate.Gate(WORKSPACE)
        tds = audit_gate.parse_ledger(tmp)
        g.check(tds[0])
        criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
        assert len(criticals) == 0, f"Future review_by must not be Critical: {criticals}"
        print("✅ test_pending_verification_within_deadline_is_ok: PASS")
    finally:
        tmp.unlink()


def test_pending_verification_past_deadline_is_critical():
    """pending-verification with past review_by MUST be Critical."""
    past = "2020-01-01"
    ledger_text = textwrap.dedent(f"""
    ```yaml
    td_id:      TD-PENDING-STALE
    status:     pending-verification
    review_by:  {past}
    pending_gate: some gate
    ```
    """)
    tmp = make_ledger(ledger_text)
    try:
        g = audit_gate.Gate(WORKSPACE)
        tds = audit_gate.parse_ledger(tmp)
        g.check(tds[0])
        criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
        assert len(criticals) >= 1, "Past review_by must be Critical"
        print("✅ test_pending_verification_past_deadline_is_critical: PASS")
    finally:
        tmp.unlink()


# ── Runner ─────────────────────────────────────────────────────────────────────

TESTS = [
    test_extract_passed_count,
    test_evidence_git_hash,
    test_gate_rejects_bad_ledger_a_no_evidence,
    test_gate_rejects_bad_ledger_b_zero_passed,
    test_gate_rejects_bad_ledger_c_empty_negative_control,
    test_gate_rejects_full_bad_ledger_exit_1,
    test_pending_verification_within_deadline_is_ok,
    test_pending_verification_past_deadline_is_critical,
    test_gate_accepts_good_ledger,  # last — depends on real ledger state
]


def main() -> int:
    print(f"\n{'='*60}")
    print("LUMYX Audit Gate — self-verification tests (KB-012)")
    print(f"{'='*60}\n")
    passed = failed = 0
    for test in TESTS:
        try:
            test()
            passed += 1
        except AssertionError as e:
            print(f"❌ {test.__name__}: FAIL\n   {e}")
            failed += 1
        except Exception as e:
            print(f"💥 {test.__name__}: ERROR — {type(e).__name__}: {e}")
            failed += 1
    print(f"\n{'='*60}")
    print(f"{'='*60}")
    print(f"Tests: {passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
