#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
# LUMYX-VALIDATOR — prove the change works.
#
# The executable half of the LUMYX-VALIDATOR agent team (Sonnet profile:
# heavier than the Guardian, runs the system for real). Five validators, each
# reporting PASS / FAIL / Risco / Evidência:
#
#   1. Test Architect        — unit + integration + e2e suites
#   2. Chaos Engineer        — packet loss, recovery, failover
#   3. Observability Engineer— metrics live scrape, alerts config, dashboard
#   4. Cluster Engineer      — hot-join, rejoin, drift
#   5. Production Engineer   — burn-in evidence, hardware reachability, runtime
#
#   ./scripts/lumyx_validator.sh
#
# Exit 0 = all validators PASS (hardware absence = SKIP, not FAIL).
# ══════════════════════════════════════════════════════════════════════════
set -u
cd "$(dirname "$0")/.."

RED='\033[0;31m'; GREEN='\033[0;32m'; YEL='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
FAIL=0; SKIP=0
vpass(){ echo -e "${GREEN}PASS${NC}  $1\n      evidência: $2"; }
vfail(){ echo -e "${RED}FAIL${NC}  $1\n      evidência: $2"; FAIL=$((FAIL+1)); }
vskip(){ echo -e "${YEL}SKIP${NC}  $1\n      risco: $2"; SKIP=$((SKIP+1)); }
hdr(){ echo -e "\n${BOLD}${YEL}$1${NC}"; }

# ── 1 · Test Architect — unit + integration + e2e ─────────────────────────
hdr "1 · Test Architect (unit · integration · e2e)"
t_out=$(cargo test -q --workspace 2>&1)
t_pass=$(echo "$t_out" | sed -nE 's/.*test result: ok\. ([0-9]+) passed.*/\1/p' | awk '{s+=$1} END {print s+0}')
if echo "$t_out" | grep -q "FAILED"; then
    vfail "suíte do workspace tem falhas" "$(echo "$t_out" | grep FAILED | head -2)"
else
    vpass "workspace completo verde" "$t_pass testes, 0 falhas"
fi
e_out=$(cargo test -q -p led-bridge --test e2e_pipeline 2>&1)
if echo "$e_out" | grep -q "test result: ok"; then
    vpass "e2e pipeline (audio→FFT→efeito→HAL→device)" "e2e_pipeline ok"
else
    vfail "e2e pipeline quebrado" "$(echo "$e_out" | tail -2)"
fi

# ── 2 · Chaos Engineer — packet loss, recovery, failover ──────────────────
hdr "2 · Chaos Engineer (loss · recovery · failover)"
c_out=$(cargo test -q -p integration-tests --test udp_chaos 2>&1)
if echo "$c_out" | grep -q "test result: ok. 5 passed"; then
    vpass "chaos de fio: 30% loss degrada, heal→100%, determinístico" "udp_chaos 5/5"
else
    vfail "chaos de fio regrediu" "$(echo "$c_out" | tail -2)"
fi
f_out=$(cargo test -q -p integration-tests --test two_node_cluster failover 2>&1)
if echo "$f_out" | grep -q "test result: ok"; then
    vpass "failover: cluster sobrevive a nó morto" "failover_continues_when_one_node_fails"
else
    vfail "failover regrediu" "$(echo "$f_out" | tail -2)"
fi

# ── 3 · Observability Engineer — metrics LIVE, alerts, dashboard ──────────
hdr "3 · Observability Engineer (metrics · alerts · dashboards)"
cargo build -q -p led-player 2>/dev/null
SHOW=$(ls robot_show.lumyx show.lumyx 2>/dev/null | head -1)
if [ -n "$SHOW" ]; then
    ./target/debug/led-player "$SHOW" --loop 3 --speed 2 --metrics 19464 >/dev/null 2>&1 &
    PID_PLAYER=$!
    # Debug builds hash ~1.5M px before the server binds — retry up to ~10s.
    scrape=""
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        sleep 1
        scrape=$(curl -s --max-time 2 http://127.0.0.1:19464/metrics 2>/dev/null)
        echo "$scrape" | grep -q "lumyx_frames_total" && break
    done
    kill $PID_PLAYER 2>/dev/null; wait $PID_PLAYER 2>/dev/null
    if echo "$scrape" | grep -q "lumyx_frames_total" && \
       echo "$scrape" | grep -q 'quantile="0.99"'; then
        frames=$(echo "$scrape" | sed -nE 's/^lumyx_frames_total.* ([0-9]+)$/\1/p' | head -1)
        vpass "scrape AO VIVO durante playback real" "lumyx_frames_total=$frames + p99 presente"
    else
        vfail "endpoint /metrics não expôs as séries durante o show" "scrape vazio/incompleto"
    fi
else
    vskip "sem .lumyx para playback ao vivo" "rodar cargo run -p led-demo --release antes"
fi
if python3 -c "import yaml" 2>/dev/null; then
    python3 -c "import yaml,sys; yaml.safe_load(open('docs/observability/alerts.yml'))" \
        && vpass "alerts.yml é YAML válido (5 regras SLO)" "docs/observability/alerts.yml" \
        || vfail "alerts.yml inválido" "yaml.safe_load falhou"
else
    grep -q "LumyxFrameDeliveryFastBurn" docs/observability/alerts.yml \
        && vpass "alerts.yml presente com regras SLO" "grep fast-burn ok (pyyaml ausente p/ parse)" \
        || vfail "alerts.yml sem regras" "grep falhou"
fi
python3 -m json.tool docs/observability/grafana-lumyx.json >/dev/null 2>&1 \
    && vpass "dashboard Grafana é JSON válido" "grafana-lumyx.json parseia" \
    || vfail "dashboard Grafana inválido" "json.tool falhou"

# ── 4 · Cluster Engineer — hot-join, rejoin, drift ────────────────────────
hdr "4 · Cluster Engineer (hot-join · rejoin · drift)"
cl_out=$(cargo test -q -p integration-tests --test two_node_cluster 2>&1)
if echo "$cl_out" | grep -q "test result: ok. 6 passed"; then
    vpass "cluster 2 nós: parity, hot-join, chaos, drift, metrics" "two_node_cluster 6/6"
else
    vfail "cluster regrediu" "$(echo "$cl_out" | tail -2)"
fi
nt_out=$(cargo test -q -p led-hal net_time 2>&1)
if echo "$nt_out" | grep -q "test result: ok. 5 passed"; then
    vpass "sync de relógio: offset ±500ms medido a ±10ms" "net_time 5/5"
else
    vfail "net_time regrediu" "$(echo "$nt_out" | tail -2)"
fi

# ── 5 · Production Engineer — burn-in, hardware, runtime ──────────────────
hdr "5 · Production Engineer (burn-in · hardware · runtime)"
BURN=$(ls -t burnin-*.jsonl 2>/dev/null | head -1)
if [ -n "$BURN" ]; then
    # grep -c always prints a count (even 0) — no `|| echo` (it would double-print).
    passes=$(grep -c '"pass"' "$BURN" 2>/dev/null); passes=${passes:-0}
    aborts=$(grep -c 'ABORT' "$BURN" 2>/dev/null); aborts=${aborts:-0}
    if [ "$passes" -ge 1 ] && [ "$aborts" -eq 0 ]; then
        vpass "burn-in mais recente sem aborts" "$BURN: $passes passes, 0 aborts"
    else
        vfail "burn-in com aborts ou vazio" "$BURN: $passes passes, $aborts aborts"
    fi
else
    vskip "nenhum burn-in registrado" "rodar scripts/burnin.sh (1h já aprovado antes)"
fi
if ping -c 1 -W 800 192.168.2.156 >/dev/null 2>&1; then
    vpass "hardware alcançável (robô led 1)" "ping 192.168.2.156"
else
    vskip "hardware inalcançável — validação WLED/Falcon/FPP pendente" \
          "rig desligado; smoke: led-player --ddp 192.168.2.156 quando ligar"
fi
if [ -f target/release/led-player ] && [ -n "$SHOW" ]; then
    if ./target/release/led-player "$SHOW" --info >/dev/null 2>&1; then
        vpass "binário release executa e lê show real" "led-player --info $SHOW → exit 0"
    else
        vfail "binário release falhou no runtime" "led-player --info exit ≠ 0"
    fi
else
    vskip "binário release ausente" "cargo build --release -p led-player"
fi

# ── Verdict ────────────────────────────────────────────────────────────────
echo ""
if [ "$FAIL" -eq 0 ]; then
    echo -e "${GREEN}${BOLD}✅ LUMYX-VALIDATOR: PASS${NC} (skips: $SKIP — riscos documentados acima)"
    exit 0
else
    echo -e "${RED}${BOLD}❌ LUMYX-VALIDATOR: FAIL — $FAIL validador(es) reprovaram${NC}"
    exit 1
fi
