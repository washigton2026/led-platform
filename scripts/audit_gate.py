#!/usr/bin/env python3
"""
LUMYX Audit Gate — KB-012 enforcement.

Enforces the closure schema for Technical Debt entries:
  1. status=closed requires evidence_ref + negative_control (both non-empty).
  2. evidence_ref must point to a committed file with N>0 tests passing.
  3. Evidence files record the git hash at generation; gate detects stale evidence
     when source files changed after that hash.
  4. status=pending-verification is valid state; becomes Critical if review_by has passed.
  5. "0 passed" / "0 tests" in evidence is explicitly rejected (KB-012: Miri N=0 pattern).
  6. For TDs with required_test: the named test must appear by name in the evidence file.

Exit codes:
  0 — gate passes (no Critical findings)
  1 — gate fails (Critical findings present)
  2 — usage/config error

Usage:
  python3 scripts/audit_gate.py [--workspace PATH] [--ledger PATH] [--check-td ID]
"""

from __future__ import annotations
import re
import sys
import subprocess
import argparse
from datetime import date, datetime
from pathlib import Path

# ── Severity ──────────────────────────────────────────────────────────────────
CRITICAL = 'CRITICAL'
WARNING  = 'WARNING'

# ── TD fields that require multi-line parsing ─────────────────────────────────
MULTILINE_KEYS = {'evidence_ref', 'negative_control', 'pending_gate', 'required_test',
                  'review_by', 'fixed_in', 'source_files'}


# ── Ledger parser ──────────────────────────────────────────────────────────────

def parse_ledger(ledger_path: Path) -> list[dict]:
    """
    Parse ```yaml ... ``` blocks in the ledger into TD dicts.
    Supports single-line and pipe-continuation multi-line values.
    """
    text = ledger_path.read_text()
    tds: list[dict] = []

    for block in re.findall(r'```yaml\n(.*?)```', text, re.DOTALL):
        td: dict = {}
        current_key: str | None = None
        current_lines: list[str] = []

        def flush():
            if current_key:
                td[current_key] = '\n'.join(current_lines).strip()

        for raw in block.splitlines():
            # Continuation line (starts with 2+ spaces)
            if raw.startswith('  ') and current_key:
                current_lines.append(raw.strip())
                continue
            flush()
            current_key, current_lines = None, []
            # Key: value  OR  key: |
            m = re.match(r'^(\w[\w_]*):\s*(.*)', raw)
            if not m:
                continue
            key, val = m.group(1).strip(), m.group(2).strip()
            if val == '|':          # pipe-block — collect following lines
                current_key = key
            else:
                td[key] = val
        flush()
        if 'td_id' in td:
            tds.append(td)
    return tds


# ── Evidence helpers ───────────────────────────────────────────────────────────

_GIT_HASH_RE = re.compile(r'git-hash:\s*([0-9a-f]{7,40})', re.IGNORECASE)
_PASSED_RE   = re.compile(r'(\d+)\s+passed;\s*0\s+failed', re.IGNORECASE)
_RESULT_OK   = re.compile(r'test result:\s*ok\.?\s+(\d+)\s+passed', re.IGNORECASE)


def extract_passed_count(content: str) -> int:
    """Return the highest N from 'N passed; 0 failed' lines, or -1 if absent."""
    counts = [int(m.group(1)) for m in _PASSED_RE.finditer(content)]
    counts += [int(m.group(1)) for m in _RESULT_OK.finditer(content)]
    return max(counts) if counts else -1


def evidence_git_hash(content: str) -> str | None:
    """Extract the git-hash: line from an evidence file header."""
    m = _GIT_HASH_RE.search(content)
    return m.group(1) if m else None


def files_changed_since(workspace: Path, git_hash: str, paths: list[str]) -> list[str]:
    """Return which paths have commits newer than git_hash."""
    changed: list[str] = []
    for p in paths:
        try:
            result = subprocess.run(
                ['git', 'log', '--oneline', f'{git_hash}..HEAD', '--', p],
                capture_output=True, text=True, cwd=workspace
            )
            if result.stdout.strip():
                changed.append(p)
        except Exception:
            pass  # git unavailable — skip stale check
    return changed


# ── Gate logic ─────────────────────────────────────────────────────────────────

class Gate:
    def __init__(self, workspace: Path):
        self.workspace = workspace
        self.findings: list[tuple[str, str, str]] = []

    def report(self, level: str, td_id: str, msg: str) -> None:
        self.findings.append((level, td_id, msg))
        icon = '🔴' if level == CRITICAL else '🟡'
        print(f"  {icon} [{level}] {td_id}: {msg}")

    def ok(self, td_id: str, msg: str) -> None:
        print(f"  ✅ [OK]      {td_id}: {msg}")

    def check(self, td: dict) -> None:
        td_id  = td.get('td_id', '?')
        status = td.get('status', '').strip().lower()

        # ── pending-verification ──────────────────────────────────────────────
        if status == 'pending-verification':
            review_by = td.get('review_by', '').strip()
            if review_by:
                try:
                    deadline = date.fromisoformat(review_by)
                    if date.today() > deadline:
                        self.report(CRITICAL, td_id,
                            f"pending-verification past review_by {review_by} — "
                            f"debt rotting. Complete the evidence gate or reopen as open.")
                        return
                except ValueError:
                    pass
            # Within review_by (or no deadline) — valid transient state
            gate_desc = td.get('pending_gate', '(not specified)')
            self.ok(td_id, f"pending-verification (valid) — gate: {gate_desc}")
            return

        # ── not closed — nothing to enforce ───────────────────────────────────
        if status != 'closed':
            return

        # ── closed: enforce evidence_ref + negative_control ───────────────────
        evidence_ref     = td.get('evidence_ref', '').strip()
        negative_control = td.get('negative_control', '').strip()

        missing = []
        if not evidence_ref:
            missing.append('evidence_ref')
        if not negative_control:
            missing.append('negative_control')
        if missing:
            self.report(CRITICAL, td_id,
                f"closed without required fields: {', '.join(missing)}. "
                f"(KB-012: every closed TD needs evidence_ref + negative_control)")
            return

        # ── evidence file exists ───────────────────────────────────────────────
        ref_path = self.workspace / evidence_ref
        if not ref_path.exists():
            self.report(CRITICAL, td_id,
                f"evidence_ref '{evidence_ref}' not found at {ref_path}. "
                f"Commit the artefact or update the path.")
            return

        content = ref_path.read_text()

        # ── require N > 0 passed ──────────────────────────────────────────────
        # Check max first: if ANY result line has N>0 the evidence is substantive.
        n_passed = extract_passed_count(content)
        if n_passed < 0:
            self.report(CRITICAL, td_id,
                f"evidence_ref contains no 'N passed; 0 failed' line at all. "
                f"Expected 'test result: ok. N passed' (N≥1). "
                f"Re-run the verification and commit the output.")
            return
        if n_passed == 0:
            # All result lines show 0 — KB-012 Miri N=0 pattern
            self.report(CRITICAL, td_id,
                f"evidence_ref shows only 0 tests passed — gate ran but exercised "
                f"nothing (KB-012: Miri N=0 pattern). Re-run with N>0.")
            return

        # ── optional: required_test must appear by name ───────────────────────
        required_test = td.get('required_test', '').strip()
        if required_test and required_test not in content:
            self.report(CRITICAL, td_id,
                f"required_test '{required_test}' not found in evidence_ref. "
                f"The named test must appear by name (verifies the right test ran).")
            return

        # ── stale evidence check (git-hash in artefact header) ────────────────
        source_files = td.get('source_files', '').strip()
        ev_hash = evidence_git_hash(content)
        if ev_hash and source_files:
            src_list = [s.strip() for s in source_files.split(',') if s.strip()]
            stale = files_changed_since(self.workspace, ev_hash, src_list)
            if stale:
                self.report(CRITICAL, td_id,
                    f"evidence is stale — source files changed after evidence was "
                    f"generated (hash {ev_hash[:8]}): {stale}. "
                    f"Re-run verification and commit updated evidence_ref.")
                return

        self.ok(td_id,
            f"closed — {n_passed} tests passed, negative_control present"
            + (f", required_test '{required_test}' found" if required_test else ""))

    def run(self, tds: list[dict]) -> int:
        print(f"\nLUMYX Audit Gate (KB-012) — {len(tds)} TD entries\n")
        for td in tds:
            self.check(td)
        criticals = [f for f in self.findings if f[0] == CRITICAL]
        print(f"\n{'='*60}")
        print(f"Result: {len(criticals)} Critical, "
              f"{len(self.findings)-len(criticals)} Warning, "
              f"{len(tds)-len(self.findings)} OK")
        if criticals:
            print("Gate FAILED — fix Critical findings before closing TDs.")
            return 1
        print("Gate PASSED.")
        return 0


# ── Main ───────────────────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='LUMYX Audit Gate (KB-012)')
    parser.add_argument('--workspace', default='.', help='Workspace root (default: .)')
    parser.add_argument('--ledger',    default=None, help='Override ledger path')
    parser.add_argument('--check-td',  default=None, help='Check a single TD by id')
    args = parser.parse_args(argv)

    workspace = Path(args.workspace).resolve()
    ledger    = Path(args.ledger).resolve() if args.ledger \
                else workspace / 'docs' / 'technical-debt-ledger.md'

    if not ledger.exists():
        print(f"ERROR: ledger not found at {ledger}", file=sys.stderr)
        return 2

    tds = parse_ledger(ledger)
    if not tds:
        print("WARNING: no TD entries found in ledger", file=sys.stderr)
        return 0

    if args.check_td:
        tds = [td for td in tds if td.get('td_id') == args.check_td]
        if not tds:
            print(f"ERROR: {args.check_td} not found in ledger", file=sys.stderr)
            return 2

    gate = Gate(workspace)
    return gate.run(tds)


if __name__ == '__main__':
    sys.exit(main())
