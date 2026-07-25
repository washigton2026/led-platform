# LUMYX — Red Team findings ledger

Regra: nenhuma mudança crítica entra sem auditoria. Todo achado CRITICAL/HIGH
bloqueia até ter **mitigação com teste** (verificado por `scripts/lumyx_red_team.sh`)
ou aceite explícito com dono de risco nomeado.

Fonte: LUMYX-RED-TEAM (5 subagentes). Formato: Severidade · Como explorar ·
Mitigação · Evidência · Status.

---

## RT-001 — Verificação de assinatura confiava na chave embutida (CRITICAL) ✅ MITIGADO

- **Red team**: security ("como quebrar isso?")
- **Como explorar**: `verify_manifest` valida a assinatura contra a chave pública
  contida **no mesmo blob**. Um adversário adultera o manifest, re-assina com a
  própria chave e embute a própria pubkey — a verificação retorna `Ok`. A
  assinatura provava consistência interna, não autenticidade.
- **Prova de exploração**: `redteam_resigned_tamper_defeats_unpinned_verify`
  (passa, documentando o buraco) + demo CLI: atacante re-assina `robot_show.lumyx.sig`
  com chave `34b4…`; palco verifica com a chave do estúdio → **`SIG VERIFY FAILED`, exit 1**.
- **Mitigação**: `verify_manifest_pinned(signed, &trusted_key)` — rejeita
  (`UntrustedKey`) se a chave embutida ≠ a chave pré-confiada, que viaja
  out-of-band. Ligado no consumidor real: `led-player --verify-key <pubkey>`
  carrega o sidecar `.sig`, confere que cobre o show e verifica com chave fixada.
- **Evidência**: `signing.rs` (12 testes incl. `pinned_verify_rejects_resigned_tamper`
  negativo); `led-player/src/main.rs` caminho `--verify-key`; demo CLI acima.
- **Status**: MITIGADO 2026-07-12. `verify_manifest` permanece (uso local, doc
  ⚠️ explícita); o caminho de fronteira de confiança usa o pinned.

## RT-002 — Endpoint /metrics é single-thread (MEDIUM) 🟡 ACEITO

- **Red team**: reliability ("como derrubar isso?")
- **Como explorar**: `serve_metrics` aceita uma conexão por vez; um cliente lento
  (slowloris) que não fecha a conexão bloqueia scrapes subsequentes.
- **Mitigação/aceite**: o endpoint é interno (rede de show cabeada, não exposto à
  internet); read timeout na conexão limita o hold. Risco aceito para v1; dono:
  operador de rede (IGMP + rede isolada do show). Upgrade futuro: thread por
  conexão ou timeout de accept.
- **Evidência**: `led-hal/src/prometheus.rs` (loop de accept sequencial).
- **Status**: ACEITO (MEDIUM, rede isolada).

## RT-003 — Operador pode acionar controlador ausente sem aviso (MEDIUM) ✅ MITIGADO

- **Red team**: product ("o operador consegue errar?")
- **Como explorar**: controlador desligado / WiFi morto / subnet errada → o
  player mandava frames para o vazio, palco escuro, sem erro (UDP fire-and-forget).
- **Mitigação**: **discovery pré-show** (`led-player --discover` / `--require-all`)
  faz ArtPoll broadcast antes do 1º frame e reporta quem não respondeu. Um WLED no
  IP certo responde ArtPoll independente do protocolo de saída, cobrindo também o
  caso "IP certo mas configuração errada". `--require-all` aborta (exit 1) se algum
  controlador esperado silenciar. `led-protocols::{presence, discover_controllers}`.
- **Evidência**: `artnet.rs` 4 testes (incl. negativo `negative_control_rogue_reply…`);
  CLI provado contra o rig real offline → `⚠ SEM resposta` + ABORT exit 1.
- **Status**: MITIGADO 2026-07-12. GAP residual menor: `--first-universe` errado
  (numérico) ainda não é detectável sem receber pixels de volta — rastreado, baixo risco.

## RT-004 — Classes de falha ainda não simuladas (INFO) 📋 GAP

- **Red team**: chaos ("qual falha ainda não simulamos?")
- **Simulado hoje**: perda de pacote (in-process + fio), latência, crash,
  failover, drift de relógio, hot-join.
- **Ainda NÃO simulado**: entrega fora de ordem, pacotes duplicados, frame
  rasgado no fio (universo parcial), relógio andando para trás durante o show,
  ArtPoll conflitante mid-show, partição de rede assimétrica.
- **Status**: GAP rastreado. Próximo alvo de maior valor: relógio para trás
  (o `SharedClock` já é monotônico por construção — candidato a teste que prova).

## RT-005 — CompiledLayout reconstruído 2× nos exemplos (LOW) 🟢 COSMÉTICO

- **Red team**: architecture ("onde está o acoplamento oculto?")
- **Achado**: `robot_show.rs` chama `CompiledLayout::compile(&assigns)` duas vezes
  (uma para device_universes, outra para o Hal) — estado duplicado, não bug.
- **Status**: COSMÉTICO. Não é acoplamento de produção (só exemplos).

---

## Histórico de execuções (append-only pelo harness)

## Auditoria 2026-07-12T06:58Z
- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned
- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards

## Auditoria 2026-07-12T07:02Z
- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned
- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards

## Auditoria 2026-07-12T07:07Z
- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned
- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards

## Auditoria 2026-07-12T22:05Z
- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned
- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards

## Auditoria 2026-07-13T08:15Z
- HIGH mitigado: verify_manifest confia na chave embutida → verify_manifest_pinned
- OPEN: ArtPoll pré-show, --verify-key no player, chaos reorder/dup/clock-backwards

