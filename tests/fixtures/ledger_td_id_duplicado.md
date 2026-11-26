# td_id duplicado — fixture do TD-037, usada por tests/test_audit_gate.py
# Reproduz a forma real de 8e64f68 (merge do #64 em fix/td-035-fanout-tempo): duas entradas
# com o mesmo td_id, uma open e outra pending-verification. Cada uma passa sozinha no gate;
# juntas, o gate TEM de reprovar.

## TD-DUP-A — primeira entrada (open)

```yaml
td_id:     TD-DUP-A
title:     "primeira divida a ocupar o numero"
severity:  Medium
status:    open
```

---

## TD-DUP-A — segunda entrada (pending-verification), registada noutro ramo

```yaml
td_id:     TD-DUP-A
title:     "segunda divida que escolheu o mesmo numero"
severity:  Medium
status:    pending-verification
pending_gate: |
  gate de teste
review_by: 2099-12-31
```
