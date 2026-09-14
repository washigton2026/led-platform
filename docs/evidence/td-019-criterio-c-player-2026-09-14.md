# TD-019 critério C — o `led-player` está preso ao dono do endereçamento

# git-hash: f080b00b79b7a8a7d99605e3a602e337c7884fba
# Generated: 2026-09-14
# source_files: crates/led-player/tests/profile_no_fio.rs crates/led-player/src/main.rs crates/led-hardware-profile/src/compile.rs

## Escopo — o que prova, e o que NAO prova

PROVA: o BINARIO `led-player`, corrido com `--profile esp32-devkit-wled-artnet --artnet`,
poe no fio o `ColorFormat` e a fronteira de universo que o `led-hardware-profile` dita — e
nao o fallback escrito a mao (`linear_assignments`, RGB, 170, calibracao identidade).
Criterio C do closure_criteria do TD-019.

NAO PROVA: (a) hardware — o rig esta offline, isto mede o que SAI para um socket de
loopback; (b) o `pixels_per_universe`, que NAO e discriminado (ver facto 3 abaixo);
(c) o caminho `--ddp`, que nao consome o layout (facto 1). E NAO e a comparacao literal
player-vs-daemon do texto do criterio C: o que esta provado e que AMBOS estao presos ao
MESMO dono — o daemon pela mutacao cruzada do C2a + `wled_driver`, o player por este teste.
O criterio D continua por cumprir, logo o TD-019 permanece `open`.

## Teste discriminante

required_test: o_player_com_profile_poe_no_fio_o_formato_do_profile_e_nao_o_do_fallback
  crates/led-player/tests/profile_no_fio.rs

```
$ cargo test -p led-player --test profile_no_fio
test o_player_com_profile_poe_no_fio_o_formato_do_profile_e_nao_o_do_fallback ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.29s
```

E o primeiro teste do crate fora do `lib.rs`/`stream.rs`: corre o binario via
`CARGO_BIN_EXE_led-player` contra um socket UDP efemero, com o `.lumyx` gerado no proprio
teste. ZERO linhas de producao.

## Os quatro factos medidos que ditaram o desenho

1. O arm DDP do `main.rs` usa SO `profile_color` e NAO consome o `CompiledLayout` — o DDP
   contorna o HAL por desenho (ADR-0003). Logo o enderecamento so e observavel no arm
   `--artnet`, onde `Hal::new(layout, ...)` o consome.
2. O player declara `protocols: &[ArtNet, Ddp]`. sACN REPROVA a validacao ali, portanto o
   preset RGBW/sACN usado no teste do daemon e inutilizavel neste lado.
3. TODOS os presets Art-Net do catalogo declaram `pixels_per_universe: 170` — exactamente a
   constante do fallback. Nesse eixo, honrar o profile e ignora-lo sao INDISTINGUIVEIS, e
   uma assercao sobre ele seria falso-verde. O que discrimina e ordem + gamma. Para o pixel
   logico (200,100,50), os tres bytes no fio:
     profile honrado (GRB + gamma 2.2) : [33, 149, 7]     (= [lut(g), lut(r), lut(b)])
     fallback (RGB, sem calibracao)    : [200, 100, 50]
   Os TRES bytes diferem — nao ha sobreposicao parcial que deixe a assercao ambigua.
   `lut[i] = ((i/255)^2.2 * 1.0 * 255 + 0.5) as u8` (calibration.rs:57-58). Os tres bytes
   foram PREVISTOS da formula e confirmados pela execucao — dois calculos independentes.
4. O Art-Net TAMBEM preenche o universo ate 512 canais. Previ `170*3 = 510`; vieram 512, e
   o teste reprovou nessa premissa antes de chegar a cor. Mesma familia do padding sACN:
   COMPRIMENTO E CEGO. O que prova e o PERIODO do padrao mais a FRONTEIRA do preenchimento
   (170 px no universo 1, 130 no 2, resto a zero).

## Controlos negativos — ambos reprovam

### R1 — `main.rs` valida o profile e DESCARTA o layout dele (cai no fallback)
```
test o_player_com_profile_poe_no_fio_o_formato_do_profile_e_nao_o_do_fallback ... FAILED
assertion `left == right` failed: datagrama 0, universo 1, pixel 0: esperava [33, 149, 7] (GRB do profile + gamma 2.2), veio [149, 33, 7]. Em RGB sem calibração sairia [200, 100, 50] — o fallback escrito à mão, que é o TD-019.
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
```
Isola a propriedade certa: so o layout foi descartado, o gamma continuou aplicado — logo o
que reprovou foi o ENDERECAMENTO/FORMATO, nao a calibracao.

### R2 — o DONO (`compile.rs`) ignora o `ColorFormat`
```
assertion `left == right` failed: datagrama 0, universo 1, pixel 0: esperava [33, 149, 7] (GRB do profile + gamma 2.2), veio [149, 33, 7]. Em RGB sem calibração sairia [200, 100, 50] — o fallback escrito à mão, que é o TD-019.
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.29s
```
A mensagem e IDENTICA a do R1, e isso NAO e copia: as duas mutacoes colapsam GRB->RGB em
sitios diferentes (R1 descarta o layout no consumidor; R2 corrompe o formato no produtor) e
o gamma continua aplicado nas duas, logo o observavel no fio e o mesmo `[149, 33, 7]`. O que
as distingue e a CAUSA, nao o sintoma — e e por isso que sao precisas as duas: o R1 prova
que o player LE o dono, o R2 prova que o player esta preso AO dono.

negative_control: R2 E O FECHO DO CRITERIO C. E a mesma classe de mutacao cruzada que,
ANTES deste teste, deixava o `led-player` em `18 passed; 0 failed` enquanto reprovava o
`led-daemon-bin`. O player passou a divergir => reprovar.

## Gates

```
$ cargo test -p led-player
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.30s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
$ cargo clippy -p led-player --all-targets -- -D warnings
exit 0
$ git diff --check
exit 0
```
19 passed, 0 failed, 0 ignored. Aritmetica: +1 teste e +1 alvo contra os 18 medidos antes.
Exits lidos SEM pipe (KB-013).

## Restauro
Confirmado, nao assumido: `git diff --stat` VAZIO em `main.rs` e em `compile.rs` apos cada
mutacao. Restaurado de copia em scratchpad, NUNCA por `git checkout` — o teste estava por
commitar e um checkout tê-lo-ia apagado.
