#!/usr/bin/env python3
"""
pg_check_plan.py — valida um plan.json do Parallel Gauntlet V1 ANTES de qualquer fan-out.

Uso:
  python3 scripts/pg/pg_check_plan.py <plan.json> [--workspace <repo>]

Exit 0 = plano aceite. Exit 1 = viola uma ou mais regras (cada uma numa linha `VIOLA Rn: …`).
Exit 2 = plano ilegível (JSON inválido, campos em falta, tipos errados).

As 7 regras (operador, D2 do F0, 2026-10-08):
  R1  DAG acíclico; ids únicos; toda a `deps` existe.
  R2  V1: `lane: "parallel"` só com `box: "READ"` — a construção é SEMPRE sequencial.
  R3  `owns[]` disjuntos entre tarefas paralelas.
  R4  Nenhuma tarefa paralela toca em ficheiro QUENTE: `hot: true` é recusado, e nenhum `owns` pode
      casar com a lista quente = QUENTES_FIXOS ∪ plan.hot ∪ {todo o `watched:` de docs/evidence/,
      lido AO VIVO — a mesma leitura do audit_gate}.
  R5  Área protegida (PROTEGIDO_MINIMO ∪ plan.protected, ou `protected: true`) → só `lane: "serial"`
      e com `accept_sha` (sha256 do accept.md aprovado pelo operador).
  R6  `needs_worktree: true` → disco livre ≥ 20 GiB, medido agora.
  R7  `box: "READ"` → `owns: []` (leitura não é dona de nada).

Nada aqui escreve no repo; só lê o plano, o `git ls-files` e as evidências.
"""
from __future__ import annotations

import argparse
import fnmatch
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

# A leitura de `watched:` é a do próprio audit_gate (uma regra, uma implementação).
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import audit_gate  # noqa: E402

# D1 do F0 (operador, 2026-10-08): a lista fixa. A parte viva vem de `watched:`.
QUENTES_FIXOS = [
    "docs/technical-debt-ledger.md",
    ".github/workflows/ci.yml",
    "CLAUDE.md",
    "docs/ROADMAP.md",
    "Cargo.lock",
    "crates/led-daemon-bin/src/run.rs",
]

# Áreas protegidas do Gauntlet («só com ordem»). Um plano pode ACRESCENTAR, nunca retirar.
PROTEGIDO_MINIMO = [
    "docs/adr/**",
    "crates/led-daemon/**",
    "crates/led-core/**",
    "crates/led-daemon-bin/src/proto.rs",   # IPC / PROTOCOL_V
    "crates/led-daemon-bin/src/server.rs",  # IPC
    "crates/led-protocols/**",              # protocolos no fio
    "crates/led-hal/**",                    # comportamento de hardware / safety
]

MIN_DISCO_GIB = 20
BOXES = {"READ", "BUILD", "DIFF-ONLY"}
LANES = {"parallel", "serial"}
_SHA256 = re.compile(r"^[0-9a-f]{64}$")


class PlanoIlegivel(Exception):
    pass


def casa(caminho: str, padrao: str) -> bool:
    """`dir/**` casa com o diretório e tudo o que está debaixo dele; o resto é fnmatch."""
    if padrao.endswith("/**"):
        raiz = padrao[:-3]
        return caminho == raiz or caminho.startswith(raiz + "/")
    return fnmatch.fnmatchcase(caminho, padrao)


def sobrepoem(a: str, b: str, universo: list[str]) -> bool:
    """Dois padrões sobrepõem-se se algum ficheiro do repo casa com os dois, ou se um, lido como
    caminho, casa com o outro (apanha ficheiros que ainda não existem)."""
    if a == b or casa(a, b) or casa(b, a):
        return True
    return any(casa(f, a) and casa(f, b) for f in universo)


def watched_vivos(workspace: Path) -> set[str]:
    """Todos os caminhos em `watched:` de todas as evidências em docs/evidence/, AGORA."""
    vistos: set[str] = set()
    raiz = workspace / "docs" / "evidence"
    if not raiz.is_dir():
        return vistos
    for f in sorted(raiz.rglob("*")):
        if f.is_file() and f.suffix in {".md", ".txt"}:
            vistos.update(audit_gate.evidence_watched(f.read_text(errors="replace")))
    return vistos


def ficheiros_do_repo(workspace: Path, base_sha: str | None) -> list[str]:
    cmd = ["git", "-C", str(workspace), "ls-tree", "-r", "--name-only", base_sha] if base_sha else \
          ["git", "-C", str(workspace), "ls-files"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0 and base_sha:
        r = subprocess.run(["git", "-C", str(workspace), "ls-files"], capture_output=True, text=True)
    return r.stdout.splitlines() if r.returncode == 0 else []


def disco_livre_gib(workspace: Path) -> float:
    return shutil.disk_usage(workspace).free / 2**30


def _lista(t: dict, campo: str) -> list[str]:
    v = t.get(campo, [])
    if not isinstance(v, list) or not all(isinstance(x, str) for x in v):
        raise PlanoIlegivel(f"tarefa {t.get('id')!r}: `{campo}` tem de ser lista de strings")
    return v


def validar_esquema(plano: dict) -> list[dict]:
    if not isinstance(plano, dict) or not isinstance(plano.get("tasks"), list) or not plano["tasks"]:
        raise PlanoIlegivel("o plano tem de ter `tasks`: lista não vazia")
    for campo in ("hot", "protected"):
        if not isinstance(plano.get(campo, []), list):
            raise PlanoIlegivel(f"`{campo}` do plano tem de ser lista")
    for t in plano["tasks"]:
        if not isinstance(t, dict) or not isinstance(t.get("id"), str) or not t["id"]:
            raise PlanoIlegivel(f"tarefa sem `id`: {t!r}")
        if t.get("box") not in BOXES:
            raise PlanoIlegivel(f"tarefa {t['id']!r}: `box` tem de ser um de {sorted(BOXES)}")
        if t.get("lane") not in LANES:
            raise PlanoIlegivel(f"tarefa {t['id']!r}: `lane` tem de ser um de {sorted(LANES)}")
        for campo in ("deps", "owns", "forbids"):
            _lista(t, campo)
        for campo in ("hot", "protected", "needs_worktree"):
            if not isinstance(t.get(campo, False), bool):
                raise PlanoIlegivel(f"tarefa {t['id']!r}: `{campo}` tem de ser booleano")
    return plano["tasks"]


def verificar(plano: dict, workspace: Path, disco_gib: float | None = None) -> list[str]:
    """Devolve a lista de violações (vazia = plano aceite). Lança PlanoIlegivel se não há plano."""
    tarefas = validar_esquema(plano)
    viol: list[str] = []
    ids = [t["id"] for t in tarefas]
    por_id = {t["id"]: t for t in tarefas}

    # R1 — ids únicos, deps existem, DAG acíclico (DFS com cores; o ciclo é nomeado).
    for i in sorted({i for i in ids if ids.count(i) > 1}):
        viol.append(f"VIOLA R1: id duplicado {i!r}")
    for t in tarefas:
        for d in t.get("deps", []):
            if d not in por_id:
                viol.append(f"VIOLA R1: {t['id']!r} depende de {d!r}, que não existe")
    cor: dict[str, int] = {}
    caminho: list[str] = []

    def dfs(n: str) -> list[str] | None:
        cor[n] = 1
        caminho.append(n)
        for d in por_id[n].get("deps", []):
            if d not in por_id:
                continue
            if cor.get(d) == 1:
                return caminho[caminho.index(d):] + [d]
            if cor.get(d) is None:
                c = dfs(d)
                if c:
                    return c
        cor[n] = 2
        caminho.pop()
        return None

    for n in por_id:
        if cor.get(n) is None:
            ciclo = dfs(n)
            if ciclo:
                viol.append(f"VIOLA R1: ciclo {' -> '.join(ciclo)}")
                break

    paralelas = [t for t in tarefas if t["lane"] == "parallel"]
    universo = ficheiros_do_repo(workspace, plano.get("base_sha"))

    # R2 — V1: paralelo só de leitura.
    for t in paralelas:
        if t["box"] != "READ":
            viol.append(f"VIOLA R2: {t['id']!r} é paralela com box {t['box']} — no V1 só READ corre em paralelo")

    # R3 — owns disjuntos entre paralelas.
    for i, a in enumerate(paralelas):
        for b in paralelas[i + 1:]:
            for pa in a.get("owns", []):
                for pb in b.get("owns", []):
                    if sobrepoem(pa, pb, universo):
                        viol.append(f"VIOLA R3: owns sobrepostos {a['id']!r}:{pa!r} × {b['id']!r}:{pb!r}")

    # R4 — nada quente num lane paralelo (fixo, do plano, ou `watched:` lido agora).
    vigiados = watched_vivos(workspace)
    quentes = [(q, "fixo") for q in QUENTES_FIXOS] + [(q, "plano") for q in plano.get("hot", [])] + \
              [(q, "watched") for q in sorted(vigiados)]
    for t in paralelas:
        if t.get("hot", False):
            viol.append(f"VIOLA R4: {t['id']!r} é paralela e está marcada hot: true")
        for p in t.get("owns", []):
            for q, origem in quentes:
                if sobrepoem(p, q, universo):
                    viol.append(f"VIOLA R4: {t['id']!r} é paralela e possui {p!r}, quente ({origem}: {q})")

    # R5 — área protegida: só em série e com accept aprovado.
    protegidos = PROTEGIDO_MINIMO + list(plano.get("protected", []))
    for t in tarefas:
        toca = [p for p in t.get("owns", []) if any(sobrepoem(p, q, universo) for q in protegidos)]
        if t.get("protected", False) or toca:
            motivo = f"possui {toca}" if toca else "protected: true"
            if t["lane"] != "serial":
                viol.append(f"VIOLA R5: {t['id']!r} é área protegida ({motivo}) e não é serial")
            acc = t.get("accept_sha")
            if not (isinstance(acc, str) and _SHA256.match(acc)):
                viol.append(f"VIOLA R5: {t['id']!r} é área protegida ({motivo}) sem accept_sha (sha256 do accept.md)")

    # R6 — worktree só com ≥ 20 GiB livres, medido agora.
    if any(t.get("needs_worktree", False) for t in tarefas):
        livre = disco_livre_gib(workspace) if disco_gib is None else disco_gib
        for t in tarefas:
            if t.get("needs_worktree", False) and livre < MIN_DISCO_GIB:
                viol.append(f"VIOLA R6: {t['id']!r} pede worktree com {livre:.1f} GiB livres (< {MIN_DISCO_GIB})")

    # R7 — leitura não é dona de nada.
    for t in tarefas:
        if t["box"] == "READ" and t.get("owns"):
            viol.append(f"VIOLA R7: {t['id']!r} é READ e tem owns {t['owns']}")

    return viol


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    ap.add_argument("plano")
    ap.add_argument("--workspace", default=str(Path(__file__).resolve().parents[2]))
    a = ap.parse_args(argv)
    try:
        plano = json.loads(Path(a.plano).read_text())
        viol = verificar(plano, Path(a.workspace))
    except (OSError, json.JSONDecodeError, PlanoIlegivel) as e:
        print(f"pg_check_plan: PLANO ILEGÍVEL — {e}")
        return 2
    if viol:
        for v in viol:
            print(v)
        print(f"pg_check_plan: RECUSADO — {len(viol)} violação(ões)")
        return 1
    n = len(plano["tasks"])
    p = sum(t["lane"] == "parallel" for t in plano["tasks"])
    print(f"pg_check_plan: OK — {n} tarefa(s), {p} paralela(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
