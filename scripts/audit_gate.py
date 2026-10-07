#!/usr/bin/env python3
"""
LUMYX Audit Gate — KB-012 enforcement.

Enforces the closure schema for Technical Debt entries:
  1. status=closed requires evidence_ref + negative_control (both non-empty).
  2. evidence_ref must point to a committed file with N>0 tests passing.
  3. Evidence files pin the CONTENT they were generated against: one line per watched file,
     `watched: <path> sha256:<hex>`. Every source_file of the TD must be pinned, and the
     evidence is stale iff the file's current content hashes differently (TD-028). No git
     is involved, so a shallow clone cannot turn the check green (TD-026); an unreadable or
     missing watched file is Critical («not verifiable»), never «unchanged». Only lines that
     START with `watched:` pin; a path pinned twice or outside the workspace is Critical.
     LIMITS (by design, declared): the gate proves that the evidence was regenerated for the
     current content, NOT that the verification was re-run — editing only the sha passes.
     And the gate watches itself: any change to scripts/audit_gate.py or
     tests/test_audit_gate.py must regenerate the TD-026/TD-028 evidence in the same commit.
     A closed TD without source_files is not watched at all. Result lines inside ```
     code blocks count (that is where raw output lives). The author can still put a lie
     INSIDE the proof region — the region is a declaration, reviewed like the sha.
  4. status=pending-verification is valid state; becomes Critical if review_by has passed.
  5. "0 passed" / "0 tests" in evidence is explicitly rejected (KB-012: Miri N=0 pattern).
  6. For TDs with required_test: the evidence declares ONE proof region (`--- prova ---` …
     `--- fim da prova ---`), the run that proves the fix. Inside it the named test must have
     a STRUCTURED RESULT LINE that passed — `test [path::]<name> ... ok` or `<name>: N passed;
     0 failed` (N>0) — no FAILED for it, and an `N passed; 0 failed` summary with N>0. Nothing
     outside the region counts (negative controls, prose, quotes live there by construction).
     A closed TD WITHOUT required_test keeps the older, weaker rule: any `N passed; 0 failed`
     line anywhere in the evidence (N>0) — there is no named test to prove.

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
import hashlib
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


_WATCHED_RE = re.compile(r'^watched:[ \t]*(\S+)[ \t]+sha256:([0-9a-f]{64})[ \t]*$', re.MULTILINE)
# Um `<!--` sem fecho esconde tudo até ao fim no markdown renderizado — e aqui também.
_HTML_COMMENT = re.compile(r'<!--.*?(?:-->|\Z)', re.DOTALL)
# A REGIÃO DE PROVA (lista positiva, R4.1 após 3 rondas do falsificador): a evidência declara
# explicitamente o bloco que é o run real. Só ele conta para o required_test — controlos
# negativos, prosa, títulos e citações ficam fora por construção, sem lista de formas a excluir.
PROVA_INICIO = '--- prova ---'
PROVA_FIM = '--- fim da prova ---'


def _sem_comentarios_html(content: str) -> str:
    return _HTML_COMMENT.sub('', content)


def evidence_watched(content: str) -> dict[str, str]:
    """`watched: <path> sha256:<hex>` lines of an evidence file → {path: sha256}. Only lines
    that START with `watched:` count — a commented or quoted line pins nothing."""
    return {m.group(1): m.group(2) for m in _WATCHED_RE.finditer(_sem_comentarios_html(content))}


def watched_problems(content: str) -> list[str]:
    """Pins that cannot be trusted: the same path pinned twice (which one would win?), and paths
    that escape the workspace (absolute or with `..`) — in the pre-commit those would read the
    working tree instead of the index."""
    vistos: dict[str, int] = {}
    for m in _WATCHED_RE.finditer(_sem_comentarios_html(content)):
        vistos[m.group(1)] = vistos.get(m.group(1), 0) + 1
    probs = [f"{p} pinned {n}x" for p, n in vistos.items() if n > 1]
    probs += [f"{p} escapes the workspace" for p in vistos
              if p.startswith('/') or '..' in Path(p).parts]
    return probs


def escapa_por_symlink(workspace: Path, path: str) -> bool:
    """Um caminho textualmente limpo pode sair do workspace por um symlink commitado."""
    try:
        (workspace / path).resolve().relative_to(workspace.resolve())
        return False
    except ValueError:
        return True


def sha256_of(workspace: Path, path: str) -> str:
    """sha256 of a workspace file. Raises OSError if it cannot be read — the caller turns
    that into Critical («not verifiable»): an unreadable file is never «unchanged»."""
    return hashlib.sha256((workspace / path).read_bytes()).hexdigest()


def stale_by_content(workspace: Path, watched: dict[str, str]) -> tuple[list[str], list[str]]:
    """Return (changed, unverifiable) among the watched paths, by CONTENT (TD-028/TD-026)."""
    changed: list[str] = []
    unverifiable: list[str] = []
    for path, recorded in sorted(watched.items()):
        try:
            if sha256_of(workspace, path) != recorded:
                changed.append(path)
        except OSError as e:
            pass  # NEG R4.9: regra partida de propósito (ficheiro ilegível passa)
    return changed, unverifiable


# The only prefix a result line may carry: none, or the GitHub Actions log prefix that
# `gh run view --log` adds (`job<TAB>step<TAB>timestamp `). Anything else — `NEG:`, `#`, `>`,
# prose — means the line is ABOUT a result, not a result: a negative control's `... ok` against
# the old code must never prove that the test passed against the new one.
_CAMPO_LOG = r'[A-Za-z0-9][^\t\n]*'      # job/step do log do GitHub Actions
_PREFIXO_RESULTADO = (r'(?:[ \t]*|' + _CAMPO_LOG + r'\t' + _CAMPO_LOG
                      + r'\t[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z[ \t]+)')


def regiao_de_prova(content: str) -> tuple[str | None, str]:
    """Devolve (região, '') ou (None, porquê). Exige EXATAMENTE um par de marcadores, em linhas
    próprias, fora de comentários HTML, e o fim depois do início."""
    linhas = _sem_comentarios_html(content).splitlines()
    ini = [i for i, l in enumerate(linhas) if l.strip() == PROVA_INICIO]
    fim = [i for i, l in enumerate(linhas) if l.strip() == PROVA_FIM]
    if len(ini) != 1 or len(fim) != 1:
        return None, (f"evidence must declare exactly ONE proof region ('{PROVA_INICIO}' … "
                      f"'{PROVA_FIM}'); found {len(ini)} start / {len(fim)} end markers")
    if fim[0] <= ini[0]:
        return None, "proof region ends before it starts"
    return '\n'.join(linhas[ini[0] + 1:fim[0]]) + '\n', ''


def required_test_failed(content: str, name: str) -> bool:
    """True iff the text reports `name` as FAILED (libtest) or with failures (harness)."""
    n = re.escape(name)
    if re.search(rf'^.*\btest[ \t]+(?:\S+::)?{n}[ \t]+\.\.\.[ \t]+FAILED', content, re.MULTILINE):
        return True
    return bool(re.search(rf'^.*\b{n}:[ \t]*[0-9]+[ \t]+passed;[ \t]*[1-9][0-9]*[ \t]+failed', content, re.MULTILINE))


def required_test_passed(content: str, name: str) -> bool:
    """True iff a STRUCTURED result line reports `name` as passed: libtest
    `test [path::]name ... ok`, or a harness summary `name: N passed; 0 failed` with N > 0.
    The line may only carry the CI log prefix (see `_PREFIXO_RESULTADO`); HTML comments are
    ignored."""
    content = _sem_comentarios_html(content)
    n = re.escape(name)
    libtest = re.compile(rf'^{_PREFIXO_RESULTADO}test[ \t]+(?:\S+::)?{n}[ \t]+\.\.\.[ \t]+ok[ \t]*\r?$',
                         re.MULTILINE)
    # Estrito: a linha ACABA em `0 failed` — `; 1 crashed`, `; 2 panicked`, `0 failedX` não contam.
    harness = re.compile(rf'^{_PREFIXO_RESULTADO}{n}:[ \t]*([0-9]+)[ \t]+passed;[ \t]*0[ \t]+failed[ \t]*\r?$',
                         re.MULTILINE)
    if libtest.search(content):
        return True
    return any(int(m.group(1)) > 0 for m in harness.finditer(content))


# ── Gate logic ─────────────────────────────────────────────────────────────────

class Gate:
    def __init__(self, workspace: Path):
        self.workspace = workspace
        self.findings: list[tuple[str, str, str]] = []
        # TD que o gate não fiscaliza (open, wontfix, …): não são achados, mas também não
        # são «OK». Contá-los como OK fazia «20 OK» esconder 6 TD abertos.
        self.nao_fechados: list[tuple[str, str]] = []

    def visivel(self, td_id: str, status: str, td: dict) -> None:
        self.nao_fechados.append((status or '(sem status)', td_id))
        icone = {'open': '⬜', 'wontfix': '⏸ '}.get(status, '❔')
        etiqueta = f"[{(status or 'sem status').upper()}]"
        sev = td.get('severity', '?').strip() or '?'
        titulo = td.get('title', '').strip().strip('"')
        if len(titulo) > 90:
            titulo = titulo[:87] + '...'
        print(f"  {icone} {etiqueta:<9} {td_id}: {status or 'sem status'} — {sev} — {titulo}")

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
                    # NAO engolir. Um review_by ilegivel nao e "sem prazo": e um prazo
                    # que o autor pediu e que o gate deixou cair em silencio, desligando
                    # o detector de apodrecimento (KB-012). Causa habitual: aspas — o
                    # parse_ledger guarda o valor verbatim, e o ledger cita escalares de
                    # uma linha, logo '"2026-10-07"' chega assim a fromisoformat().
                    self.report(CRITICAL, td_id,
                        f"review_by {review_by!r} nao e uma data ISO (YYYY-MM-DD) — "
                        f"o prazo nao e verificavel e a verificacao nao faz nada. "
                        f"Escrever sem aspas: review_by: 2026-10-07")
                    return
            # Within review_by (or no deadline) — valid transient state
            gate_desc = td.get('pending_gate', '(not specified)')
            self.ok(td_id, f"pending-verification (valid) — gate: {gate_desc}")
            return

        # ── not closed — nothing to enforce, mas VISÍVEL (nunca contado como OK) ─
        if status != 'closed':
            self.visivel(td_id, status, td)
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

        # ── optional: required_test must have PASSED inside the PROOF REGION ───
        required_test = td.get('required_test', '').strip()
        if required_test:
            prova, porque = regiao_de_prova(content)
            if prova is None:
                self.report(CRITICAL, td_id, porque + " — only the proof region can prove required_test.")
                return
            if extract_passed_count(prova) <= 0:
                self.report(CRITICAL, td_id,
                    "the proof region has no 'N passed; 0 failed' summary with N>0.")
                return
            if required_test_failed(prova, required_test):
                self.report(CRITICAL, td_id,
                    f"required_test '{required_test}' is reported FAILED inside the proof region.")
                return
            if not required_test_passed(prova, required_test):
                self.report(CRITICAL, td_id,
                    f"required_test '{required_test}' has no passing result line in the proof region "
                    f"(`test [path::]{required_test} ... ok` or `{required_test}: N passed; 0 failed`, "
                    f"no prefix except the CI log one).")
                return

        # ── stale evidence check, by CONTENT (TD-028) and without git (TD-026) ─
        source_files = td.get('source_files', '').strip()
        src_list = [s.strip() for s in source_files.split(',') if s.strip()]
        watched = evidence_watched(content)
        problemas = watched_problems(content)
        if problemas:
            self.report(CRITICAL, td_id, f"untrustworthy watched: lines: {problemas}.")
            return
        fora = [p for p in watched if escapa_por_symlink(self.workspace, p)]
        if fora:
            self.report(CRITICAL, td_id, f"watched paths resolve outside the workspace (symlink): {fora}.")
            return
        unpinned = [p for p in src_list if p not in watched]
        if unpinned:
            self.report(CRITICAL, td_id,
                f"evidence does not pin the content of source files {unpinned}: add "
                f"`watched: <path> sha256:<hex>` lines, computed when the evidence was generated.")
            return
        changed, unverifiable = stale_by_content(self.workspace, watched)
        if unverifiable:
            self.report(CRITICAL, td_id,
                f"watched files are not verifiable (never read as «unchanged»): {unverifiable}.")
            return
        if changed:
            self.report(CRITICAL, td_id,
                f"evidence is stale — content of watched files differs from what the evidence "
                f"was generated against: {changed}. Re-run verification and regenerate "
                f"evidence_ref in the same commit.")
            return

        self.ok(td_id,
            f"closed — {n_passed} tests passed, negative_control present"
            + (f", required_test '{required_test}' found" if required_test else ""))

    def run(self, tds: list[dict]) -> int:
        print(f"\nLUMYX Audit Gate (KB-012) — {len(tds)} TD entries\n")
        for td in tds:
            self.check(td)
        criticals = [f for f in self.findings if f[0] == CRITICAL]
        por_status: dict[str, int] = {}
        for st, _ in self.nao_fechados:
            por_status[st] = por_status.get(st, 0) + 1
        resto = ''.join(f" · {n} {st}" for st, n in sorted(por_status.items()))
        print(f"\n{'='*60}")
        print(f"Result: {len(criticals)} Critical, "
              f"{len(self.findings)-len(criticals)} Warning, "
              f"{len(tds)-len(self.findings)-len(self.nao_fechados)} OK{resto}")
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
