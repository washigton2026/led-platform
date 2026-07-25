#!/usr/bin/env bash
# LUMYX burn-in runner — loops the show against real hardware for N hours,
# logging one JSON line per pass. Aborts (exit 1) on the first integrity or
# delivery failure; that abort IS the burn-in finding.
#
# Usage:
#   ./scripts/burnin.sh 72 show.lumyx 192.168.2.156     # 72h against WLED
#   ./scripts/burnin.sh 1  show.lumyx                    # 1h against simulator
set -euo pipefail

HOURS="${1:?usage: burnin.sh <hours> <show.lumyx> [artnet-ip]}"
SHOW="${2:?usage: burnin.sh <hours> <show.lumyx> [artnet-ip]}"
IP="${3:-}"

LOG="burnin-$(date +%Y%m%d-%H%M%S).jsonl"
END=$(( $(date +%s) + HOURS * 3600 ))

ARGS=("$SHOW" "--loop" "0")
if [ -n "$IP" ]; then ARGS+=("--artnet" "$IP" "--first-universe" "1"); fi

echo "burn-in: ${HOURS}h, show=$SHOW, target=${IP:-simulator}, log=$LOG"

# led-player --loop 0 runs forever; we bound it by wall clock and kill cleanly.
cargo run --release -q -p led-player -- "${ARGS[@]}" >> "$LOG" 2>&1 &
PID=$!

trap 'kill $PID 2>/dev/null || true' EXIT

while kill -0 "$PID" 2>/dev/null; do
    if [ "$(date +%s)" -ge "$END" ]; then
        echo "burn-in window complete (${HOURS}h) — stopping player"
        kill "$PID"
        wait "$PID" 2>/dev/null || true
        PASSES=$(grep -c '"pass"' "$LOG" || echo 0)
        echo "RESULT: PASS — $PASSES passes, 0 aborts (log: $LOG)"
        exit 0
    fi
    sleep 10
done

# Player exited on its own → it aborted on a failure.
wait "$PID" || true
echo "RESULT: FAIL — player aborted before the window ended (log: $LOG)"
tail -3 "$LOG"
exit 1
