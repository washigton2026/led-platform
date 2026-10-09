#!/usr/bin/env python3
"""
Testes de scripts/pg/pg_check_plan.py e scripts/pg/pg_check_diff.py (Parallel Gauntlet V1, F1b).

Testes de TABELA por regra, com caminhos REAIS do repo escritos à mão (nunca iterando as constantes da
ferramenta — isso seria tautológico: a falsificação do F1 mostrou que tirar `Cargo.lock` da lista ficava
verde). Cada regra tem o seu caso VERMELHO ao lado do verde (KB-012).

Cada teste corre num repo git TEMPORÁRIO com um `origin/main` simulado (`git update-ref`), para que
a R9 (base = origin/main real) seja exercitada e para não depender do checkout da CI ter origin/main.

Run:
  python3 tests/test_pg_check.py
"""
import hashlib
import io
import json
import shutil
import subprocess
import sys
import tempfile
from contextlib import redirect_stdout
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RAIZ / "scripts" / "pg"))
import pg_check_diff  # noqa: E402
import pg_check_plan  # noqa: E402

# Caminhos reais do repo, escritos à mão (um por área protegida mínima e um por ficheiro quente fixo).
PROTEGIDOS_REAIS = [
    "docs/adr/0005-wifi-proibido-producao.md",
    "crates/led-daemon/src/lib.rs",
    "crates/led-core/src/lib.rs",
    "crates/led-daemon-bin/src/proto.rs",
    "crates/led-daemon-bin/src/server.rs",
    "crates/led-protocols/src/lib.rs",
    "crates/led-hal/src/lib.rs",
]
QUENTES_REAIS = [
    "docs/technical-debt-ledger.md",
    ".github/workflows/ci.yml",
    "CLAUDE.md",
    "docs/ROADMAP.md",
    "Cargo.lock",
    "crates/led-daemon-bin/src/run.rs",
]
LIVRES_REAIS = ["crates/led-xlights/src/lib.rs", "crates/led-xlights/src/parse.rs", "scripts/pg/pg_check_plan.py"]


def git(repo: Path, *args) -> str:
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    assert r.returncode == 0, f"git {args}: {r.stderr}"
    return r.stdout.strip()


def repo(ficheiros=(), evidencia=None, com_origin=True) -> tuple[Path, str]:
    """Repo temporário com `ficheiros` (caminhos reais) em main, e origin/main = HEAD."""
    r = Path(tempfile.mkdtemp())
    git(r, "init", "-q", "-b", "main")
    git(r, "config", "user.email", "t@t")
    git(r, "config", "user.name", "t")
    for f in list(ficheiros) or ["README"]:
        p = r / f
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(f"{f}\n")
    for nome, texto in (evidencia or {}).items():
        p = r / "docs" / "evidence" / nome
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(texto.encode())
    git(r, "add", "-A")
    git(r, "commit", "-q", "-m", "base")
    head = git(r, "rev-parse", "HEAD")
    if com_origin:
        git(r, "update-ref", "refs/remotes/origin/main", head)
    return r, head


def tarefa(i, box="READ", lane="parallel", deps=(), owns=(), **extra):
    t = {"id": i, "box": box, "lane": lane, "deps": list(deps), "owns": list(owns), "forbids": []}
    t.update(extra)
    return t


def plano(base, *tarefas, **extra):
    p = {"schema": "pg-plan/1", "plan_id": "teste", "base_sha": base, "tasks": list(tarefas)}
    p.update(extra)
    return p


def regras(viol):
    return {v.split(":")[0].replace("VIOLA ", "") for v in viol}


def accept(d: Path, texto="accept aprovado\n") -> tuple[str, str]:
    f = d / "accept.md"
    f.write_text(texto)
    return "accept.md", hashlib.sha256(texto.encode()).hexdigest()


def verifica(ws, p, **kw):
    return pg_check_plan.verificar(p, ws, plano_dir=kw.pop("plano_dir", ws), **kw)


# ── R1 / R2 / R3 / R7 ─────────────────────────────────────────────────────────────

def test_plano_valido_passa():
    ws, b = repo(LIVRES_REAIS)
    p = plano(b, tarefa("T1"), tarefa("T2"), tarefa("T3"),
              tarefa("T4", box="BUILD", lane="serial", deps=["T1", "T2", "T3"], owns=["crates/led-xlights/**"]))
    assert verifica(ws, p) == [], verifica(ws, p)


def test_ciclo_e_dependencias():
    ws, b = repo()
    v = verifica(ws, plano(b, tarefa("T1", deps=["T3"]), tarefa("T2", deps=["T1"]), tarefa("T3", deps=["T2"])))
    assert any(x.startswith("VIOLA R1: ciclo") for x in v), v
    v = verifica(ws, plano(b, tarefa("T1", deps=["T1"])))
    assert any("ciclo T1 -> T1" in x for x in v), v
    v = verifica(ws, plano(b, tarefa("T1", deps=["T9"]), tarefa("T1")))
    assert any("'T9', que não existe" in x for x in v) and any("id duplicado 'T1'" in x for x in v), v


def test_r2_construcao_paralela_reprova():
    ws, b = repo(LIVRES_REAIS)
    for box in ("BUILD", "DIFF-ONLY"):
        v = verifica(ws, plano(b, tarefa("T1", box=box, owns=["crates/led-xlights/src/lib.rs"])))
        assert "R2" in regras(v), (box, v)


def test_r3_tabela_de_sobreposicao():
    ws, b = repo(LIVRES_REAIS)
    # (padrão A, padrão B, sobrepõem?)
    tabela = [
        ("crates/led-xlights/**", "crates/led-xlights/src/lib.rs", True),
        ("crates/led-xlights/**", "crates/led-xlights/src/*.rs", True),
        ("crates/led-xlights/src/*.rs", "crates/led-xlights/src/parse.rs", True),
        ("crates/led-xlights/src/*.rs", "crates/led-xlights/src/a*.rs", True),   # fecha na dúvida
        ("crates/**", "crates/led-xlights/**", True),
        ("crates/led-xlights/**", "crates/led-xlightsX/**", False),
        ("crates/led-xlights/src/*.rs", "crates/led-xlights/tests/*.rs", False),
        ("crates/led-xlights/src/lib.rs", "crates/led-xlights/src/parse.rs", False),
    ]
    for a, c, esperado in tabela:
        v = verifica(ws, plano(b, tarefa("T1", box="DIFF-ONLY", owns=[a]), tarefa("T2", box="DIFF-ONLY", owns=[c])))
        assert ("R3" in regras(v)) == esperado, (a, c, esperado, v)


def test_r7_read_nao_possui_nada_em_qualquer_lane():
    ws, b = repo(LIVRES_REAIS)
    for lane in ("serial", "parallel"):
        v = verifica(ws, plano(b, tarefa("T1", lane=lane, owns=["crates/led-xlights/src/lib.rs"])))
        assert "R7" in regras(v), (lane, v)


# ── R4: quente fixo, do plano e watched ───────────────────────────────────────────

def test_r4_quentes_fixos_um_a_um():
    ws, b = repo(QUENTES_REAIS)
    for q in QUENTES_REAIS:
        v = verifica(ws, plano(b, tarefa("T1", owns=[q])))
        assert any(x.startswith("VIOLA R4") and f"(fixo: {q})" in x for x in v), f"{q} aceite em paralelo: {v}"


def test_r4_glob_de_ultimo_segmento_apanha_quente():
    ws, b = repo(QUENTES_REAIS)
    v = verifica(ws, plano(b, tarefa("T1", owns=["docs/*.md"])))
    assert any("(fixo: docs/ROADMAP.md)" in x for x in v), v


def test_r4_watched_ao_vivo_md_txt_subpasta_crlf():
    alvo = "crates/led-xlights/src/lib.rs"
    sha = "a" * 64
    for nome, texto in [("td.md", f"watched: {alvo} sha256:{sha}\n"),
                        ("sub/td.txt", f"watched: {alvo} sha256:{sha}\n"),
                        ("td-crlf.md", f"# x\r\nwatched: {alvo} sha256:{sha}\r\n"),
                        ("TD.MD", f"watched: {alvo} sha256:{sha}\n")]:
        ws, b = repo(LIVRES_REAIS)
        p = plano(b, tarefa("T1", owns=[alvo]))
        assert not any("(watched:" in x for x in verifica(ws, p)), "sem evidência não é quente"
        ev = ws / "docs" / "evidence" / nome
        ev.parent.mkdir(parents=True, exist_ok=True)
        ev.write_bytes(texto.encode())
        v = verifica(ws, p)
        assert any(x.startswith("VIOLA R4") and f"(watched: {alvo})" in x for x in v), (nome, v)


def test_r4_watched_comentado_nao_fixa():
    ws, b = repo(LIVRES_REAIS, evidencia={
        "x.md": "<!--\nwatched: crates/led-xlights/src/lib.rs sha256:" + "a" * 64 + "\n-->\n"})
    assert pg_check_plan.watched_vivos(ws) == set()


def test_r4_watched_reais_do_repo():
    vig = pg_check_plan.watched_vivos(RAIZ)
    for f in ["crates/audio-core/src/ring_buffer.rs", ".github/workflows/ci.yml",
              "crates/led-console-bin/tests/ipc_contra_o_daemon.rs", "crates/led-hal/tests/no_alloc.rs"]:
        assert f in vig, f"{f} é vigiado no repo real e não foi lido: {sorted(vig)}"


def test_r4_hot_true_e_hot_do_plano():
    ws, b = repo(LIVRES_REAIS)
    assert any("hot: true" in x for x in verifica(ws, plano(b, tarefa("T1", hot=True))))
    v = verifica(ws, plano(b, tarefa("T1", owns=["crates/led-xlights/src/lib.rs"]), hot=["crates/led-xlights/**"]))
    assert any("(plano: crates/led-xlights/**)" in x for x in v), v


# ── R5: área protegida, com accept verificado ─────────────────────────────────────

def test_r5_protegidos_reais_um_a_um():
    ws, b = repo(PROTEGIDOS_REAIS)
    ref, sha = accept(ws)
    for f in PROTEGIDOS_REAIS:
        sem = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[f])))
        assert any(x.startswith("VIOLA R5") and "sem accept_ref" in x for x in sem), f"{f} sem accept aceite: {sem}"
        com = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[f], accept_ref=ref, accept_sha=sha)))
        assert "R5" not in regras(com), f"{f} com accept verificado recusado: {com}"
        par = verifica(ws, plano(b, tarefa("T1", box="DIFF-ONLY", owns=[f], accept_ref=ref, accept_sha=sha)))
        assert any("não é serial" in x for x in par), f"{f} paralelo aceite: {par}"


def test_r5_globs_que_alcancam_a_area_protegida():
    ws, b = repo(PROTEGIDOS_REAIS + LIVRES_REAIS)
    for o in ["crates/led-core/**", "crates/led-core/src/*.rs", "crates/**", "docs/**", "docs/adr",
              "docs/adr/0099-novo.md", "crates/led-hal/src/novo.rs", "crates/led-daemon-bin/src/*.rs"]:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[o])))
        assert any(x.startswith("VIOLA R5") for x in v), f"{o} alcança área protegida e passou: {v}"
    for o in ["crates/led-xlights/**", "crates/led-daemonX/src/lib.rs", "crates/led-daemon-bin/tests/*.rs"]:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[o])))
        assert "R5" not in regras(v), f"{o} não é protegido e foi recusado: {v}"


def test_r5_accept_e_verificado_de_facto():
    ws, b = repo(PROTEGIDOS_REAIS)
    ref, sha = accept(ws)
    f = "crates/led-core/src/lib.rs"
    casos = [
        (dict(accept_ref=ref, accept_sha="0" * 64), "não é o sha256"),
        (dict(accept_ref=ref, accept_sha=sha[:12]), "não é sha256"),
        (dict(accept_ref="nao-existe.md", accept_sha=sha), "não existe"),
        (dict(accept_sha=sha), "sem accept_ref"),
    ]
    for extra, motivo in casos:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[f], **extra)))
        assert any(motivo in x for x in v), (extra, v)
    (ws / ref).write_text("accept alterado depois da aprovação\n")
    v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[f], accept_ref=ref, accept_sha=sha)))
    assert any("não é o sha256" in x for x in v), f"accept alterado depois de aprovado foi aceite: {v}"


def test_r5_protected_do_plano_literal_e_arvore_somam_ao_minimo():
    ws, b = repo(LIVRES_REAIS)
    for prot, owns in [(["crates/led-xlights/src/lib.rs"], "crates/led-xlights/src/lib.rs"),
                       (["crates/led-xlights/**"], "crates/led-xlights/src/parse.rs")]:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[owns]), protected=prot))
        assert "R5" in regras(v), (prot, v)


# ── R6 / R8 / R9 ──────────────────────────────────────────────────────────────────

def test_r6_fronteira_dos_20_gib():
    ws, b = repo(LIVRES_REAIS)
    p = plano(b, tarefa("T1", box="BUILD", lane="serial", owns=["crates/led-xlights/**"], needs_worktree=True))
    assert any(x.startswith("VIOLA R6") for x in verifica(ws, p, disco_gib=19.99))
    assert "R6" not in regras(verifica(ws, p, disco_gib=20.0)), "exatamente 20 GiB tem de passar"


def test_r8_tabela_de_owns_indecidiveis():
    ws, b = repo(PROTEGIDOS_REAIS + LIVRES_REAIS)
    for o in ["crates/*/build.rs", "docs/ad?/0099-novo.md", "crates/led-[c]ore/src/lib.rs", "**/lib.rs",
              "./CLAUDE.md", "docs/../CLAUDE.md", "/etc/passwd", "crates//x.rs", "{crates,docs}/x.rs",
              "crates/led-hal*/**", "crates\\led-core", "crates/**/lib.rs", ""]:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[o])))
        assert any(x.startswith("VIOLA R8") for x in v), f"{o!r} indecidível aceite: {v}"
    for o in ["crates/led-xlights/**", "crates/led-xlights/src/*.rs", "crates/led-xlights/src/lib.rs",
              "crates/led-xlights/src/[ab]*.rs"]:
        v = verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=[o])))
        assert "R8" not in regras(v), f"{o!r} decidível recusado: {v}"


def test_r9_a_base_e_o_origin_main_lido_pela_ferramenta():
    ws, b = repo(LIVRES_REAIS)
    vazia = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    for base in [None, "", vazia, "0" * 40, "zzzzzzz", b[:6]]:
        v = verifica(ws, plano(base, tarefa("T1")))
        assert "R9" in regras(v), f"base {base!r} aceite: {v}"
    for base in [b, b[:7], b[:12]]:
        assert "R9" not in regras(verifica(ws, plano(base, tarefa("T1")))), base


def test_r9_sem_origin_main_nao_ha_veredito():
    ws, b = repo(LIVRES_REAIS, com_origin=False)
    try:
        verifica(ws, plano(b, tarefa("T1")))
    except pg_check_plan.PlanoIlegivel as e:
        assert "origin/main" in str(e)
        return
    raise AssertionError("sem origin/main a ferramenta decidiu na mesma")


def test_arvore_real_e_defesa_em_profundidade_da_regra_estrutural():
    """Se a regra estrutural falhar (aqui: forçada a dizer «nunca se sobrepõem»), a expansão contra a árvore
    real de origin/main tem de o apanhar e recusar decidir. Sem esta defesa, apagar a árvore real era
    uma mutação equivalente."""
    ws, b = repo(PROTEGIDOS_REAIS)
    original = pg_check_plan.sobrepoem_estrutural
    pg_check_plan.sobrepoem_estrutural = lambda a, c: False
    try:
        verifica(ws, plano(b, tarefa("T1", box="BUILD", lane="serial", owns=["crates/led-core/**"])))
    except pg_check_plan.PlanoIlegivel as e:
        assert "inconsistência interna" in str(e), e
        return
    finally:
        pg_check_plan.sobrepoem_estrutural = original
    raise AssertionError("a árvore real não apanhou um ficheiro protegido que a regra estrutural deixou passar")


# ── Esquema e CLI ─────────────────────────────────────────────────────────────────

def test_plano_ilegivel():
    ws, b = repo()
    for mau in [{}, [], {"schema": "pg-plan/1", "tasks": []}, {"tasks": [tarefa("T1")]},
                {"schema": "lixo/999", "tasks": [tarefa("T1")]},
                plano(b, {"id": "T1"}), plano(b, {**tarefa("T1"), "owns": "a.rs"}), plano(b, tarefa("T1", hot="sim")),
                plano(b, tarefa("T1"), protected=[1]), plano(b, tarefa("T1"), protected=["crates/*.rs"]),
                plano(b, tarefa("T1"), hot=["crates/*/x"]), plano(b, tarefa("T1", accept_ref=5))]:
        try:
            verifica(ws, mau)
        except pg_check_plan.PlanoIlegivel:
            continue
        raise AssertionError(f"plano ilegível aceite: {mau}")


def test_cli_exit_codes():
    ws, b = repo(LIVRES_REAIS)
    casos = [(plano(b, tarefa("T1")), 0), (plano(b, tarefa("T1", deps=["T1"])), 1),
             (plano("0" * 40, tarefa("T1")), 1)]
    for p, esperado in casos:
        f = ws / "plan.json"
        f.write_text(json.dumps(p))
        with redirect_stdout(io.StringIO()):
            rc = pg_check_plan.main([str(f), "--workspace", str(ws)])
        assert rc == esperado, f"exit {rc}, esperado {esperado}: {p}"
    f = ws / "mau.json"
    f.write_text("{ não é json")
    with redirect_stdout(io.StringIO()):
        assert pg_check_plan.main([str(f), "--workspace", str(ws)]) == 2


# ── pg_check_diff ─────────────────────────────────────────────────────────────────

def ramo(base_ficheiros, mudar: dict, apagar=(), mover=(), renames=None) -> Path:
    """Repo com `base_ficheiros` em main; ramo `t` com escritas, remoções e `git mv`."""
    r, _ = repo(base_ficheiros)
    if renames is not None:
        git(r, "config", "diff.renames", renames)
    git(r, "checkout", "-q", "-b", "t")
    for caminho, conteudo in mudar.items():
        p = r / caminho
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(conteudo)
        git(r, "add", caminho)
    for caminho in apagar:
        git(r, "rm", "-q", caminho)
    for de, para in mover:
        (r / para).parent.mkdir(parents=True, exist_ok=True)
        git(r, "mv", de, para)
    git(r, "commit", "-q", "-m", "ramo")
    return r


def card(owns, forbids=()) -> str:
    return f"---\nplan: x\ntask: T1\nowns: [{', '.join(owns)}]\nforbids: [{', '.join(forbids)}]\n---\n# T1\n"


def corre_diff(r: Path, texto_card: str) -> tuple[int, str]:
    c = Path(tempfile.mkdtemp()) / "card.md"
    c.write_text(texto_card)
    out = io.StringIO()
    with redirect_stdout(out):
        rc = pg_check_diff.main(["main", "t", str(c), "--repo", str(r)])
    return rc, out.getvalue()


def test_diff_dentro_da_area_passa():
    r = ramo(LIVRES_REAIS, {"crates/led-xlights/src/lib.rs": "x", "crates/led-xlights/src/novo.rs": "n"})
    rc, out = corre_diff(r, card(["crates/led-xlights/**"]))
    assert rc == 0, out


def test_diff_com_um_ficheiro_fora_reprova():
    r = ramo(LIVRES_REAIS + QUENTES_REAIS, {"crates/led-xlights/src/lib.rs": "x", "CLAUDE.md": "intruso"})
    rc, out = corre_diff(r, card(["crates/led-xlights/**"]))
    assert rc == 1 and "FORA DE owns: CLAUDE.md" in out and "crates/led-xlights/src/lib.rs" not in out, out


def test_diff_remocao_conta():
    r = ramo(LIVRES_REAIS + PROTEGIDOS_REAIS, {}, apagar=["crates/led-core/src/lib.rs"])
    rc, out = corre_diff(r, card(["crates/led-xlights/**"]))
    assert rc == 1 and "FORA DE owns: crates/led-core/src/lib.rs" in out, out


def test_diff_rename_ve_os_dois_lados_qualquer_que_seja_a_config():
    for cfg in (None, "true", "copies", "false"):
        r = ramo(LIVRES_REAIS + PROTEGIDOS_REAIS, {}, mover=[("crates/led-core/src/lib.rs", "src_livre/lib.rs")],
                 renames=cfg)
        rc, out = corre_diff(r, card(["src_livre/**"], ["crates/led-core/**"]))
        assert rc == 1, (cfg, out)
        assert "PROIBIDO: crates/led-core/src/lib.rs" in out and "FORA DE owns: crates/led-core/src/lib.rs" in out, (cfg, out)


def test_diff_forbids_reprova_mesmo_dentro_de_owns():
    r = ramo(LIVRES_REAIS, {"crates/led-xlights/Cargo.lock": "l"})
    rc, out = corre_diff(r, card(["crates/led-xlights/**"], ["crates/led-xlights/Cargo.lock"]))
    assert rc == 1 and "PROIBIDO: crates/led-xlights/Cargo.lock" in out, out


def test_diff_read_qualquer_alteracao_reprova():
    r = ramo(LIVRES_REAIS, {"notas.md": "x"})
    rc, out = corre_diff(r, card([]))
    assert rc == 1 and "FORA DE owns: notas.md" in out, out


def test_diff_card_com_owns_indecidivel_reprova():
    r = ramo(LIVRES_REAIS + PROTEGIDOS_REAIS, {"crates/led-core/build.rs": "fn main(){}"})
    for o in ["crates/*/build.rs", "crates/led-[c]ore/build.rs", "**/build.rs", "./crates/led-core/build.rs"]:
        rc, out = corre_diff(r, card([o]))
        assert rc == 1 and "INDECIDÍVEL: owns" in out, (o, out)


def test_diff_so_conta_o_que_o_ramo_trouxe():
    r = ramo(LIVRES_REAIS, {"crates/led-xlights/src/lib.rs": "x"})
    git(r, "checkout", "-q", "main")
    (r / "outro.txt").write_text("a main andou\n")
    git(r, "add", "outro.txt")
    git(r, "commit", "-q", "-m", "main avança")
    rc, out = corre_diff(r, card(["crates/led-xlights/**"]))
    assert rc == 0, out


def test_card_ilegivel_da_exit_2():
    r = ramo(LIVRES_REAIS, {"crates/led-xlights/src/lib.rs": "x"})
    for mau in ["# sem front-matter\n", "---\nowns: [a.rs]\n---\n", "---\nowns: a.rs\nforbids: []\n---\n",
                "---\nowns: [a.rs]\nforbids: []\n",
                "---\nowns: [crates/led-xlights/**]\nowns: [x]\nforbids: []\n---\n"]:
        rc, out = corre_diff(r, mau)
        assert rc == 2, f"card ilegível aceite ({rc}): {mau!r}\n{out}"


TESTS = [
    test_plano_valido_passa,
    test_ciclo_e_dependencias,
    test_r2_construcao_paralela_reprova,
    test_r3_tabela_de_sobreposicao,
    test_r7_read_nao_possui_nada_em_qualquer_lane,
    test_r4_quentes_fixos_um_a_um,
    test_r4_glob_de_ultimo_segmento_apanha_quente,
    test_r4_watched_ao_vivo_md_txt_subpasta_crlf,
    test_r4_watched_comentado_nao_fixa,
    test_r4_watched_reais_do_repo,
    test_r4_hot_true_e_hot_do_plano,
    test_r5_protegidos_reais_um_a_um,
    test_r5_globs_que_alcancam_a_area_protegida,
    test_r5_accept_e_verificado_de_facto,
    test_r5_protected_do_plano_literal_e_arvore_somam_ao_minimo,
    test_r6_fronteira_dos_20_gib,
    test_r8_tabela_de_owns_indecidiveis,
    test_r9_a_base_e_o_origin_main_lido_pela_ferramenta,
    test_r9_sem_origin_main_nao_ha_veredito,
    test_arvore_real_e_defesa_em_profundidade_da_regra_estrutural,
    test_plano_ilegivel,
    test_cli_exit_codes,
    test_diff_dentro_da_area_passa,
    test_diff_com_um_ficheiro_fora_reprova,
    test_diff_remocao_conta,
    test_diff_rename_ve_os_dois_lados_qualquer_que_seja_a_config,
    test_diff_forbids_reprova_mesmo_dentro_de_owns,
    test_diff_read_qualquer_alteracao_reprova,
    test_diff_card_com_owns_indecidivel_reprova,
    test_diff_so_conta_o_que_o_ramo_trouxe,
    test_card_ilegivel_da_exit_2,
]


def main() -> int:
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
    print(f"test_pg_check: {passed} passed; {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
