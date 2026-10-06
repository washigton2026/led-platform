# TD-026 + TD-028 — o detector de stale do audit_gate passa a julgar CONTEÚDO, sem git
git-hash: f318c97
watched: scripts/audit_gate.py sha256:34adeb7e7c2275a37d521143c3355ada3e312dce13716fbca2358c6a5f926998
watched: tests/test_audit_gate.py sha256:4ef05bc3d2d00ef1d26dd33ce90c451c427c4834cccac85845797fa774306caa
data: 2026-10-06

# PORQUE EXISTE
#
# TD-028: o stale era «há commits em git log <hash>..HEAD», e um merge que não muda o conteúdo
# tornava a evidência stale (falso-vermelho). TD-026: o returncode do git log era ignorado, e
# num clone raso o hash não existe — o stale passava a verde em silêncio (falso-verde).
# R4.1 substitui o detector: cada evidência fixa o sha256 do CONTEÚDO de cada ficheiro vigiado
# (linhas watched:), e o gate compara-o com o conteúdo do workspace (no pre-commit, a worktree
# do commit candidato). Sem git; ficheiro vigiado ilegível/inexistente é Critical.

## 1. Gate novo — python3 tests/test_audit_gate.py (base f318c97 + R4.1), exit 0
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
test test_gate_accepts_good_ledger ... ok
test_audit_gate: 23 passed; 0 failed

## 2. Controlo negativo — os MESMOS testes contra o audit_gate.py ANTIGO (git show f318c97:scripts/audit_gate.py), exit 1
# Os 8 testes do R4.1 que exigem o comportamento novo reprovam com o gate antigo, entre eles
# os dois que estes TD nomeiam (TD-028: história sem mudança de conteúdo; TD-026: clone raso).
NEG: test test_r41_required_test_so_em_comentario_e_vermelho ... FAILED
NEG: test test_r41_required_test_so_failed_e_vermelho ... FAILED
NEG: test test_r41_vigiado_alterado_sem_evidencia_nova_e_stale ... FAILED
NEG: test test_r41_vigiado_inexistente_e_critical_nao_verificavel ... FAILED
NEG: test test_r41_source_file_sem_watched_e_critical ... FAILED
NEG: test test_r41_ci_yml_e_declaravel ... FAILED
NEG: test test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde ... FAILED
NEG: test test_r41_td026_clone_raso_igual_a_completo ... FAILED
NEG: test_audit_gate: 15 passed; 8 failed
