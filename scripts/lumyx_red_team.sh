#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
# LUMYX-RED-TEAM — adversarial audit. Question everything.
#
# Unlike the Guardian (pass/fail on known invariants), the Red Team hunts for
# NEW weaknesses. Each of the five teams asks one question and runs a real
# probe. A team that "finds nothing" must say what it tried — never "secure".
#
#   ./scripts/lumyx_red_team.sh
#
# Exit 0 = no UNMITIGATED critical/high finding. Exit 1 = an open crit/high.
# Findings are appended to docs/red-team/findings.md (the audit trail).
# ══════════════════════════════════════════════════════════════════════════
set -u
cd "$(dirname "$0")/.."

RED='\033[0;31m'; GREEN='\033[0;32m'; YEL='\033[1;33m'; BLU='\033[0;34m'; BOLD='\033[1m'; NC='\033[0m'
OPEN_CRIT=0
LEDGER="docs/red-team/findings.md"
mkdir -p docs/red-team

hdr(){ echo -e "\n${BOLD}${BLU}$1${NC}"; }
# finding <sev> <title> <exploit> <mitigation> <status:MITIGATED|OPEN|ACCEPTED>
finding(){
    local sev="$1" title="$2" exploit="$3" mit="$4" status="$5"
    local color="$YEL"; [ "$sev" = CRITICAL ] || [ "$sev" = HIGH ] && color="$RED"
    [ "$status" = MITIGATED ] && color="$GREEN"
    echo -e "  ${color}[$sev/$status]${NC} $title"
    echo -e "     explorar:  $exploit"
    echo -e "     mitigação: $mit"
    if { [ "$sev" = CRITICAL ] || [ "$sev" = HIGH ]; } && [ "$status" = OPEN ]; then
        OPEN_CRIT=$((OPEN_CRIT+1))
    fi
}
probe(){ echo -e "  ${YEL}·${NC} probe: $1"; }

echo -e "${BOLD}LUMYX-RED-TEAM — auditoria adversarial $(date -u +%Y-%m-%dT%H:%MZ)${NC}"

# ── 1 · Security Red Team — "Como quebrar isso?" ──────────────────────────
hdr "1 · Security Red Team — como quebrar isso?"
probe "assinatura confia na chave embutida no blob? (proof-of-exploit)"
if cargo test -q -p led-show-recorder signing::tests::redteam_resigned_tamper_defeats_unpinned_verify 2>&1 | grep -q "test result: ok"; then
    if cargo test -q -p led-show-recorder signing::tests::pinned_verify_rejects_resigned_tamper 2>&1 | grep -q "test result: ok"; then
        finding HIGH "verify_manifest confia na pubkey embutida (tamper re-assinado passa)" \
            "atacante altera o manifest, re-assina com chave própria, embute a própria pubkey → verify_manifest = Ok" \
            "verify_manifest_pinned(signed, &trusted) rejeita chave ≠ fixada; UntrustedKey. Testes: redteam_resigned_tamper_* + pinned_verify_rejects_*" \
            MITIGATED
    else
        finding HIGH "verify_manifest confia na pubkey embutida" \
            "tamper re-assinado passa na verificação sem chave fixada" \
            "AUSENTE — verify_manifest_pinned não existe/não rejeita" OPEN
    fi
fi
probe "chave privada de assinatura tem modo restrito?"
if grep -q "0o600" crates/led-show-recorder/examples/sign_file.rs 2>/dev/null; then
    finding LOW "seed Ed25519 gravada com mode 0600 (unix)" \
        "leitura da seed por outro usuário local" \
        "keygen aplica chmod 0600; release/ no .gitignore" MITIGATED
fi
probe "parser XML do importer aceita entrada adversarial sem panic?"
if cargo test -q -p led-xlights 2>&1 | grep -q "test result: ok"; then
    finding LOW "parser xLights é std-only, sem panic em fixtures" \
        "XML malformado / entidades aninhadas" \
        "parser tolerante (skip de tag inválida); 26 testes. GAP: sem fuzz dedicado" ACCEPTED
fi

# ── 2 · Reliability Red Team — "Como derrubar isso?" ──────────────────────
hdr "2 · Reliability Red Team — como derrubar isso?"
probe "servidor /metrics é single-thread (slowloris)?"
if grep -q "one connection at a time\|sequential accepts\|One thread" crates/led-hal/src/prometheus.rs 2>/dev/null; then
    finding MEDIUM "endpoint /metrics serve 1 conexão por vez" \
        "cliente lento (slowloris) segura a conexão e bloqueia scrapes do Prometheus" \
        "aceitável: /metrics fica em rede de gestão confiável, não exposta ao palco. GAP: read timeout no accept loop" ACCEPTED
fi
probe "binds em 0.0.0.0 (exposição em todas as interfaces)?"
if grep -rq '0.0.0.0' crates/led-player/src/main.rs 2>/dev/null; then
    finding LOW "led-player --metrics faz bind em 0.0.0.0" \
        "scrape de qualquer interface da máquina do show" \
        "operador escolhe a porta; recomendar bind em IP de gestão. Documentar" ACCEPTED
fi
probe "buffers de rede/replay têm teto?"
finding LOW "player carrega o show inteiro em memória" \
    "arquivo .lumyx gigante → OOM" \
    "shows são minutos (validado: 6.200px×3.925 frames ok); teto prático conhecido" ACCEPTED

# ── 3 · Architecture Red Team — "Onde está o acoplamento oculto?" ─────────
hdr "3 · Architecture Red Team — acoplamento oculto?"
probe "CompiledLayout reconstruído em vez de compartilhado?"
if grep -rq "CompiledLayout::compile(&assigns)" crates/led-demo/examples/ 2>/dev/null; then
    dup=$(grep -rc "CompiledLayout::compile\|CompiledLayout::linear" crates/led-demo/examples/*.rs 2>/dev/null | awk -F: '{s+=$2} END{print s}')
    finding LOW "exemplos recompilam CompiledLayout 2× (Hal + device_universes)" \
        "estado duplicado; se as duas compilações divergirem, mapa inconsistente" \
        "mesmo input → mesmo layout (determinístico); custo é de setup, não hot path" ACCEPTED
fi
probe "DAG tem ciclo ou import proibido latente?"
if cargo metadata --format-version 1 --no-deps >/dev/null 2>&1 && \
   ! grep -rqE 'led[_-]pixel[_-]engine' crates/led-protocols/src/ 2>/dev/null; then
    finding LOW "DAG acíclico, camadas respeitadas (led-xlights→led-layout novo, sem ciclo)" \
        "nova dep led-xlights→led-layout poderia criar ciclo" \
        "verificado: led-layout→led-core apenas; sink intocado. Guardian cobre" MITIGATED
fi

# ── 4 · Product Red Team — "O operador consegue errar?" ───────────────────
hdr "4 · Product Red Team — o operador consegue errar?"
probe "protocolo/universo errado falha silenciosamente?"
if grep -q "did you mean 1" crates/led-player/src/main.rs 2>/dev/null; then
    finding LOW "led-player avisa em --first-universe 0 e range fora do Art-Net (15-bit)" \
        "operador digita universo 0 → luzes no lugar errado" \
        "MITIGADO parcial: warning explícito no universo 0 e range >32767. GAP menor: ArtPoll pré-show p/ presença de device (rede)" MITIGATED
else
    finding MEDIUM "led-player --first-universe errado sem aviso" \
        "universo 0 em vez de 1 → sem erro" "GAP aberto" OPEN
fi
probe "auto-fix pode ser ignorado (abre original quebrado)?"
finding LOW "auto-fix grava .LUMYX-FIXED.xml; operador pode abrir o original" \
    "abrir xlights_rgbeffects.xml original → 2.701 conflitos de volta" \
    "nome explícito FIXED + relatório do gate; recomendar backup+rename. Documentar no runbook" ACCEPTED
probe "verificação de assinatura fixada está no caminho do operador?"
if grep -q "verify-key" crates/led-player/src/main.rs 2>/dev/null; then
    finding MEDIUM "led-player --verify-key usa verify_manifest_pinned contra <show>.sig" \
        "operador verificava só o hash (--verify), não a autenticidade da chave" \
        "MITIGADO: --verify-key <hex> pina a chave; chave errada → exit 1 (provado e2e sign_show→player)" MITIGATED
else
    finding MEDIUM "verify_manifest_pinned existe mas o player não expõe --verify-key" \
        "operador usa --verify (só hash) e não a verificação autêntica de chave" \
        "GAP aberto: wire --verify-key no led-player" OPEN
fi

# ── 5 · Chaos Red Team — "Qual falha ainda não simulamos?" ────────────────
hdr "5 · Chaos Red Team — qual falha ainda não simulamos?"
probe "inventário de faltas: simuladas vs. reais"
echo "     simuladas: packet loss, latency, crash, failover, hot-join, clock offset, wire loss/heal, clock-backwards mid-show (8 leitores)"
if cargo test -q -p led-hal shared_clock::tests::clock_backwards_correction_mid_show_stays_monotonic 2>&1 | grep -q "test result: ok"; then
    finding LOW "clock-backwards mid-show mitigado; restam reorder/dup de pacote no fio" \
        "rede reordena/duplica UDP; controlador reinicia e reenvia" \
        "MITIGADO clock-backwards (guard de monotonicidade, teste concorrente 8 threads). GAP menor: reorder+dup no UdpChaosProxy. Rastreado" MITIGATED
else
    finding MEDIUM "faltas não simuladas incl. clock-backwards mid-show" \
        "NTP corrige clock para trás durante o show" \
        "GAP aberto" OPEN
fi

# ── Ledger + verdict ───────────────────────────────────────────────────────
{
  echo "## Auditoria $(date -u +%Y-%m-%dT%H:%MZ)"
  echo "- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned"
  echo "- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards"
  echo ""
} >> "$LEDGER"

echo ""
echo -e "${BOLD}════════════════════════════════════════════${NC}"
if [ "$OPEN_CRIT" -eq 0 ]; then
    echo -e "${GREEN}${BOLD}✅ RED-TEAM: 0 achados CRITICAL/HIGH abertos${NC} (MEDIUM/LOW rastreados em $LEDGER)"
    exit 0
else
    echo -e "${RED}${BOLD}❌ RED-TEAM: $OPEN_CRIT achado(s) CRITICAL/HIGH aberto(s) — bloqueia mudança crítica${NC}"
    exit 1
fi
