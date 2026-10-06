# TD-026 + TD-028 — o detector de stale do audit_gate passa a julgar CONTEÚDO, sem git
git-hash: 94b85ab
watched: scripts/audit_gate.py sha256:42b3a4dcd774a0692473fc98669c6f35c32ed3940d2b6f91145aff6fb3a6893b
watched: tests/test_audit_gate.py sha256:0fd271b6eefd249dede862857e0bb7f60f00c1b0d04e19fea14b5bc4767a435d
data: 2026-10-06

# PORQUE EXISTE
#
# TD-028: o stale era «há commits em git log <hash>..HEAD», e um merge que não muda o conteúdo
# tornava a evidência stale (falso-vermelho). TD-026: o returncode do git log era ignorado, e
# num clone raso o hash não existe — o stale passava a verde em silêncio (falso-verde).
# R4.1 substitui o detector: cada evidência fixa o sha256 do CONTEÚDO de cada ficheiro vigiado
# (linhas watched:), e o gate compara-o com o conteúdo do workspace (no pre-commit, a worktree
# do commit candidato). Sem git; ficheiro vigiado ilegível/inexistente/diretório é Critical.
# Regenerada depois do falsificador do R4.1 (linhas NEG e prosa deixam de provar required_test;
# pins duplicados/fora do workspace → Critical) — por isso o git-hash é o commit anterior, 94b85ab.

## 1. Gate novo — python3 tests/test_audit_gate.py (run real, no repo), exit 0
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
test test_r41_required_test_so_em_comentario_e_vermelho ... ok
test test_r41_required_test_so_failed_e_vermelho ... ok
test test_r41_n_zero_e_vermelho ... ok
test test_r41_formatos_estruturados_aceites ... ok
test test_r41_vigiado_alterado_sem_evidencia_nova_e_stale ... ok
test test_r41_alterado_com_evidencia_regenerada_no_mesmo_commit_e_verde ... ok
test test_r41_vigiado_inexistente_e_critical_nao_verificavel ... ok
test test_r41_source_file_sem_watched_e_critical ... ok
test test_r41_ci_yml_e_declaravel ... ok
test test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde ... ok
test test_r41_td026_clone_raso_igual_a_completo ... ok
test test_r41_neg_ok_com_real_failed_e_vermelho ... ok
test test_r41_so_prefixo_de_log_ci_e_aceite ... ok
test test_r41_harness_exige_n_maior_que_zero_e_zero_falhas_sem_comentario ... ok
test test_r41_nome_com_metacaracteres_e_literal ... ok
test test_r41_vigiado_ilegivel_ou_diretorio_e_critical ... ok
test test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado ... ok
test test_r41_pins_nao_confiaveis_sao_critical ... ok
test test_gate_accepts_good_ledger ... ok
test_audit_gate: 30 passed; 0 failed

## 2. Controlo negativo — os MESMOS testes contra o audit_gate.py ANTIGO (git show f318c97:scripts/audit_gate.py), exit 1
# 14 FAILED, entre eles os dois que estes TD nomeiam (TD-028: história sem mudança de conteúdo;
# TD-026: clone raso). Contra 94b85ab (a 1.ª versão do R4.1) reprovam 3 — os do falsificador.
NEG: test test_r41_required_test_so_em_comentario_e_vermelho ... FAILED
NEG: test test_r41_required_test_so_failed_e_vermelho ... FAILED
NEG: test test_r41_vigiado_alterado_sem_evidencia_nova_e_stale ... FAILED
NEG: test test_r41_vigiado_inexistente_e_critical_nao_verificavel ... FAILED
NEG: test test_r41_source_file_sem_watched_e_critical ... FAILED
NEG: test test_r41_ci_yml_e_declaravel ... FAILED
NEG: test test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde ... FAILED
NEG: test test_r41_td026_clone_raso_igual_a_completo ... FAILED
NEG: test test_r41_neg_ok_com_real_failed_e_vermelho ... FAILED
NEG: test test_r41_so_prefixo_de_log_ci_e_aceite ... FAILED
NEG: test test_r41_harness_exige_n_maior_que_zero_e_zero_falhas_sem_comentario ... FAILED
NEG: test test_r41_vigiado_ilegivel_ou_diretorio_e_critical ... FAILED
NEG: test test_r41_vigiado_extra_alem_dos_source_files_tambem_e_julgado ... FAILED
NEG: test test_r41_pins_nao_confiaveis_sao_critical ... FAILED
NEG: test_audit_gate: 16 passed; 14 failed
