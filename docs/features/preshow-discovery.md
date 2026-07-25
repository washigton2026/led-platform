# Feature: Discovery pré-show (ArtPoll presence-check)

Subagente responsável: network-architect (+ product-architect na UX do CLI)

## Motivação
O footgun nº 1 do operador (RT-003, e a dor real do rig do usuário): mandar
frames para um controlador que está desligado, em WiFi morto ou na subnet
errada → **palco escuro, sem erro**. UDP é fire-and-forget: o player nunca
soube que ninguém recebeu. No rig real de 5 robôs isso é presente — um robô que
não ligou some silenciosamente. A revisão suprema classificou isto como
melhoria de ROI real no 1x (não escala prematura).

## Design
`led-protocols` (domínio network-architect) ganha discovery baseada em ArtPoll,
reusando o padrão de `poll_conflicts`:
- `presence(&[Ipv4Addr], &[ArtPollReply]) -> DiscoveryResult` — **lógica pura**:
  particiona os IPs esperados em `responded`/`missing`. Reply de IP fora do
  esperado é ignorado (não pode mascarar um ausente).
- `discover_controllers(&[Ipv4Addr], timeout) -> io::Result<DiscoveryResult>` —
  broadcast ArtPoll, coleta replies, chama `presence`.
Player: flags `--discover` (avisa) e `--require-all` (aborta se algum não
responde), rodadas ANTES do primeiro frame. Seam: nenhum tipo de `led-core`
tocado → sem evento SemVer.

## Implementação
- `crates/led-protocols/src/artnet.rs` (+`DiscoveryResult`, `presence`,
  `discover_controllers`)
- `crates/led-protocols/src/lib.rs` (+exports)
- `crates/led-player/src/main.rs` (+`--discover`/`--require-all`, bloco pré-show)

## Testes
4 novos em `led-protocols` (14 no módulo artnet):
- `presence_partitions_expected_into_responded_and_missing` — 5 esperados, 4
  respondem → o 5º (o rig real .160) fica em `missing`.
- `presence_all_present_when_every_expected_answers` — caminho feliz.
- `presence_empty_expected_is_trivially_present` — borda.

Teste negativo: `negative_control_rogue_reply_cannot_mask_a_missing_controller`
— um nó em .99 (errado) responde, mas o esperado .160 não; `missing` DEVE conter
.160. Se algum dia reportar `all_present`, o presence-check é inútil (anti KB-012).

Evidência de integração (CLI, rig real offline):
- `--require-all` contra .156 offline → **exit 1** com ABORT explícito.
- `--discover` sem alvo hw → avisa e segue (opt-in).
- run normal de simulador → inalterado.

## Rollback
Aditiva: remover as 3 funções de `artnet.rs`, os exports e o bloco de discovery
+ 2 flags de `main.rs` restaura o estado anterior. Nenhum contrato alterado;
sem flag, o comportamento do player é idêntico ao de antes.

## Evidência
```
$ ./target/debug/led-player robot_show.lumyx --artnet 192.168.2.156 --require-all
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ⚠ SEM resposta
ABORT: --require-all e 192.168.2.156 não respondeu ao ArtPoll (desligado? WiFi morto? subnet errada?)
exit=1

$ cargo test -p led-protocols artnet
test artnet::tests::negative_control_rogue_reply_cannot_mask_a_missing_controller ... ok
test artnet::tests::presence_partitions_expected_into_responded_and_missing ... ok
test result: ok. 14 passed; 0 failed
```
