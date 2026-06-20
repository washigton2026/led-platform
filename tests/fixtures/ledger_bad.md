# Bad Ledger Fixture — used by test_audit_gate.py
# Contains 3 TDs that the gate MUST reject (exit 1).
# If the gate passes this ledger, the gate itself is broken.

## TD-BAD-A — closed without evidence_ref or negative_control

```yaml
td_id:     TD-BAD-A
title:     "Closed with no evidence at all"
severity:  High
status:    closed
closed_on: 2026-06-19
fix: |
  Some fix was applied.
```

---

## TD-BAD-B — evidence_ref points to file with 0 passed

```yaml
td_id:          TD-BAD-B
title:          "Evidence file shows 0 tests passed"
severity:       High
status:         closed
closed_on:      2026-06-19
evidence_ref:   tests/fixtures/evidence_zero_passed.txt
negative_control: "assert_eq!(x, 1) would fail if x != 1"
```

---

## TD-BAD-C — negative_control is empty

```yaml
td_id:          TD-BAD-C
title:          "negative_control field is present but empty"
severity:       High
status:         closed
closed_on:      2026-06-19
evidence_ref:   tests/fixtures/evidence_good.txt
negative_control:
```
