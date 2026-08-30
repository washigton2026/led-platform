#!/usr/bin/env python3
"""Amostra Opkts/Oerrs por interface durante o burn-in.

Responde a UMA pergunta: durante o surto de ENOBUFS, o trafego muda de
interface? A janela de 60s medida em 2026-08-28 nao teve falhas, portanto
mostrou o caminho normal e nao o caminho durante o surto.
"""
import subprocess
import sys
import time

IFACES = ("en0", "en7")
PERIOD = 0.2


def sample():
    """Devolve {iface: (opkts, oerrs, ipkts)} das linhas Link# do netstat."""
    out = subprocess.run(
        ["netstat", "-ib"], capture_output=True, text=True, timeout=5
    ).stdout
    vals = {}
    for line in out.splitlines():
        f = line.split()
        if len(f) < 11 or not f[2].startswith("<Link#"):
            continue
        if f[0] in IFACES and f[0] not in vals:
            # Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll
            vals[f[0]] = (int(f[7]), int(f[8]), int(f[4]))
    return vals


def main():
    deadline = time.time() + float(sys.argv[1] if len(sys.argv) > 1 else 600)
    base = sample()
    if not all(i in base for i in IFACES):
        print(f"ABORT: interfaces em falta, vi {sorted(base)}", file=sys.stderr)
        return 2

    t0 = time.time()
    print("epoch,t_rel,en0_opkts,en0_oerrs,en7_opkts,en7_oerrs", flush=True)
    while time.time() < deadline:
        try:
            v = sample()
        except Exception as e:  # nunca matar a amostragem por um erro pontual
            print(f"# erro de amostra: {e}", flush=True)
            time.sleep(PERIOD)
            continue
        if all(i in v for i in IFACES):
            print(
                "%.3f,%.3f,%d,%d,%d,%d"
                % (
                    time.time(),
                    time.time() - t0,
                    v["en0"][0] - base["en0"][0],
                    v["en0"][1] - base["en0"][1],
                    v["en7"][0] - base["en7"][0],
                    v["en7"][1] - base["en7"][1],
                ),
                flush=True,
            )
        time.sleep(PERIOD)
    return 0


if __name__ == "__main__":
    sys.exit(main())
