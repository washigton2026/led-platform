#!/usr/bin/env bash
# TD-030 — gate do latch de sequência (src/seqlock.rs) e dos oráculos do TD-030.
#
# Corre IGUAL em local e na CI (job `loom-td030`). Três partes, todas bloqueantes:
#
#   A. Cobertura não retirada em silêncio (R01, R02, R04):
#      - zero `#[ignore]` / `cfg_attr(..ignore..)` nos ficheiros do TD-030;
#      - cada oráculo listado em REQ_STD tem de aparecer como `... ok` (o `cargo test` dá exit 0
#        com um teste `ignored`; aqui isso reprova).
#   B. Loom executado de verdade (R03, R04, KB-012):
#      - N_esperado vem de `-- --list` com o MESMO RUSTFLAGS e filtro; piso PISO_LOOM;
#      - N_executado (do resumo, em ficheiro, KB-013) == N_esperado; cada nome em REQ_LOOM `... ok`;
#      - as linhas de não-vacuidade `LOOM_O3 k=1 err=N` e `k=2 err=N` com N >= 1.
#   C. O gate sabe reprovar (M01–M03, M05–M07, M09): cada patch em td030_canarios/ é aplicado,
#      o loom TEM de falhar por asserção em src/seqlock.rs (não por erro de compilação), e o patch
#      é revertido. Um canário que já não aplica reprova: o gate tem de ser atualizado com o código.
#
# Uso: scripts/gates/td030_loom_gate.sh            (A + B + C)
#      TD030_SEM_CANARIOS=1 scripts/gates/td030_loom_gate.sh   (só A + B, para iterar localmente;
#      a CI NUNCA define esta variável — o passo recusa-a)
set -euo pipefail

CRATE=crates/led-pixel-engine
SEQ=$CRATE/src/seqlock.rs
FICHEIROS_TD030=("$SEQ" "$CRATE/src/reactive.rs" "$CRATE/tests/td030_estrutural.rs" "$CRATE/tests/no_alloc_reativo.rs")
CANARIOS_DIR=scripts/gates/td030_canarios
PISO_LOOM=2
REQ_LOOM=(
  seqlock::loom_tests::loom_snapshot_aceite_nunca_mistura_nem_recua
  seqlock::loom_tests::loom_ler_esgotado_conta_as_tentativas
)
# Oráculos std do TD-030 (O1–O5, isolamento, coerência original). Nomes como o libtest os imprime.
REQ_STD=(
  reactive::adversarial_tests::audioshare_scalars_beat_timestamp_coherent_under_concurrency
  reactive::adversarial_tests::audioshare_isolamento_entre_instancias
  reactive::tests::codec_preserva_nan_e_inf_em_cada_campo_f32
  reactive::tests::codec_preserva_nan_nao_canonico_em_cada_campo_f32
  reactive::no_alloc_fallback::fallback_forcado_nao_aloca
  reactive::no_alloc_fallback::fallback_sem_snapshot_nao_aloca
  seqlock::tests::modelo_sequencial_contra_registo
  seqlock::tests::frescura_concorrente_monotonica_por_leitor
  render_sem_lock_nem_alocacao_no_fecho_de_chamadas
  scalars_e_uma_unica_chamada_com_tentativas_do_leitor
  ler_conta_as_tentativas_por_estrutura
  escritor_unico_por_estrutura
  os_detetores_veem_o_que_foi_plantado
  efeitos_reativos_renderizam_sem_alocar
)

OUT="${RUNNER_TEMP:-$(mktemp -d)}/td030-gate"
mkdir -p "$OUT"
falha() { echo "::error::TD-030 gate: $*"; exit 1; }

for f in "${FICHEIROS_TD030[@]}"; do [ -f "$f" ] || falha "$f não existe — a guarda seria vácua."; done

# O ambiente é fixado pelo gate; configuração externa não o pode sobrepor (mesma regra do job miri, W4).
if [ -n "${RUSTFLAGS:-}${CARGO_ENCODED_RUSTFLAGS:-}" ]; then falha "RUSTFLAGS externo definido."; fi
for c in .cargo/config .cargo/config.toml "$CRATE/.cargo/config" "$CRATE/.cargo/config.toml"; do
  if [ -f "$c" ] && grep -nE 'rustflags|\[env\]' "$c"; then falha "$c pode alterar as flags do loom."; fi
done

# ── A. cobertura não retirada em silêncio ────────────────────────────────────
if grep -nE '#\[ignore|cfg_attr\([^)]*ignore' "${FICHEIROS_TD030[@]}"; then
  falha "teste ignorado num ficheiro do TD-030 (R02)."
fi
grep -qF '#[cfg(all(test, loom))]' "$SEQ" || falha "o módulo loom desapareceu de $SEQ (R01)."

set +e
cargo test -p led-pixel-engine --locked --no-fail-fast > "$OUT/std.txt" 2>&1
code=$?
set -e
if [ "$code" -ne 0 ]; then grep -aE '^test .* FAILED|panicked at' "$OUT/std.txt" | head -20; falha "suite std exit $code."; fi
for t in "${REQ_STD[@]}"; do
  grep -qE "^test ${t} \.\.\. ok$" "$OUT/std.txt" || falha "oráculo '$t' não executou como ok (removido, renomeado ou ignorado)."
done
if grep -aqE '^test .* \.\.\. ignored' "$OUT/std.txt"; then
  grep -aE '^test .* \.\.\. ignored' "$OUT/std.txt"; falha "há testes ignorados no led-pixel-engine."
fi
echo "A: ${#REQ_STD[@]} oráculos std executados como ok, 0 ignorados."

# ── B. loom executado de verdade ─────────────────────────────────────────────
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR_LOOM:-target/loom}"
FILTRO_LOOM=seqlock::loom_tests
loom() { RUSTFLAGS="--cfg loom" cargo test -p led-pixel-engine --lib --release --locked "$@"; }

set +e
loom "$FILTRO_LOOM" -- --list > "$OUT/loom-list.txt" 2>&1; code=$?
set -e
[ "$code" -eq 0 ] || { cat "$OUT/loom-list.txt"; falha "listagem loom exit $code."; }
esperado=$(grep -c ': test$' "$OUT/loom-list.txt" || true)
[ "$esperado" -ge "$PISO_LOOM" ] || falha "loom: $esperado testes listados, piso $PISO_LOOM (R03/R04, KB-012)."

set +e
loom "$FILTRO_LOOM" -- --nocapture > "$OUT/loom.txt" 2>&1; code=$?
set -e
cat "$OUT/loom.txt"
[ "$code" -eq 0 ] || falha "loom exit $code."
executado=$(grep -aoE 'test result: ok\. [0-9]+ passed' "$OUT/loom.txt" | grep -aoE '[0-9]+' | awk '{s+=$1} END {print s+0}')
[ "$executado" -eq "$esperado" ] || falha "loom: $executado executados, $esperado esperados."
for t in "${REQ_LOOM[@]}"; do grep -qE "^test ${t} \.\.\. ok$" "$OUT/loom.txt" || falha "loom: '$t' não executou como ok."; done
for k in 1 2; do
  n=$(grep -aoE "LOOM_O3 k=${k} err=[0-9]+" "$OUT/loom.txt" | grep -aoE '[0-9]+$' || true)
  [ -n "$n" ] && [ "$n" -ge 1 ] || falha "loom O3 k=$k sem Err explorado (não-vacuidade)."
done
echo "B: loom ${executado}/${esperado} (piso ${PISO_LOOM}), O3 não-vácuo."

# ── C. o gate sabe reprovar ─────────────────────────────────────────────────
if [ -n "${TD030_SEM_CANARIOS:-}" ]; then
  [ -z "${CI:-}" ] || falha "TD030_SEM_CANARIOS não é aceite na CI."
  echo "C: SALTADO (TD030_SEM_CANARIOS, só local) — o resultado NÃO é evidência de gate."; exit 0
fi
git diff --quiet -- "$CRATE/src" || falha "$CRATE/src tem alterações por commitar; os canários aplicam-se sobre o HEAD."
shopt -s nullglob
canarios=("$CANARIOS_DIR"/*.patch)
[ "${#canarios[@]}" -ge 7 ] || falha "${#canarios[@]} canários em $CANARIOS_DIR, piso 7."
reverter() { git checkout -q -- "$CRATE/src"; }
trap reverter EXIT
for p in "${canarios[@]}"; do
  id=$(basename "$p" .patch)
  git apply --check "$p" 2>/dev/null || falha "canário $id já não aplica ao código atual — atualizar o canário com o código."
  git apply "$p"
  # Cada canário tem de ser morto por CADA teste loom, isoladamente (medido: os 7 são mortos pelos 2).
  # Assim, esvaziar ou neutralizar UM dos testes loom faz um canário sobreviver e o gate reprova.
  for t in "${REQ_LOOM[@]}"; do
    log="$OUT/canario-$id-${t##*::}.txt"
    set +e
    loom "$t" -- --exact > "$log" 2>&1; code=$?
    set -e
    if grep -aqE '^error(\[E[0-9]+\])?:' "$log" && ! grep -aq 'test result: FAILED' "$log"; then
      reverter; falha "canário $id não compilou — não prova nada."
    fi
    grep -aqE "^test ${t} \.\.\. FAILED$" "$log" && [ "$code" -ne 0 ] && grep -aqE "panicked at $SEQ:" "$log" \
      || { reverter; falha "canário $id SOBREVIVEU a ${t##*::} (exit $code) — o loom deixou de detetar este erro de ordering."; }
    echo "C: canário $id morto por ${t##*::} ($(grep -aoE "panicked at $SEQ:[0-9]+" "$log" | head -1))."
  done
  reverter
done
trap - EXIT
echo "TD-030 gate: OK — std ${#REQ_STD[@]} oráculos, loom ${executado}/${esperado}, ${#canarios[@]} canários mortos."
