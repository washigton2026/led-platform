#!/usr/bin/env python3
"""
LUMYX Audit Gate — KB-012 enforcement.

Reads docs/technical-debt-ledger.md and enforces:
  1. Every TD with status=closed MUST have evidence_ref + negative_control.
  2. evidence_ref must point to an existing file that contains a passing result.
  3. TD with status=pending-verification blocks with exit code 1 (Critical).
  4. TD missing evidence_ref or negative_control → reported as unsubstantiated.

Exit codes:
  0 — gate passes (no Critical findings)
  1 — gate fails (Critical: unsubstantiated closed TD, or pending-verification TD)

Usage:
  python3 scripts/audit_gate.py [--workspace PATH]
  python3 scripts/audit_gate.py --check-td TD-002
"""

import re
import sys
import os
import argparse
from pathlib import Path


def parse_ledger(ledger_path: Path) -> list[dict]:
    """Parse the technical-debt-ledger.md into a list of TD dicts."""
    text = ledger_path.read_text()
    tds = []
    # Each TD block starts with ```yaml and ends with ```
    blocks = re.findall(r'```yaml\n(.*?)```', text, re.DOTALL)
    for block in blocks:
        td = {}
        # Parse simple key: value pairs (multi-line values are skipped for now)
        for line in block.splitlines():
            m = re.match(r'^(\w+):\s+(.+)', line)
            if m:
                key, val = m.group(1).strip(), m.group(2).strip()
                td[key] = val
        if 'td_id' in td:
            tds.append(td)
    return tds


CRITICAL = 'CRITICAL'
WARNING  = 'WARNING'

findings = []

def report(level: str, td_id: str, message: str):
    findings.append((level, td_id, message))
    prefix = '🔴' if level == CRITICAL else '🟡'
    print(f"{prefix} [{level}] {td_id}: {message}")


def check_td(td: dict, workspace: Path):
    td_id  = td.get('td_id', '?')
    status = td.get('status', '').strip().lower()

    # pending-verification blocks immediately
    if status == 'pending-verification':
        report(CRITICAL, td_id,
               f"status=pending-verification — evidence gate not yet passed. "
               f"pending_gate: {td.get('pending_gate', '(not specified)')}")
        return

    if status != 'closed':
        return  # open/diagnosed/wontfix — nothing to enforce here

    # status=closed: enforce evidence_ref + negative_control
    evidence_ref     = td.get('evidence_ref', '').strip()
    negative_control = td.get('negative_control', '').strip()

    missing = []
    if not evidence_ref:
        missing.append('evidence_ref')
    if not negative_control:
        missing.append('negative_control')

    if missing:
        report(CRITICAL, td_id,
               f"status=closed without required fields: {', '.join(missing)}. "
               f"Add evidence_ref (path to committed artefact) and "
               f"negative_control (description of run that would FAIL). "
               f"See KB-012.")
        return

    # evidence_ref exists and is a non-empty string — check the file
    ref_path = workspace / evidence_ref
    if not ref_path.exists():
        report(CRITICAL, td_id,
               f"evidence_ref '{evidence_ref}' does not exist at {ref_path}. "
               f"Commit the artefact or update the path.")
        return

    content = ref_path.read_text()
    # Look for any passing result marker
    has_pass = bool(re.search(
        r'(test result: ok\.|passed.*0 failed|\bpassed\b.*\b[1-9]\d*\b|result: ok)',
        content, re.IGNORECASE
    ))
    if not has_pass:
        report(CRITICAL, td_id,
               f"evidence_ref '{evidence_ref}' exists but contains no passing "
               f"test result (expected 'result: ok' or 'N passed; 0 failed'). "
               f"Re-run the gate and commit fresh evidence.")
        return

    # Check N > 0
    m = re.search(r'(\d+) passed', content)
    if m and int(m.group(1)) == 0:
        report(CRITICAL, td_id,
               f"evidence_ref '{evidence_ref}' shows 0 tests passed — "
               f"gate ran but exercised nothing (KB-012: Miri N=0 pattern).")
        return

    print(f"✅ [OK]      {td_id}: closed — evidence_ref verified, negative_control present.")


def main():
    parser = argparse.ArgumentParser(description='LUMYX Audit Gate (KB-012)')
    parser.add_argument('--workspace', default='.', help='Path to workspace root')
    parser.add_argument('--check-td', help='Check a single TD by id')
    args = parser.parse_args()

    workspace = Path(args.workspace).resolve()
    ledger    = workspace / 'docs' / 'technical-debt-ledger.md'

    if not ledger.exists():
        print(f"ERROR: ledger not found at {ledger}", file=sys.stderr)
        sys.exit(2)

    tds = parse_ledger(ledger)
    if not tds:
        print("WARNING: no TD entries found in ledger", file=sys.stderr)
        sys.exit(0)

    if args.check_td:
        tds = [td for td in tds if td.get('td_id') == args.check_td]
        if not tds:
            print(f"ERROR: {args.check_td} not found in ledger", file=sys.stderr)
            sys.exit(2)

    print(f"\nLUMYX Audit Gate — checking {len(tds)} TD entries\n")
    for td in tds:
        check_td(td, workspace)

    criticals = [f for f in findings if f[0] == CRITICAL]
    print(f"\n{'='*60}")
    print(f"Result: {len(criticals)} Critical, {len(findings)-len(criticals)} Warning")

    if criticals:
        print("Gate FAILED — fix Critical findings before closing TDs.")
        sys.exit(1)
    else:
        print("Gate PASSED.")
        sys.exit(0)


if __name__ == '__main__':
    main()
