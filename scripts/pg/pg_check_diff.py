#!/usr/bin/env python3
"""
pg_check_diff.py — o diff de uma tarefa do Parallel Gauntlet V1 ficou dentro da sua área?

Uso:
  python3 scripts/pg/pg_check_diff.py <base> <ramo> <card.md> [--repo <repo>]

Lê `owns:` e `forbids:` do front-matter do card (entre as duas linhas `---`) e lista os caminhos tocados
por `git diff --name-only --no-renames -z <base>...<ramo>`. É o que o ramo trouxe desde o merge-base, e um
rename aparece com os DOIS lados (o antigo como apagado, o novo como acrescentado), qualquer que seja o
`diff.renames` da máquina. Uma remoção também conta como caminho tocado. Exige:
  - cada `owns` e `forbids` decidível (a mesma gramática da R8 do pg_check_plan); senão → violação;
  - todo o caminho tocado casa com algum `owns` (os dois lados de um rename incluídos);
  - nenhum caminho tocado casa com algum `forbids`.
Exit 0 = dentro da área. Exit 1 = viola (cada caminho e porquê). Exit 2 = card ou git ilegível.

Uma tarefa READ tem `owns: []`: qualquer ficheiro alterado é violação — é o ponto.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pg_check_plan import casa, motivo_indecidivel  # noqa: E402  (uma gramática, uma implementação)


class CardIlegivel(Exception):
    pass


def _lista_yaml(valor: str) -> list[str]:
    """`[a, b]` ou `[]` — o subconjunto de YAML que o card usa. Mais do que isto é recusado."""
    v = valor.strip()
    if not (v.startswith("[") and v.endswith("]")):
        raise CardIlegivel(f"esperava lista `[...]`, veio {valor!r}")
    corpo = v[1:-1].strip()
    if not corpo:
        return []
    return [x.strip().strip("'\"") for x in corpo.split(",") if x.strip()]


def ler_card(texto: str) -> dict[str, list[str]]:
    linhas = texto.splitlines()
    if not linhas or linhas[0].strip() != "---":
        raise CardIlegivel("o card tem de começar por front-matter `---`")
    try:
        fim = next(i for i in range(1, len(linhas)) if linhas[i].strip() == "---")
    except StopIteration:
        raise CardIlegivel("front-matter sem `---` de fecho") from None
    campos: dict[str, list[str]] = {}
    for l in linhas[1:fim]:
        if ":" not in l or l.lstrip().startswith("#"):
            continue
        k, v = l.split(":", 1)
        k = k.strip()
        if k in ("owns", "forbids"):
            if k in campos:
                raise CardIlegivel(f"`{k}` declarado duas vezes")
            campos[k] = _lista_yaml(v)
    for k in ("owns", "forbids"):
        if k not in campos:
            raise CardIlegivel(f"o front-matter não declara `{k}`")
    return campos


def ficheiros_alterados(repo: Path, base: str, ramo: str) -> list[str]:
    r = subprocess.run(["git", "-C", str(repo), "diff", "--name-only", "--no-renames", "-z", f"{base}...{ramo}"],
                       capture_output=True)
    if r.returncode != 0:
        raise CardIlegivel(f"git diff {base}...{ramo} falhou: {r.stderr.decode(errors='replace').strip()}")
    return [c for c in r.stdout.decode("utf-8", errors="surrogateescape").split("\0") if c]


def verificar(alterados: list[str], owns: list[str], forbids: list[str]) -> list[str]:
    viol = []
    for campo, padroes in (("owns", owns), ("forbids", forbids)):
        for p in padroes:
            m = motivo_indecidivel(p)
            if m:
                viol.append(f"INDECIDÍVEL: {campo} {p!r} ({m})")
    owns = [p for p in owns if motivo_indecidivel(p) is None]
    forbids = [p for p in forbids if motivo_indecidivel(p) is None]
    for f in alterados:
        proibidos = [p for p in forbids if casa(f, p)]
        if proibidos:
            viol.append(f"PROIBIDO: {f} (casa com forbids {proibidos})")
        if not any(casa(f, p) for p in owns):
            viol.append(f"FORA DE owns: {f}")
    return viol


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    ap.add_argument("base")
    ap.add_argument("ramo")
    ap.add_argument("card")
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[2]))
    a = ap.parse_args(argv)
    try:
        card = ler_card(Path(a.card).read_text())
        alterados = ficheiros_alterados(Path(a.repo), a.base, a.ramo)
    except (OSError, CardIlegivel) as e:
        print(f"pg_check_diff: ILEGÍVEL — {e}")
        return 2
    viol = verificar(alterados, card["owns"], card["forbids"])
    if viol:
        for v in viol:
            print(v)
        print(f"pg_check_diff: RECUSADO — {len(viol)} violação(ões) em {len(alterados)} ficheiro(s)")
        return 1
    print(f"pg_check_diff: OK — {len(alterados)} ficheiro(s), todos dentro de owns e fora de forbids")
    return 0


if __name__ == "__main__":
    sys.exit(main())
