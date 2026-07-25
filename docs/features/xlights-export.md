# Feature: Exportador xLights (RigPlan → xlights_rgbeffects.xml)

Subagente responsável: product-architect (+ rust-architect no seam)

## Motivação
O rig real do usuário vive no xLights (5 robôs, 6.200 px). O LUMYX já importa
(com gate de conflito e auto-fix); faltava a **volta**: um rig criado pelo
`RigBuilder` (livre de conflito por construção) precisa abrir no xLights para
o usuário continuar usando o preview/sequencer que já conhece durante a
migração. Sem export, a migração é um beco sem saída de mão única.

## Design
Novo módulo `led-xlights::export`:
- `export_rgbeffects(&[XModel], &[XGroup]) -> String` — emite XML compatível
  (mesmo dialeto que `parse_rgbeffects` lê), com entity-escaping nos atributos.
- `rig_to_xmodels(&RigPlan, &[&str]) -> (Vec<XModel>, Vec<XGroup>)` — converte
  o plano (device/universo/canal 0-based) para o endereçamento xLights
  (`!controller:canal-absoluto-1-based`, 510 ch/universo) e agrupa por
  instância (`robô 1/…` → grupo `robô 1`).
Seam: `led-xlights` ganha dependência de `led-layout` (justificada no
Cargo.toml — ponte bidirecional). `led-core` intocado → sem evento SemVer.

## Implementação
- `crates/led-xlights/src/export.rs` (novo, ~200 linhas + testes)
- `crates/led-xlights/src/lib.rs` (+`pub mod export;`)
- `crates/led-xlights/Cargo.toml` (+led-layout com comentário)

## Testes
4 novos (26 total no crate):
- `export_reimport_roundtrip_preserves_every_field` — cada campo sobrevive
  export→parse (nome, controller, canal, px, string type, world, X2/Y2/Z2, grupos).
- `exported_rig_passes_the_import_gate` — nunca emitimos o que recusaríamos:
  0 conflitos, todos os pixels mapeados, canal absoluto exato (`!robô led 1:61`).
- `special_characters_survive_the_roundtrip` — `R&B "x" <robô>` escapa e volta;
  sem double-escaping.

Teste negativo: `negative_control_tampered_export_is_caught_by_the_gate` —
corrompe o XML exportado duplicando um StartChannel (o defeito real do projeto
do usuário) e exige que `find_channel_conflicts` acuse. Se este teste passar
com 0 conflitos, o gate regrediu (anti KB-012).

## Rollback
Feature é aditiva: remover `export.rs`, a linha `pub mod export;` e a
dependência `led-layout` do Cargo.toml restaura o estado anterior por completo.
Nenhum contrato ou código existente alterado.

## Evidência
```
$ cargo test -p led-xlights
test export::tests::special_characters_survive_the_roundtrip ... ok
test export::tests::export_reimport_roundtrip_preserves_every_field ... ok
test export::tests::exported_rig_passes_the_import_gate ... ok
test export::tests::negative_control_tampered_export_is_caught_by_the_gate ... ok
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
