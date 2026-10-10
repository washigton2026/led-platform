#!/usr/bin/env python3
"""
pg_check_plan.py — valida um plan.json do Parallel Gauntlet V1 ANTES de qualquer fan-out.

Uso:
  python3 scripts/pg/pg_check_plan.py <plan.json> [--workspace <repo>]

Exit 0 = plano aceite. Exit 1 = viola uma ou mais regras (uma linha `VIOLA Rn: …` cada).
Exit 2 = plano ilegível ou indecidível (JSON inválido, schema desconhecido, campos com tipo errado,
origin/main ilegível no workspace).

Regras (operador, F0 D2 + F1b, 2026-10-08/09):
  R1  DAG acíclico; ids únicos; toda a `deps` existe.
  R2  V1: `lane: "parallel"` só com `box: "READ"` — a construção é SEMPRE sequencial.
  R3  `owns[]` disjuntos entre tarefas paralelas.
  R4  Nenhuma tarefa paralela possui um ficheiro QUENTE: `hot: true` é recusado, e nenhum `owns` pode
      sobrepor-se à lista quente = QUENTES_FIXOS ∪ plan.hot ∪ {todo o `watched:` de docs/evidence/**,
      ficheiros .md/.txt, lido AGORA com o mesmo parser do audit_gate}.
  R5  Área protegida (PROTEGIDO_MINIMO ∪ plan.protected, ou `protected: true`) → só `lane: "serial"`, e com
      `accept_ref` (caminho do accept.md, relativo ao plano) + `accept_sha` IGUAL ao sha256 do conteúdo
      desse ficheiro, calculado agora. Ficheiro em falta ou sha diferente = violação.
  R6  `needs_worktree: true` → disco livre ≥ 20 GiB, medido agora.
  R7  `box: "READ"` → `owns: []` (leitura não é dona de nada), em qualquer lane.
  R8  Cada `owns` tem de ser DECIDÍVEL, numa de três formas:
      - caminho literal;
      - `prefixo/**`, com o prefixo literal;
      - prefixo literal + um glob (`*`, `?`, `[…]`) só no ÚLTIMO segmento.
      Wildcard antes do último segmento, `**` a meio, `{…}`, `.`/`..`/vazio num segmento, `/` inicial ou
      `\\` → violação (fechar na dúvida: é assim que `crates/*/build.rs` entrava em led-core sem accept).
  R9  `base_sha` é obrigatório e tem de ser o `origin/main` ATUAL, lido do workspace pela ferramenta
      (prefixo ≥ 7 hex). O plano nunca escolhe a árvore contra a qual é validado.

Sobreposição: decidida pela GRAMÁTICA da R8 (estrutural, fecha na dúvida). Os padrões são também expandidos
contra a árvore real de origin/main. Se essa expansão encontrar um ficheiro que a regra estrutural não
viu, a ferramenta FALHA com exit 2 (inconsistência interna), em vez de aceitar.

Nada aqui escreve no repo; só lê o plano, o accept referenciado, o git do workspace e as evidências.
"""
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import posixpath
import re
import shutil
import subprocess
import sys
from pathlib import Path

# A leitura de `watched:` é a do próprio audit_gate (uma regra, uma implementação).
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import audit_gate  # noqa: E402

SCHEMA = "pg-plan/1"

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
_HEX7 = re.compile(r"^[0-9a-f]{7,40}$")
_WILD = re.compile(r"[*?\[\]]")


class PlanoIlegivel(Exception):
    pass


# ── Gramática dos padrões (R8) ───────────────────────────────────────────────────

def motivo_indecidivel(p: str) -> str | None:
    """None se o padrão está numa das três formas decidíveis; senão, porquê."""
    if not isinstance(p, str) or not p:
        return "padrão vazio"
    if p.startswith("/") or "\\" in p:
        return "caminho absoluto ou com `\\`"
    if "{" in p or "}" in p:
        return "chavetas `{…}` não são suportadas"
    segs = p.split("/")
    if any(s in ("", ".", "..") for s in segs):
        return "segmento vazio, `.` ou `..` (caminho não normalizado)"
    if p.endswith("/**"):
        prefixo = p[:-3]
        return "wildcard no prefixo de `…/**`" if _WILD.search(prefixo) else None
    if "**" in p:
        return "`**` fora do sufixo `/**`"
    if any(_WILD.search(s) for s in segs[:-1]):
        return "wildcard antes do último segmento"
    return None


def forma(p: str) -> tuple[str, str, str]:
    """(tipo, prefixo, segmento): tipo ∈ {"arvore", "filhos", "literal"}. Só para padrões decidíveis."""
    if p.endswith("/**"):
        return "arvore", p[:-3], ""
    d, base = posixpath.split(p)
    if _WILD.search(base):
        return "filhos", d, base
    return "literal", p, ""


def casa(caminho: str, padrao: str) -> bool:
    """Um ficheiro concreto casa com um padrão decidível? (`dir/**` inclui o próprio `dir`.)"""
    t, pre, seg = forma(padrao)
    if t == "arvore":
        return caminho == pre or caminho.startswith(pre + "/")
    if t == "filhos":
        d, base = posixpath.split(caminho)
        return d == pre and fnmatch.fnmatchcase(base, seg)
    return caminho == pre


def sobrepoem_estrutural(a: str, b: str) -> bool:
    """Existe ALGUM caminho (existente ou futuro) que case com os dois? Fecha na dúvida."""
    ta, pa, sa = forma(a)
    tb, pb, sb = forma(b)
    if ta == "literal":
        return casa(pa, b)
    if tb == "literal":
        return casa(pb, a)
    if ta == "arvore" and tb == "arvore":
        return pa == pb or pa.startswith(pb + "/") or pb.startswith(pa + "/")
    if ta == "arvore":  # b = filhos de pb
        return pb == pa or pb.startswith(pa + "/")
    if tb == "arvore":
        return pa == pb or pa.startswith(pb + "/")
    return pa == pb  # dois globs no mesmo diretório: na dúvida, sobrepõem-se


def sobrepoem(a: str, b: str, universo: list[str]) -> bool:
    """Decisão estrutural + cruzamento com a árvore real. A árvore real nunca pode ver mais do que a regra
    estrutural; se vir, é um defeito desta ferramenta, e falha em vez de decidir."""
    estrutural = sobrepoem_estrutural(a, b)
    real = any(casa(f, a) and casa(f, b) for f in universo)
    if real and not estrutural:
        raise PlanoIlegivel(f"inconsistência interna: {a!r} e {b!r} partilham um ficheiro real que a regra "
                            f"estrutural não viu")
    return estrutural


# ── Leituras do workspace ────────────────────────────────────────────────────────

def origin_main(workspace: Path) -> str:
    r = subprocess.run(["git", "-C", str(workspace), "rev-parse", "--verify", "--quiet", "origin/main^{commit}"],
                       capture_output=True, text=True)
    if r.returncode != 0 or not r.stdout.strip():
        raise PlanoIlegivel("origin/main ilegível no workspace — sem base real não há veredito")
    return r.stdout.strip()


def ficheiros_em(workspace: Path, sha: str) -> list[str]:
    r = subprocess.run(["git", "-C", str(workspace), "ls-tree", "-r", "--name-only", sha],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise PlanoIlegivel(f"git ls-tree {sha} falhou: {r.stderr.strip()}")
    return r.stdout.splitlines()


def watched_vivos(workspace: Path) -> set[str]:
    """Todos os caminhos em `watched:` de todas as evidências (.md/.txt, em qualquer subpasta), AGORA."""
    vistos: set[str] = set()
    raiz = workspace / "docs" / "evidence"
    if not raiz.is_dir():
        return vistos
    for f in sorted(raiz.rglob("*")):
        if f.is_file() and f.suffix.lower() in {".md", ".txt"}:
            vistos.update(audit_gate.evidence_watched(f.read_text(errors="replace")))
    return vistos


def disco_livre_gib(workspace: Path) -> float:
    return shutil.disk_usage(workspace).free / 2**30


# ── Esquema ──────────────────────────────────────────────────────────────────────

def _lista_de_str(v, onde: str) -> list[str]:
    if not isinstance(v, list) or not all(isinstance(x, str) for x in v):
        raise PlanoIlegivel(f"{onde} tem de ser lista de strings")
    return v


def validar_esquema(plano) -> list[dict]:
    if not isinstance(plano, dict):
        raise PlanoIlegivel("o plano tem de ser um objeto JSON")
    if plano.get("schema") != SCHEMA:
        raise PlanoIlegivel(f"`schema` tem de ser {SCHEMA!r} (veio {plano.get('schema')!r})")
    if not isinstance(plano.get("tasks"), list) or not plano["tasks"]:
        raise PlanoIlegivel("o plano tem de ter `tasks`: lista não vazia")
    for campo in ("hot", "protected"):
        for x in _lista_de_str(plano.get(campo, []), f"`{campo}` do plano"):
            m = motivo_indecidivel(x)
            if m or forma(x)[0] == "filhos":
                raise PlanoIlegivel(f"`{campo}` do plano: {x!r} tem de ser caminho literal ou `prefixo/**` "
                                    f"({m or 'glob de último segmento não é área'})")
    for t in plano["tasks"]:
        if not isinstance(t, dict) or not isinstance(t.get("id"), str) or not t["id"]:
            raise PlanoIlegivel(f"tarefa sem `id`: {t!r}")
        if t.get("box") not in BOXES:
            raise PlanoIlegivel(f"tarefa {t['id']!r}: `box` tem de ser um de {sorted(BOXES)}")
        if t.get("lane") not in LANES:
            raise PlanoIlegivel(f"tarefa {t['id']!r}: `lane` tem de ser um de {sorted(LANES)}")
        for campo in ("deps", "owns", "forbids"):
            _lista_de_str(t.get(campo, []), f"tarefa {t['id']!r}: `{campo}`")
        for campo in ("hot", "protected", "needs_worktree"):
            if not isinstance(t.get(campo, False), bool):
                raise PlanoIlegivel(f"tarefa {t['id']!r}: `{campo}` tem de ser booleano")
        for campo in ("accept_ref", "accept_sha"):
            if t.get(campo) is not None and not isinstance(t[campo], str):
                raise PlanoIlegivel(f"tarefa {t['id']!r}: `{campo}` tem de ser string ou null")
    return plano["tasks"]


# ── Regras ───────────────────────────────────────────────────────────────────────

def _accept_valido(t: dict, plano_dir: Path) -> str | None:
    """None se o accept referenciado existe e tem o sha256 declarado; senão, porquê."""
    ref, sha = t.get("accept_ref"), t.get("accept_sha")
    if not ref or not sha:
        return "sem accept_ref + accept_sha"
    if not _SHA256.match(sha):
        return f"accept_sha {sha!r} não é sha256 (64 hex)"
    f = (plano_dir / ref) if not Path(ref).is_absolute() else Path(ref)
    if not f.is_file():
        return f"accept_ref {ref!r} não existe"
    real = hashlib.sha256(f.read_bytes()).hexdigest()
    if real != sha:
        return f"accept_sha não é o sha256 de {ref!r} (real {real[:12]}…)"
    return None


def verificar(plano, workspace: Path, disco_gib: float | None = None, plano_dir: Path | None = None) -> list[str]:
    """Lista de violações (vazia = aceite). Lança PlanoIlegivel quando não há veredito possível."""
    tarefas = validar_esquema(plano)
    plano_dir = plano_dir or workspace
    viol: list[str] = []
    ids = [t["id"] for t in tarefas]
    por_id = {t["id"]: t for t in tarefas}

    # R9 — a base é o origin/main de agora, lido aqui; nunca a escolhida pelo plano.
    base = origin_main(workspace)
    declarada = plano.get("base_sha")
    if not (isinstance(declarada, str) and _HEX7.match(declarada) and base.startswith(declarada)):
        viol.append(f"VIOLA R9: base_sha {declarada!r} não é o origin/main atual ({base[:12]})")
    universo = ficheiros_em(workspace, base)

    # R8 — owns decidíveis. Os indecidíveis ficam fora das contas seguintes (já reprovaram).
    decidiveis: dict[str, list[str]] = {}
    for t in tarefas:
        bons = []
        for p in t.get("owns", []):
            m = motivo_indecidivel(p)
            if m:
                viol.append(f"VIOLA R8: {t['id']!r} possui {p!r} — indecidível ({m})")
            else:
                bons.append(p)
        decidiveis[t["id"]] = bons

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

    # R2 — V1: paralelo só de leitura.
    for t in paralelas:
        if t["box"] != "READ":
            viol.append(f"VIOLA R2: {t['id']!r} é paralela com box {t['box']} — no V1 só READ corre em paralelo")

    # R3 — owns disjuntos entre paralelas.
    for i, a in enumerate(paralelas):
        for b in paralelas[i + 1:]:
            for pa in decidiveis[a["id"]]:
                for pb in decidiveis[b["id"]]:
                    if sobrepoem(pa, pb, universo):
                        viol.append(f"VIOLA R3: owns sobrepostos {a['id']!r}:{pa!r} × {b['id']!r}:{pb!r}")

    # R4 — nada quente num lane paralelo.
    quentes = [(q, "fixo") for q in QUENTES_FIXOS] + [(q, "plano") for q in plano.get("hot", [])] + \
              [(q, "watched") for q in sorted(watched_vivos(workspace))]
    for t in paralelas:
        if t.get("hot", False):
            viol.append(f"VIOLA R4: {t['id']!r} é paralela e está marcada hot: true")
        for p in decidiveis[t["id"]]:
            for q, origem in quentes:
                if motivo_indecidivel(q) is None and sobrepoem(p, q, universo):
                    viol.append(f"VIOLA R4: {t['id']!r} é paralela e possui {p!r}, quente ({origem}: {q})")

    # R5 — área protegida: só em série e com o accept verificado.
    protegidos = PROTEGIDO_MINIMO + list(plano.get("protected", []))
    for t in tarefas:
        toca = [p for p in decidiveis[t["id"]] if any(sobrepoem(p, q, universo) for q in protegidos)]
        if t.get("protected", False) or toca:
            motivo = f"possui {toca}" if toca else "protected: true"
            if t["lane"] != "serial":
                viol.append(f"VIOLA R5: {t['id']!r} é área protegida ({motivo}) e não é serial")
            m = _accept_valido(t, plano_dir)
            if m:
                viol.append(f"VIOLA R5: {t['id']!r} é área protegida ({motivo}): {m}")

    # R6 — worktree só com ≥ 20 GiB livres, medido agora.
    if any(t.get("needs_worktree", False) for t in tarefas):
        livre = disco_livre_gib(workspace) if disco_gib is None else disco_gib
        for t in tarefas:
            if t.get("needs_worktree", False) and livre < MIN_DISCO_GIB:
                viol.append(f"VIOLA R6: {t['id']!r} pede worktree com {livre:.2f} GiB livres (< {MIN_DISCO_GIB})")

    # R7 — leitura não é dona de nada, em qualquer lane.
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
        viol = verificar(plano, Path(a.workspace), plano_dir=Path(a.plano).resolve().parent)
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
