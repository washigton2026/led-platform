# TD-037 — o audit_gate passa a reprovar td_id duplicado (regra 7)
git-hash: b4c3b7a
watched: scripts/audit_gate.py sha256:a7095570c8d75f753796eb726778767bd2815263cb1cd59127967085c9ecaa53
watched: tests/test_audit_gate.py sha256:4aeded2cc850da4b291e870083fbfb0318ad2590336b660d5f0bd83d0170f4f6
data: 2026-11-26

# PORQUE EXISTE
#
# O merge 8e64f68 (origin/main com o #64 em fix/td-035-fanout-tempo) deixou dois blocos td_id TD-035
# no ledger e o hook do pre-commit deu «34 TD entries … 0 Critical». A regra 7 (Gate.duplicados)
# corre sobre todas as entradas antes das verificações por TD e torna Critical cada td_id com mais de
# uma entrada, seja qual for o status. A fixture tests/fixtures/ledger_td_id_duplicado.md reproduz a
# forma real (open + pending-verification com o mesmo id); cada entrada sozinha passa.
# Regenerada por ~/lumyx-evidence/2026-11-26-td037/regenerar-evidencias-gate.py.

## 1. Gate novo — python3 -I tests/test_audit_gate.py (run real, no repo), exit 0
--- prova ---
test test_extract_passed_count ... ok
test test_evidence_git_hash ... ok
test test_gate_rejects_bad_ledger_a_no_evidence ... ok
test test_gate_rejects_bad_ledger_b_zero_passed ... ok
test test_gate_rejects_bad_ledger_c_empty_negative_control ... ok
test test_gate_rejects_full_bad_ledger_exit_1 ... ok
test test_pending_verification_within_deadline_is_ok ... ok
test test_pending_verification_past_deadline_is_critical ... ok
test test_pending_verification_malformed_review_by_is_critical ... ok
test test_open_and_wontfix_are_visible_not_ok ... ok
test test_r41_evidencia_valida_passa ... ok
test test_r41_vigiado_alterado_sem_evidencia_nova_e_stale ... ok
test test_r41_alterado_com_evidencia_regenerada_no_mesmo_commit_e_verde ... ok
test test_r41_vigiado_inexistente_e_critical_nao_verificavel ... ok
test test_r41_source_file_sem_watched_e_critical ... ok
test test_r41_ci_yml_e_declaravel ... ok
test test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde ... ok
test test_r41_td026_clone_raso_igual_a_completo ... ok
test test_r41_vigiado_ilegivel_ou_diretorio_e_critical ... ok
test test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado ... ok
test test_r41_pins_nao_confiaveis_sao_critical ... ok
test test_r41_ronda2_symlink_para_fora_do_workspace_e_critical ... ok
test test_r41_sem_regiao_de_prova_e_critical ... ok
test test_r41_duas_regioes_ou_marcadores_trocados_e_critical ... ok
test test_r41_ok_fora_da_prova_nao_conta ... ok
test test_r41_failed_do_teste_dentro_da_prova_e_vermelho ... ok
test test_r41_resumo_n_maior_que_zero_dentro_da_prova ... ok
test test_r41_linhas_que_nao_sao_resultado_nao_provam ... ok
test test_r41_formatos_estruturados_aceites ... ok
test test_r41_harness_estrito ... ok
test test_r41_nome_com_metacaracteres_e_literal ... ok
test test_r41_watched_dentro_de_html_multilinha_nao_fixa ... ok
test test_r41_ronda4_exigencias_da_regiao_cada_uma_com_a_sua_assercao ... ok
test test_td_id_duplicado_e_critical ... ok
test test_td_id_distintos_nao_disparam_a_regra ... ok
test test_gate_accepts_good_ledger ... ok
test_audit_gate: 36 passed; 0 failed
--- fim da prova ---

## 2. Controlo negativo — os mesmos testes contra o audit_gate.py de b4c3b7a (cópia por git archive), exit 1
# test_td_id_duplicado_e_critical reprova (o gate antigo aceita o duplicado); o controlo
# test_td_id_distintos_nao_disparam_a_regra passa nos dois gates (a regra só reage à colisão).
NEG: test test_td_id_duplicado_e_critical ... FAILED
NEG: test test_td_id_distintos_nao_disparam_a_regra ... ok
NEG: test_audit_gate: 34 passed; 2 failed
NEG: GATE BROKEN: td_id duplicado passou (exit 0): []
