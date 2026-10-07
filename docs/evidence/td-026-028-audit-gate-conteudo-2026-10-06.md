# TD-026 + TD-028 — o detector de stale do audit_gate passa a julgar CONTEÚDO, sem git
git-hash: 89c7603
watched: scripts/audit_gate.py sha256:f18afb2cd38233dcc0f039181dc90809b25394e97636d5d2ba673e4613e07fa1
watched: tests/test_audit_gate.py sha256:fabd50f348eff6139a66790dc66dfba773396dfae5f8133c2e0b3b9ebe4d03b5
data: 2026-10-06

# PORQUE EXISTE
#
# TD-028: o stale era «há commits em git log <hash>..HEAD», e um merge que não muda o conteúdo
# tornava a evidência stale (falso-vermelho). TD-026: o returncode do git log era ignorado, e
# num clone raso o hash não existe — o stale passava a verde em silêncio (falso-verde).
# R4.1 substitui o detector: cada evidência fixa o sha256 do CONTEÚDO de cada ficheiro vigiado
# (linhas watched:), e o gate compara-o com o conteúdo do workspace (no pre-commit, a worktree
# do commit candidato). Sem git; ficheiro vigiado ilegível/inexistente/diretório é Critical.
# Após 4 rondas do falsificador, o required_test só conta dentro da REGIÃO DE PROVA declarada
# (lista positiva) — a secção 2, contra o gate antigo, fica fora por construção.
# git-hash = commit anterior (informativo). Regenerada por regenerar-evidencia-td026-028.sh.

## 1. Gate novo — python3 tests/test_audit_gate.py (run real, no repo), exit 0
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
test test_gate_accepts_good_ledger ... ok
test_audit_gate: 34 passed; 0 failed
--- fim da prova ---

## 2. Os MESMOS testes contra o audit_gate.py ANTIGO (git show f318c97:scripts/audit_gate.py), exit 1
# 19 FAILED, entre eles os dois que estes TD nomeiam (TD-028: história sem mudança de
# conteúdo; TD-026: clone raso).
NEG: test test_r41_vigiado_alterado_sem_evidencia_nova_e_stale ... FAILED
NEG: test test_r41_vigiado_inexistente_e_critical_nao_verificavel ... FAILED
NEG: test test_r41_source_file_sem_watched_e_critical ... FAILED
NEG: test test_r41_ci_yml_e_declaravel ... FAILED
NEG: test test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde ... FAILED
NEG: test test_r41_td026_clone_raso_igual_a_completo ... FAILED
NEG: test test_r41_vigiado_ilegivel_ou_diretorio_e_critical ... FAILED
NEG: test test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado ... FAILED
NEG: test test_r41_pins_nao_confiaveis_sao_critical ... FAILED
NEG: test test_r41_ronda2_symlink_para_fora_do_workspace_e_critical ... FAILED
NEG: test test_r41_sem_regiao_de_prova_e_critical ... FAILED
NEG: test test_r41_duas_regioes_ou_marcadores_trocados_e_critical ... FAILED
NEG: test test_r41_ok_fora_da_prova_nao_conta ... FAILED
NEG: test test_r41_failed_do_teste_dentro_da_prova_e_vermelho ... FAILED
NEG: test test_r41_resumo_n_maior_que_zero_dentro_da_prova ... FAILED
NEG: test test_r41_linhas_que_nao_sao_resultado_nao_provam ... FAILED
NEG: test test_r41_harness_estrito ... FAILED
NEG: test test_r41_watched_dentro_de_html_multilinha_nao_fixa ... FAILED
NEG: test test_r41_ronda4_exigencias_da_regiao_cada_uma_com_a_sua_assercao ... FAILED
NEG: test_audit_gate: 15 passed; 19 failed
