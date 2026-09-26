#!/usr/bin/env bash
# LUMYX burn-in runner — loops the show against real hardware for N hours,
# logging one JSON line per pass. Aborts (exit 1) on the first integrity or
# delivery failure; that abort IS the burn-in finding.
#
# Usage:
#   ./scripts/burnin.sh 72 show.lumyx 192.168.2.156     # 72h against WLED
#   ./scripts/burnin.sh 1  show.lumyx                    # 1h against simulator
#
# PRECONDIÇÃO — o host NÃO pode suspender a meio da janela.
#
# Medido em 2026-08-30: a bancada corre a bateria com idle sleep activo. Um burn-in
# arrancado às 16:48 apanhou `Entering Sleep state due to 'Idle Sleep'` às 16:50:45 e,
# 2 s depois, o AQM do macOS abriu flow advisory em `en7` — 46 frames falhados em 94,
# aborto ao pass 15. NÃO era o fenómeno sob teste: era o ensaio interrompido pela
# gestão de energia. A mesma corrida com o sleep inibido deu 150/150 passes limpos.
# Um aborto por suspensão é indistinguível, no log do player, de um aborto real.
# Evidência: docs/evidence/gs45-burnin-router-2026-08-30/
set -uo pipefail

HOURS="${1:?usage: burnin.sh <hours> <show.lumyx> [artnet-ip]}"
SHOW="${2:?usage: burnin.sh <hours> <show.lumyx> [artnet-ip]}"
IP="${3:-}"

LOG="burnin-$(date +%Y%m%d-%H%M%S).jsonl"
END=$(( $(date +%s) + HOURS * 3600 ))

ARGS=("$SHOW" "--loop" "0")
if [ -n "$IP" ]; then ARGS+=("--artnet" "$IP" "--first-universe" "1"); fi

# `caffeinate -dims` inibe display/idle/disco/sistema enquanto o filho corre, e sai com
# ele. Onde não existir (Linux), corremos na mesma e DIZEMOS que a precondição não está
# garantida — em vez de a assumir em silêncio, que é como o ensaio de 30-08 se perdeu.
if command -v caffeinate >/dev/null 2>&1; then
    CAFFEINATE=(caffeinate -dims)
    SLEEP_NOTE="sleep inibido (caffeinate -dims)"
else
    CAFFEINATE=()
    SLEEP_NOTE="AVISO: caffeinate ausente — suspensão do host NAO inibida; um aborto pode ser artefacto"
fi

echo "burn-in: ${HOURS}h, show=$SHOW, target=${IP:-simulator}, log=$LOG"
echo "         $SLEEP_NOTE"

# led-player --loop 0 runs forever; we bound it by wall clock and kill cleanly.
"${CAFFEINATE[@]}" cargo run --release -q -p led-player -- "${ARGS[@]}" >> "$LOG" 2>&1 &
PID=$!

trap 'kill $PID 2>/dev/null || true' EXIT

while kill -0 "$PID" 2>/dev/null; do
    if [ "$(date +%s)" -ge "$END" ]; then
        echo "burn-in window complete (${HOURS}h) — stopping player"
        kill "$PID"
        wait "$PID" 2>/dev/null || true

        # `grep -c` devolve 1 e imprime "0" quando não há acerto; o `|| echo 0` antigo
        # acrescentava um SEGUNDO zero e produzia a string "0\n0" (bug de shell já
        # registado neste repo em 2026-07-11b). Contamos com grep -c e um default só
        # se a variável ficar vazia.
        PASSES=$(grep -c '"pass"' "$LOG")
        PASSES=${PASSES:-0}

        # Zero passes numa janela completa não é sucesso: é um ensaio que não exercitou
        # nada (KB-012). Sem isto, um player que arrancasse e nunca produzisse um pass
        # sairia daqui com "RESULT: PASS — 0 passes".
        if [ "$PASSES" -eq 0 ]; then
            echo "RESULT: NAO MEDIDO — a janela fechou com 0 passes; o burn-in nao exercitou o caminho (log: $LOG)"
            exit 2
        fi

        echo "RESULT: PASS — $PASSES passes, 0 aborts (log: $LOG) [$SLEEP_NOTE]"
        exit 0
    fi
    sleep 10
done

# Player exited on its own → it aborted on a failure.
wait "$PID" || true
echo "RESULT: FAIL — player aborted before the window ended (log: $LOG)"
echo "        Antes de tratar isto como falha real, confirmar que o host nao suspendeu:"
echo "        pmset -g log | grep -E 'Entering Sleep|Wake from'"
tail -3 "$LOG"
exit 1
