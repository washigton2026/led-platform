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


def test_pending_verification_malformed_review_by_is_critical():
    """Um review_by ilegivel NAO pode passar como 'sem prazo'.

    A data e FUTURA de proposito: se a verificacao so disparasse por o prazo ter
    passado, este caso ficaria verde. O que tem de a fazer disparar e a data ser
    ILEGIVEL, independentemente do que ela diria.

    E citada de proposito, porque e a forma que o ledger real escreve — as duas
    auto-testes vizinhas usam datas sem aspas, e era so essa forma que o gate
    alguma vez tinha visto. Medido em 2026-09-23: com aspas, date.fromisoformat
    levantava ValueError, o `except` engolia-o, e um TD-022 com prazo vencido
    teria ficado OK para sempre.
    """
    quoted_future = '"2099-12-31"'
    ledger_text = textwrap.dedent(f"""
    ```yaml
    td_id:      TD-PENDING-MALFORMED
    status:     pending-verification
    review_by:  {quoted_future}
    pending_gate: some gate
    ```
    """)
    tmp = make_ledger(ledger_text)
    try:
        g = audit_gate.Gate(WORKSPACE)
        tds = audit_gate.parse_ledger(tmp)
        g.check(tds[0])
        criticals = [f for f in g.findings if f[0] == audit_gate.CRITICAL]
        assert len(criticals) >= 1, "review_by ilegivel tem de ser Critical"
        print("✅ test_pending_verification_malformed_review_by_is_critical: PASS")
    finally:
        tmp.unlink()


# ── Runner ─────────────────────────────────────────────────────────────────────

def test_open_and_wontfix_are_visible_not_ok():
    """open/wontfix não são fiscalizados, mas têm de APARECER e não podem contar como OK.

    Antes: `if status != 'closed': return` em silêncio, e o resumo contava-os como OK —
    o ledger real dava «20 OK» com 6 TD abertos. O exit não muda (não são Critical).
    """
    import io, contextlib
    ledger_text = textwrap.dedent("""
    ```yaml
    td_id:     TD-VIS-OPEN
    title:     "divida aberta de teste"
    severity:  High
    status:    open
    ```

    ```yaml
    td_id:     TD-VIS-WONTFIX
    title:     "divida aceite de teste"
    severity:  Low
    status:    wontfix
    ```
    """)
    tmp = make_ledger(ledger_text)
    try:
        g = audit_gate.Gate(WORKSPACE)
        tds = audit_gate.parse_ledger(tmp)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = g.run(tds)
        texto = out.getvalue()
        assert code == 0, f"open/wontfix não são Critical — o exit tem de ser 0, foi {code}"
        assert '[OPEN]' in texto and 'TD-VIS-OPEN' in texto, f"open não impresso:\n{texto}"
        assert '[WONTFIX]' in texto and 'TD-VIS-WONTFIX' in texto, f"wontfix não impresso:\n{texto}"
        assert '0 OK · 1 open · 1 wontfix' in texto, f"resumo conta open/wontfix como OK:\n{texto}"
        print("✅ test_open_and_wontfix_are_visible_not_ok: PASS")
    finally:
        tmp.unlink()



# ── R4.1: required_test estruturado + stale por CONTEÚDO (TD-028) sem git (TD-026) ────

import hashlib
import shutil
import subprocess

def _sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def _workspace(evidence: str, source: str = 'src/alvo.rs', conteudo: str = 'fn alvo() {}\n',
               required_test: str = 'alvo_passa', pin: bool = True, extra_src: str = '') -> Path:
    """Workspace temporário com um TD fechado, a sua evidência e o ficheiro vigiado.
    `{PIN}` na evidência é substituído pela linha `watched:` do conteúdo ATUAL."""
    ws = Path(tempfile.mkdtemp(prefix='gate-r41-'))
    f = ws / source
    f.parent.mkdir(parents=True, exist_ok=True)
    f.write_text(conteudo)
    pin_line = f"watched: {source} sha256:{_sha(f)}" if pin else ''
    (ws / 'docs').mkdir()
    (ws / 'docs' / 'ev.md').write_text(evidence.replace('{PIN}', pin_line))
    srcs = source + (f", {extra_src}" if extra_src else '')
    (ws / 'docs' / 'technical-debt-ledger.md').write_text(textwrap.dedent(f"""
    ```yaml
    td_id:     TD-R41
    status:    closed
    evidence_ref: docs/ev.md
    required_test: {required_test}
    source_files: {srcs}
    negative_control: |
      mutacao -> vermelho
    ```
    """))
    return ws


def _gate(ws: Path) -> tuple[int, list]:
    g = audit_gate.Gate(ws)
    rc = g.run(audit_gate.parse_ledger(ws / 'docs' / 'technical-debt-ledger.md'))
    return rc, [f for f in g.findings if f[0] == audit_gate.CRITICAL]


EV_OK = "git-hash: abc1234\n{PIN}\ntest tests::alvo_passa ... ok\ntest result: ok. 1 passed; 0 failed\n"


def test_r41_evidencia_valida_passa():
    rc, crit = _gate(_workspace(EV_OK))
    assert rc == 0 and not crit, f"evidência válida reprovou: {crit}"
    print("✅ test_r41_evidencia_valida_passa: PASS")


def test_r41_required_test_so_em_comentario_e_vermelho():
    ev = "{PIN}\n# test tests::alvo_passa ... ok\ntest result: ok. 3 passed; 0 failed\n"
    rc, crit = _gate(_workspace(ev))
    assert rc == 1 and any('required_test' in c[2] for c in crit), crit
    print("✅ test_r41_required_test_so_em_comentario_e_vermelho: PASS")


def test_r41_required_test_so_failed_e_vermelho():
    ev = "{PIN}\nrequired_test: alvo_passa\ntest tests::alvo_passa ... FAILED\ntest result: ok. 3 passed; 0 failed\n"
    rc, crit = _gate(_workspace(ev))
    assert rc == 1 and any('required_test' in c[2] for c in crit), crit
    print("✅ test_r41_required_test_so_failed_e_vermelho: PASS")


def test_r41_n_zero_e_vermelho():
    ev = "{PIN}\ntest tests::alvo_passa ... ok\ntest result: ok. 0 passed; 0 failed\n"
    rc, crit = _gate(_workspace(ev))
    assert rc == 1 and any('0 tests passed' in c[2] or 'N=0' in c[2] for c in crit), crit
    rc, crit = _gate(_workspace("{PIN}\nalvo_passa: 0 passed; 0 failed\n"))
    assert rc == 1, "harness com N=0 não pode provar o required_test"
    print("✅ test_r41_n_zero_e_vermelho: PASS")


def test_r41_formatos_estruturados_aceites():
    ci = ("{PIN}\nmiri (led-triple)\tSTEP\t2026-10-05T05:41:30Z test ring::tests::alvo_passa ... ok\n"
          "test result: ok. 5 passed; 0 failed\n")
    assert _gate(_workspace(ci))[0] == 0, "linha libtest com prefixo de log CI tem de contar"
    assert _gate(_workspace("{PIN}\nalvo_passa: 4 passed; 0 failed\n"))[0] == 0, "resumo de harness"
    print("✅ test_r41_formatos_estruturados_aceites: PASS")


def test_r41_vigiado_alterado_sem_evidencia_nova_e_stale():
    ws = _workspace(EV_OK)
    (ws / 'src/alvo.rs').write_text('fn alvo() { mudou() }\n')
    rc, crit = _gate(ws)
    assert rc == 1 and any('stale' in c[2] for c in crit), crit
    print("✅ test_r41_vigiado_alterado_sem_evidencia_nova_e_stale: PASS")


def test_r41_alterado_com_evidencia_regenerada_no_mesmo_commit_e_verde():
    ws = _workspace(EV_OK)
    f = ws / 'src/alvo.rs'
    f.write_text('fn alvo() { mudou() }\n')
    ev = ws / 'docs/ev.md'
    ev.write_text(EV_OK.replace('{PIN}', f"watched: src/alvo.rs sha256:{_sha(f)}"))
    rc, crit = _gate(ws)
    assert rc == 0, f"evidência regenerada para o conteúdo novo tem de passar: {crit}"
    print("✅ test_r41_alterado_com_evidencia_regenerada_no_mesmo_commit_e_verde: PASS")


def test_r41_vigiado_inexistente_e_critical_nao_verificavel():
    ws = _workspace(EV_OK)
    (ws / 'src/alvo.rs').unlink()
    rc, crit = _gate(ws)
    assert rc == 1 and any('not verifiable' in c[2] for c in crit), crit
    print("✅ test_r41_vigiado_inexistente_e_critical_nao_verificavel: PASS")


def test_r41_source_file_sem_watched_e_critical():
    rc, crit = _gate(_workspace(EV_OK, pin=False))
    assert rc == 1 and any('does not pin' in c[2] for c in crit), crit
    print("✅ test_r41_source_file_sem_watched_e_critical: PASS")


def test_r41_ci_yml_e_declaravel():
    ws = _workspace(EV_OK, source='.github/workflows/ci.yml', conteudo='name: CI\n')
    assert _gate(ws)[0] == 0
    (ws / '.github/workflows/ci.yml').write_text('name: CI\n# passo Miri apagado\n')
    rc, crit = _gate(ws)
    assert rc == 1 and any('stale' in c[2] for c in crit), "apagar um passo do ci.yml vigiado tem de dar stale"
    print("✅ test_r41_ci_yml_e_declaravel: PASS")


def _git(ws: Path, *args: str) -> str:
    r = subprocess.run(['git', '-C', str(ws), '-c', 'user.email=t@t', '-c', 'user.name=t', *args],
                       capture_output=True, text=True)
    assert r.returncode == 0, f"git {args}: {r.stderr}"
    return r.stdout.strip()


def test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde():
    """TD-028: commits que tocam o ficheiro e voltam ao mesmo conteúdo NÃO tornam a evidência
    stale. O gate antigo (`git log <hash>..HEAD`) dava-o como stale (falso-vermelho)."""
    ws = _workspace(EV_OK)
    _git(ws, 'init', '-q')
    _git(ws, 'add', '-A'); _git(ws, 'commit', '-qm', 'A')
    # A evidência aponta para um commit REAL: só assim o gate antigo (`git log A..HEAD`) vê os
    # commits B e C e dá o falso-vermelho — com um hash inexistente ele falhava calado.
    hash_a = _git(ws, 'rev-parse', 'HEAD')
    ev = ws / 'docs/ev.md'
    ev.write_text(ev.read_text().replace('git-hash: abc1234', f'git-hash: {hash_a}'))
    _git(ws, 'commit', '-qam', 'evidencia aponta para A')
    original = (ws / 'src/alvo.rs').read_text()
    (ws / 'src/alvo.rs').write_text('fn alvo() { temporario() }\n')
    _git(ws, 'commit', '-qam', 'B: muda')
    (ws / 'src/alvo.rs').write_text(original)
    _git(ws, 'commit', '-qam', 'C: reverte')
    rc, crit = _gate(ws)
    assert rc == 0, f"historia sem mudanca de conteudo deu stale (TD-028): {crit}"
    print("✅ test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde: PASS")


def test_r41_td026_clone_raso_igual_a_completo():
    """TD-026: num clone raso o hash da evidência não existe. O gate antigo engolia a falha
    do `git log` e dava verde com o ficheiro MUDADO. Agora o veredito é igual ao do clone
    completo: verde com o conteúdo igual, stale com o conteúdo mudado."""
    origem = _workspace(EV_OK)
    _git(origem, 'init', '-q')
    _git(origem, 'add', '-A'); _git(origem, 'commit', '-qm', 'A')
    hash_a = _git(origem, 'rev-parse', 'HEAD')
    ev = origem / 'docs/ev.md'
    ev.write_text(ev.read_text().replace('git-hash: abc1234', f'git-hash: {hash_a}'))
    _git(origem, 'commit', '-qam', 'evidencia aponta para A')
    (origem / 'src/alvo.rs').write_text('fn alvo() { mudou() }\n')
    _git(origem, 'commit', '-qam', 'muda o vigiado')
    for _ in range(2):
        (origem / 'docs/x.txt').write_text(str(_)); _git(origem, 'add', '-A'); _git(origem, 'commit', '-qm', 'x')
    raso = Path(tempfile.mkdtemp(prefix='gate-r41-raso-')) / 'r'
    r = subprocess.run(['git', 'clone', '-q', '--depth', '1', f'file://{origem}', str(raso)],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    assert subprocess.run(['git', '-C', str(raso), 'cat-file', '-e', hash_a],
                          capture_output=True).returncode != 0, "premissa: o hash NAO existe no clone raso"
    rc_completo, _ = _gate(origem)
    rc_raso, crit = _gate(raso)
    assert rc_completo == 1 and rc_raso == 1, \
        f"vigiado mudado tem de ser stale nos DOIS clones (completo={rc_completo}, raso={rc_raso}): {crit}"
    print("✅ test_r41_td026_clone_raso_igual_a_completo: PASS")




# ── R4.1 (falsificador): mutantes que sobreviviam com o pin regenerado ────────────

def _req(ev: str) -> int:
    return _gate(_workspace(ev))[0]


def test_r41_neg_ok_com_real_failed_e_vermelho():
    """HIGH-1: o `... ok` do controlo NEGATIVO (contra o código antigo) nunca prova o teste."""
    ev = ("{PIN}\ntest t::alvo_passa ... FAILED\ntest result: FAILED. 4 passed; 1 failed\n"
          "NEG: test t::alvo_passa ... ok\nNEG: test result: ok. 5 passed; 0 failed\n")
    assert _req(ev) == 1, "linha NEG não pode contar como teste que passou"
    print("✅ test_r41_neg_ok_com_real_failed_e_vermelho: PASS")


def test_r41_so_prefixo_de_log_ci_e_aceite():
    base = "{PIN}\ntest result: ok. 3 passed; 0 failed\n"
    for linha in ["nota # test t::alvo_passa ... ok", "> test t::alvo_passa ... ok",
                  "o output dizia test t::alvo_passa ... ok", "<!-- test t::alvo_passa ... ok -->",
                  "<!--\ntest t::alvo_passa ... ok\n-->", "    # test t::alvo_passa ... ok",
                  "test t::alvo_passa ... ok extra"]:
        assert _req(base + linha + "\n") == 1, f"aceitou como resultado: {linha!r}"
    for linha in ["test t::alvo_passa ... ok", "   test alvo_passa ... ok",
                  "job x\tSTEP y\t2026-10-05T05:41:30.565Z test t::alvo_passa ... ok"]:
        assert _req(base + linha + "\n") == 0, f"recusou um resultado legítimo: {linha!r}"
    print("✅ test_r41_so_prefixo_de_log_ci_e_aceite: PASS")


def test_r41_harness_exige_n_maior_que_zero_e_zero_falhas_sem_comentario():
    """Mata M1 (harness lê comentários), M2 (N>=0) e M15 (aceita M failed > 0)."""
    resumo = "test result: ok. 9 passed; 0 failed\n"  # N>0 global: o harness tem de ser julgado sozinho
    assert _req("{PIN}\n" + resumo + "alvo_passa: 0 passed; 0 failed\n") == 1, "M2: N=0 no harness"
    assert _req("{PIN}\n" + resumo + "alvo_passa: 4 passed; 1 failed\n") == 1, "M15: harness com falhas"
    assert _req("{PIN}\n" + resumo + "alvo_passa: 4 passed; 10 failed\n") == 1, "M15: 10 failed"
    assert _req("{PIN}\n" + resumo + "# alvo_passa: 4 passed; 0 failed\n") == 1, "M1: comentado"
    assert _req("{PIN}\n" + resumo + "alvo_passa: 4 passed; 0 failed\n") == 0
    print("✅ test_r41_harness_exige_n_maior_que_zero_e_zero_falhas_sem_comentario: PASS")


def test_r41_nome_com_metacaracteres_e_literal():
    """Mata M5 (sem re.escape): `a.b` não pode casar com `aXb`."""
    ev = "{PIN}\ntest t::aXb ... ok\ntest result: ok. 1 passed; 0 failed\n"
    assert _gate(_workspace(ev, required_test='a.b'))[0] == 1
    ev = "{PIN}\ntest t::a.b ... ok\ntest result: ok. 1 passed; 0 failed\n"
    assert _gate(_workspace(ev, required_test='a.b'))[0] == 0
    print("✅ test_r41_nome_com_metacaracteres_e_literal: PASS")


def test_r41_vigiado_ilegivel_ou_diretorio_e_critical():
    """Mata M3/M4: PermissionError e IsADirectoryError nunca são «inalterado»."""
    import os
    ws = _workspace(EV_OK)
    f = ws / 'src/alvo.rs'
    if os.geteuid() != 0:          # root lê ficheiros 000 — aí não há erro a provar
        os.chmod(f, 0)
        try:
            rc, crit = _gate(ws)
        finally:
            os.chmod(f, 0o644)
        assert rc == 1 and any('not verifiable' in c[2] for c in crit), crit
    f.unlink(); f.mkdir()
    rc, crit = _gate(ws)
    assert rc == 1 and any('not verifiable' in c[2] for c in crit), crit
    print("✅ test_r41_vigiado_ilegivel_ou_diretorio_e_critical: PASS")


def test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado():
    """Mata M8 (stale só sobre source_files): um `watched:` extra (ex.: ci.yml) também conta."""
    ws = _workspace(EV_OK)
    extra = ws / 'ci.yml'
    extra.write_text('a\n')
    ev = ws / 'docs/ev.md'
    ev.write_text(ev.read_text() + f"watched: ci.yml sha256:{_sha(extra)}\n")
    assert _gate(ws)[0] == 0
    extra.write_text('b\n')
    rc, crit = _gate(ws)
    assert rc == 1 and any('ci.yml' in c[2] for c in crit), crit
    print("✅ test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado: PASS")


def test_r41_pins_nao_confiaveis_sao_critical():
    """`watched:` comentado não fixa; duplicado, absoluto ou com `..` → Critical."""
    rc, crit = _gate(_workspace("# {PIN}\n" + EV_OK.replace("{PIN}\n", "")))
    assert rc == 1 and any('does not pin' in c[2] for c in crit), "pin comentado não pode valer"
    ws = _workspace(EV_OK)
    ev = ws / 'docs/ev.md'
    ev.write_text(ev.read_text() + "watched: src/alvo.rs sha256:" + "0" * 64 + "\n")
    rc, crit = _gate(ws)
    assert rc == 1 and any('pinned 2x' in c[2] for c in crit), crit
    for mau in ["/etc/hosts", "../fora.txt"]:
        ws = _workspace(EV_OK)
        ev = ws / 'docs/ev.md'
        ev.write_text(ev.read_text() + f"watched: {mau} sha256:" + "0" * 64 + "\n")
        rc, crit = _gate(ws)
        assert rc == 1 and any('escapes the workspace' in c[2] for c in crit), (mau, crit)
    print("✅ test_r41_pins_nao_confiaveis_sao_critical: PASS")



def test_r41_ronda2_neg_disfarcado_de_log_ci_e_vermelho():
    """Falsificador R2 (F1–F4): o prefixo de CI não pode disfarçar uma linha NEG ou um comentário."""
    real = "{PIN}\ntest t::alvo_passa ... FAILED\ntest result: FAILED. 4 passed; 1 failed\n"
    for neg in ["NEG\tx\t2026-01-01T00:00:00Z test t::alvo_passa ... ok",
                "NEG: miri (old)\tRun tests\t2026-10-05T05:41:30.565Z test t::alvo_passa ... ok",
                "# antigo\tx\t2026-10-05T05:41:30Z test t::alvo_passa ... ok",
                "NEG\tx\t2026-01-01T00:00:00Z alvo_passa: 5 passed; 0 failed",
                "> job\tstep\t2026-10-05T05:41:30Z test t::alvo_passa ... ok"]:
        assert _req(real + neg + "\nNEG: test result: ok. 5 passed; 0 failed\n") == 1, f"aceitou: {neg!r}"
    print("✅ test_r41_ronda2_neg_disfarcado_de_log_ci_e_vermelho: PASS")


def test_r41_ronda2_seccao_de_controlo_negativo_nao_prova():
    ev = ("{PIN}\ntest t::alvo_passa ... FAILED\ntest result: ok. 3 passed; 0 failed\n"
          "## 2. Controlo negativo — contra o código antigo\ntest t::alvo_passa ... ok\n")
    assert _req(ev) == 1, "um ok depois do título de controlo negativo não prova o teste"
    ev = ("{PIN}\ntest t::alvo_passa ... ok\ntest result: ok. 3 passed; 0 failed\n"
          "## 2. Controlo negativo\ntest t::alvo_passa ... FAILED\n")
    assert _req(ev) == 0, "o ok ANTES do título continua a provar"
    print("✅ test_r41_ronda2_seccao_de_controlo_negativo_nao_prova: PASS")


def test_r41_ronda2_comentario_html_nao_fechado_esconde_ate_ao_fim():
    ev = "{PIN}\ntest result: ok. 3 passed; 0 failed\n<!-- rascunho\ntest t::alvo_passa ... ok\n"
    assert _req(ev) == 1, "um <!-- sem fecho esconde o resto (F5)"
    ev = EV_OK.replace("{PIN}", "<!-- {PIN} -->")
    rc, crit = _gate(_workspace(ev))
    assert rc == 1 and any('does not pin' in c[2] for c in crit), "watched: dentro de HTML não fixa (M23)"
    print("✅ test_r41_ronda2_comentario_html_nao_fechado_esconde_ate_ao_fim: PASS")


def test_r41_ronda2_harness_com_panicked_ou_sufixo_colado_e_vermelho():
    resumo = "test result: ok. 9 passed; 0 failed\n"
    assert _req("{PIN}\n" + resumo + "alvo_passa: 4 passed; 0 failed; 2 panicked\n") == 1, "F6"
    assert _req("{PIN}\n" + resumo + "alvo_passa: 4 passed; 0 failedX\n") == 1, "M22: \\b do harness"
    assert _req("{PIN}\n" + resumo + "job\tstep\tsem-timestamp test t::alvo_passa ... ok\n") == 1, \
        "M21: prefixo de CI sem timestamp não é log de CI"
    print("✅ test_r41_ronda2_harness_com_panicked_ou_sufixo_colado_e_vermelho: PASS")


def test_r41_ronda2_symlink_para_fora_do_workspace_e_critical():
    import os
    fora = Path(tempfile.mkdtemp(prefix='gate-r41-fora-'))
    (fora / 'alvo.rs').write_text('fn alvo() {}\n')
    ws = _workspace(EV_OK)
    (ws / 'link').symlink_to(fora, target_is_directory=True)
    ev = ws / 'docs/ev.md'
    ev.write_text(ev.read_text() + f"watched: link/alvo.rs sha256:{_sha(fora / 'alvo.rs')}\n")
    rc, crit = _gate(ws)
    assert rc == 1 and any('symlink' in c[2] for c in crit), crit
    print("✅ test_r41_ronda2_symlink_para_fora_do_workspace_e_critical: PASS")


TESTS = [
    test_extract_passed_count,
    test_evidence_git_hash,
    test_gate_rejects_bad_ledger_a_no_evidence,
    test_gate_rejects_bad_ledger_b_zero_passed,
    test_gate_rejects_bad_ledger_c_empty_negative_control,
    test_gate_rejects_full_bad_ledger_exit_1,
    test_pending_verification_within_deadline_is_ok,
    test_pending_verification_past_deadline_is_critical,
    test_pending_verification_malformed_review_by_is_critical,
    test_open_and_wontfix_are_visible_not_ok,
    test_r41_evidencia_valida_passa,
    test_r41_required_test_so_em_comentario_e_vermelho,
    test_r41_required_test_so_failed_e_vermelho,
    test_r41_n_zero_e_vermelho,
    test_r41_formatos_estruturados_aceites,
    test_r41_vigiado_alterado_sem_evidencia_nova_e_stale,
    test_r41_alterado_com_evidencia_regenerada_no_mesmo_commit_e_verde,
    test_r41_vigiado_inexistente_e_critical_nao_verificavel,
    test_r41_source_file_sem_watched_e_critical,
    test_r41_ci_yml_e_declaravel,
    test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde,
    test_r41_td026_clone_raso_igual_a_completo,
    test_r41_neg_ok_com_real_failed_e_vermelho,
    test_r41_so_prefixo_de_log_ci_e_aceite,
    test_r41_harness_exige_n_maior_que_zero_e_zero_falhas_sem_comentario,
    test_r41_nome_com_metacaracteres_e_literal,
    test_r41_vigiado_ilegivel_ou_diretorio_e_critical,
    test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado,
    test_r41_pins_nao_confiaveis_sao_critical,
    test_r41_ronda2_neg_disfarcado_de_log_ci_e_vermelho,
    test_r41_ronda2_seccao_de_controlo_negativo_nao_prova,
    test_r41_ronda2_comentario_html_nao_fechado_esconde_ate_ao_fim,
    test_r41_ronda2_harness_com_panicked_ou_sufixo_colado_e_vermelho,
    test_r41_ronda2_symlink_para_fora_do_workspace_e_critical,
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
            print(f"test {test.__name__} ... ok")
        except AssertionError as e:
            print(f"❌ {test.__name__}: FAIL\n   {e}")
            print(f"test {test.__name__} ... FAILED")
            failed += 1
        except Exception as e:
            print(f"💥 {test.__name__}: ERROR — {type(e).__name__}: {e}")
            print(f"test {test.__name__} ... FAILED")
            failed += 1
    print(f"\n{'='*60}")
    print(f"{'='*60}")
    print(f"Tests: {passed} passed, {failed} failed")
    # Resumo no formato que o próprio gate aceita como evidência (`N passed; M failed`).
    print(f"test_audit_gate: {passed} passed; {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
