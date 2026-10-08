#!/usr/bin/env python3
"""
Testes de scripts/pg/pg_check_plan.py e scripts/pg/pg_check_diff.py (Parallel Gauntlet V1).

Cada regra tem o seu caso VERMELHO (controlo negativo) ao lado do verde: um gate que nunca reprova
não prova nada (KB-012). Os do operador (F1, 2026-10-08): plano com ciclo; owns sobrepostos;
ficheiro quente — fixo OU watched — num lane paralelo; diff com 1 ficheiro fora da área.

Run:
  python3 tests/test_pg_check.py
"""
import io
import json
import subprocess
import sys
import tempfile
from contextlib import redirect_stdout
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RAIZ / "scripts" / "pg"))
import pg_check_diff  # noqa: E402
import pg_check_plan  # noqa: E402

SHA = "a" * 64


def tarefa(i, box="READ", lane="parallel", deps=(), owns=(), **extra):
    t = {"id": i, "box": box, "lane": lane, "deps": list(deps), "owns": list(owns), "forbids": []}
    t.update(extra)
    return t


def plano(*tarefas, **extra):
    p = {"schema": "pg-plan/1", "plan_id": "teste", "tasks": list(tarefas)}
    p.update(extra)
    return p


def ws_vazio() -> Path:
    """Um workspace sem git e sem evidências: só as regras fixas se aplicam."""
    return Path(tempfile.mkdtemp())


def regras(viol):
    return {v.split(":")[0].replace("VIOLA ", "") for v in viol}


# ── pg_check_plan ──────────────────────────────────────────────────────────────

def test_plano_valido_passa():
    p = plano(
        tarefa("T1"), tarefa("T2"), tarefa("T3"),
        tarefa("T4", box="BUILD", lane="serial", deps=["T1", "T2", "T3"], owns=["crates/led-daemon-bin/src/main.rs"]),
    )
    v = pg_check_plan.verificar(p, ws_vazio())
    assert v == [], f"plano válido recusado: {v}"


def test_ciclo_reprova():
    p = plano(tarefa("T1", deps=["T3"]), tarefa("T2", deps=["T1"]), tarefa("T3", deps=["T2"]))
    v = pg_check_plan.verificar(p, ws_vazio())
    assert any(x.startswith("VIOLA R1: ciclo") for x in v), f"ciclo não apanhado: {v}"


def test_auto_dependencia_e_ciclo():
    v = pg_check_plan.verificar(plano(tarefa("T1", deps=["T1"])), ws_vazio())
    assert any("ciclo T1 -> T1" in x for x in v), v


def test_dep_inexistente_e_id_duplicado_reprovam():
    v = pg_check_plan.verificar(plano(tarefa("T1", deps=["T9"]), tarefa("T1")), ws_vazio())
    assert any("'T9', que não existe" in x for x in v), v
    assert any("id duplicado 'T1'" in x for x in v), v


def test_build_paralelo_reprova_no_v1():
    v = pg_check_plan.verificar(plano(tarefa("T1", box="BUILD", owns=["a.rs"])), ws_vazio())
    assert "R2" in regras(v), f"construção paralela aceite no V1: {v}"


def test_owns_sobrepostos_reprova():
    p = plano(tarefa("T1", box="DIFF-ONLY", owns=["crates/x/**"]),
              tarefa("T2", box="DIFF-ONLY", owns=["crates/x/src/lib.rs"]))
    v = pg_check_plan.verificar(p, ws_vazio())
    assert any(x.startswith("VIOLA R3") for x in v), f"owns sobrepostos não apanhados: {v}"


def test_owns_disjuntos_entre_paralelas_nao_dao_r3():
    p = plano(tarefa("T1", box="DIFF-ONLY", owns=["crates/x/**"]),
              tarefa("T2", box="DIFF-ONLY", owns=["crates/y/**"]))
    v = pg_check_plan.verificar(p, ws_vazio())
    assert "R3" not in regras(v), v


def test_owns_em_serie_podem_sobrepor():
    p = plano(tarefa("T1", box="BUILD", lane="serial", owns=["a.rs"]),
              tarefa("T2", box="BUILD", lane="serial", deps=["T1"], owns=["a.rs"]))
    assert pg_check_plan.verificar(p, ws_vazio()) == []


def test_quente_fixo_em_lane_paralelo_reprova():
    for q in pg_check_plan.QUENTES_FIXOS:
        v = pg_check_plan.verificar(plano(tarefa("T1", box="DIFF-ONLY", owns=[q])), ws_vazio())
        assert any(x.startswith("VIOLA R4") and "(fixo:" in x for x in v), f"{q} quente aceite em paralelo: {v}"


def test_quente_fixo_por_glob_reprova():
    v = pg_check_plan.verificar(plano(tarefa("T1", box="DIFF-ONLY", owns=["docs/*.md"])), ws_vazio())
    assert any(x.startswith("VIOLA R4") for x in v), f"glob que apanha o ROADMAP aceite: {v}"


def test_hot_true_em_paralelo_reprova():
    v = pg_check_plan.verificar(plano(tarefa("T1", hot=True)), ws_vazio())
    assert any("hot: true" in x for x in v), v


def test_quente_watched_e_lido_ao_vivo():
    ws = ws_vazio()
    p = plano(tarefa("T1", box="DIFF-ONLY", owns=["src/vigiado.rs"]))
    assert "R4" not in regras(pg_check_plan.verificar(p, ws)), "sem evidência, o ficheiro não é quente"
    ev = ws / "docs" / "evidence"
    ev.mkdir(parents=True)
    (ev / "td-999.md").write_text(f"# TD-999\nwatched: src/vigiado.rs sha256:{SHA}\n")
    v = pg_check_plan.verificar(p, ws)
    assert any(x.startswith("VIOLA R4") and "(watched:" in x for x in v), \
        f"um `watched:` acabado de escrever não tornou o ficheiro quente: {v}"


def test_watched_comentado_nao_fixa():
    ws = ws_vazio()
    ev = ws / "docs" / "evidence"
    ev.mkdir(parents=True)
    (ev / "x.md").write_text(f"<!--\nwatched: src/a.rs sha256:{SHA}\n-->\n  watched: src/b.rs sha256:{SHA}\n")
    assert pg_check_plan.watched_vivos(ws) == set(), "só linhas que COMEÇAM por watched: contam (regra do audit_gate)"


def test_watched_real_do_repo_e_quente():
    vig = pg_check_plan.watched_vivos(RAIZ)
    assert "crates/audio-core/src/ring_buffer.rs" in vig, f"premissa: o TD-027 vigia o ring_buffer: {sorted(vig)}"
    v = pg_check_plan.verificar(plano(tarefa("T1", box="DIFF-ONLY", owns=["crates/audio-core/src/ring_buffer.rs"])), RAIZ)
    assert any(x.startswith("VIOLA R4") and "(watched:" in x for x in v), v


def test_protegido_exige_serie_e_accept():
    for alvo in ["docs/adr/0031-x.md", "crates/led-core/src/lib.rs", "crates/led-daemon-bin/src/proto.rs"]:
        v = pg_check_plan.verificar(plano(tarefa("T1", box="BUILD", lane="serial", owns=[alvo])), ws_vazio())
        assert any(x.startswith("VIOLA R5") and "accept_sha" in x for x in v), f"{alvo} sem accept aceite: {v}"
        ok = pg_check_plan.verificar(plano(tarefa("T1", box="BUILD", lane="serial", owns=[alvo], accept_sha=SHA)), ws_vazio())
        assert "R5" not in regras(ok), f"{alvo} com accept e em série recusado: {ok}"


def test_protegido_em_paralelo_reprova_mesmo_com_accept():
    v = pg_check_plan.verificar(plano(tarefa("T1", box="DIFF-ONLY", owns=["docs/adr/**"], accept_sha=SHA)), ws_vazio())
    assert any(x.startswith("VIOLA R5") and "não é serial" in x for x in v), v


def test_protegido_do_plano_soma_ao_minimo():
    p = plano(tarefa("T1", box="BUILD", lane="serial", owns=["crates/led-xlights/src/lib.rs"]),
              protected=["crates/led-xlights/**"])
    assert "R5" in regras(pg_check_plan.verificar(p, ws_vazio()))


def test_accept_sha_tem_de_ser_sha256():
    v = pg_check_plan.verificar(plano(tarefa("T1", box="BUILD", lane="serial", protected=True, accept_sha="ok")), ws_vazio())
    assert "R5" in regras(v), v


def test_worktree_exige_20_gib():
    p = plano(tarefa("T1", box="BUILD", lane="serial", owns=["a.rs"], needs_worktree=True))
    assert any(x.startswith("VIOLA R6") for x in pg_check_plan.verificar(p, ws_vazio(), disco_gib=14.0))
    assert "R6" not in regras(pg_check_plan.verificar(p, ws_vazio(), disco_gib=25.0))


def test_read_nao_e_dono_de_nada():
    v = pg_check_plan.verificar(plano(tarefa("T1", lane="serial", owns=["a.rs"])), ws_vazio())
    assert "R7" in regras(v), v


def test_plano_ilegivel():
    for mau in [{}, {"tasks": []}, {"tasks": [{"id": "T1"}]},
                {"tasks": [{**tarefa("T1"), "owns": "a.rs"}]}, {"tasks": [tarefa("T1", hot="sim")]}]:
        try:
            pg_check_plan.verificar(mau, ws_vazio())
        except pg_check_plan.PlanoIlegivel:
            continue
        raise AssertionError(f"plano ilegível aceite: {mau}")


def test_cli_exit_codes():
    d = ws_vazio()
    casos = [(plano(tarefa("T1")), 0), (plano(tarefa("T1", deps=["T1"])), 1)]
    for p, esperado in casos:
        f = d / "plan.json"
        f.write_text(json.dumps(p))
        with redirect_stdout(io.StringIO()):
            rc = pg_check_plan.main([str(f), "--workspace", str(d)])
        assert rc == esperado, f"exit {rc}, esperado {esperado}"
    f = d / "mau.json"
    f.write_text("{ não é json")
    with redirect_stdout(io.StringIO()):
        assert pg_check_plan.main([str(f), "--workspace", str(d)]) == 2


# ── pg_check_diff ──────────────────────────────────────────────────────────────

def git(repo: Path, *args):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    assert r.returncode == 0, f"git {args}: {r.stderr}"


def repo_com_ramo(alteracoes: dict[str, str]) -> Path:
    """Repo temporário: `main` com um ficheiro base; ramo `t` com `alteracoes` por cima."""
    repo = Path(tempfile.mkdtemp())
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.email", "t@t")
    git(repo, "config", "user.name", "t")
    (repo / "base.txt").write_text("base\n")
    git(repo, "add", "base.txt")
    git(repo, "commit", "-q", "-m", "base")
    git(repo, "checkout", "-q", "-b", "t")
    for caminho, conteudo in alteracoes.items():
        p = repo / caminho
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(conteudo)
        git(repo, "add", caminho)
    git(repo, "commit", "-q", "-m", "ramo")
    return repo


def card(owns, forbids=()) -> str:
    return f"---\nplan: x\ntask: T1\nowns: [{', '.join(owns)}]\nforbids: [{', '.join(forbids)}]\n---\n# T1\n"


def corre_diff(repo: Path, texto_card: str) -> tuple[int, str]:
    c = repo / "card.md"
    c.write_text(texto_card)
    out = io.StringIO()
    with redirect_stdout(out):
        rc = pg_check_diff.main(["main", "t", str(c), "--repo", str(repo)])
    return rc, out.getvalue()


def test_diff_dentro_da_area_passa():
    repo = repo_com_ramo({"crates/x/src/a.rs": "a", "crates/x/src/b.rs": "b"})
    rc, out = corre_diff(repo, card(["crates/x/**"]))
    assert rc == 0, out


def test_diff_com_um_ficheiro_fora_reprova():
    repo = repo_com_ramo({"crates/x/src/a.rs": "a", "CLAUDE.md": "intruso"})
    rc, out = corre_diff(repo, card(["crates/x/**"]))
    assert rc == 1 and "FORA DE owns: CLAUDE.md" in out, f"rc={rc}\n{out}"
    assert "crates/x/src/a.rs" not in out, "só o ficheiro fora é nomeado"


def test_diff_em_forbids_reprova_mesmo_dentro_de_owns():
    repo = repo_com_ramo({"crates/x/Cargo.lock": "l"})
    rc, out = corre_diff(repo, card(["crates/x/**"], ["**/Cargo.lock", "crates/x/Cargo.lock"]))
    assert rc == 1 and "PROIBIDO: crates/x/Cargo.lock" in out, out


def test_diff_de_tarefa_read_reprova_qualquer_alteracao():
    repo = repo_com_ramo({"notas.md": "x"})
    rc, out = corre_diff(repo, card([]))
    assert rc == 1 and "FORA DE owns: notas.md" in out, out


def test_diff_so_conta_o_que_o_ramo_trouxe():
    repo = repo_com_ramo({"crates/x/a.rs": "a"})
    git(repo, "checkout", "-q", "main")
    (repo / "outro.txt").write_text("a main andou\n")
    git(repo, "add", "outro.txt")
    git(repo, "commit", "-q", "-m", "main avança")
    rc, out = corre_diff(repo, card(["crates/x/**"]))
    assert rc == 0, f"o que a main fez depois do ramo entrou na conta: {out}"


def test_card_ilegivel_da_exit_2():
    repo = repo_com_ramo({"a.rs": "a"})
    for mau in ["# sem front-matter\n", "---\nowns: [a.rs]\n---\n", "---\nowns: a.rs\nforbids: []\n---\n",
                "---\nowns: [a.rs]\nforbids: []\n"]:
        rc, out = corre_diff(repo, mau)
        assert rc == 2, f"card ilegível aceite ({rc}): {mau!r}\n{out}"


TESTS = [
    test_plano_valido_passa,
    test_ciclo_reprova,
    test_auto_dependencia_e_ciclo,
    test_dep_inexistente_e_id_duplicado_reprovam,
    test_build_paralelo_reprova_no_v1,
    test_owns_sobrepostos_reprova,
    test_owns_disjuntos_entre_paralelas_nao_dao_r3,
    test_owns_em_serie_podem_sobrepor,
    test_quente_fixo_em_lane_paralelo_reprova,
    test_quente_fixo_por_glob_reprova,
    test_hot_true_em_paralelo_reprova,
    test_quente_watched_e_lido_ao_vivo,
    test_watched_comentado_nao_fixa,
    test_watched_real_do_repo_e_quente,
    test_protegido_exige_serie_e_accept,
    test_protegido_em_paralelo_reprova_mesmo_com_accept,
    test_protegido_do_plano_soma_ao_minimo,
    test_accept_sha_tem_de_ser_sha256,
    test_worktree_exige_20_gib,
    test_read_nao_e_dono_de_nada,
    test_plano_ilegivel,
    test_cli_exit_codes,
    test_diff_dentro_da_area_passa,
    test_diff_com_um_ficheiro_fora_reprova,
    test_diff_em_forbids_reprova_mesmo_dentro_de_owns,
    test_diff_de_tarefa_read_reprova_qualquer_alteracao,
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
