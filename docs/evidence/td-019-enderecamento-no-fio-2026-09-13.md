# TD-019 — endereçamento Art-Net/sACN pedido ao dono, provado no fio
# git-hash: 09c135ed0120851b386029e82ec2324a999c97b5
# Generated: 2026-09-13
# source_files: crates/led-daemon-bin/src/output.rs crates/led-daemon-bin/tests/wled_driver.rs crates/led-hardware-profile/src/compile.rs

## Escopo — o que este artefacto prova, e o que NAO prova

PROVA: o daemon honra `pixels_per_universe` e `ColorFormat` do profile nos bytes que
SAEM, para o preset `generic-sk6812-rgbw-sacn` (ppu 128, Rgbw(Grb, MinSubtract)).
Criterio A do closure_criteria do TD-019.

NAO PROVA: (a) que um controlador real aceita estes bytes — o rig esta offline e nenhum
no RGBW+sACN foi alguma vez observado; (b) o criterio D, que exige o gate da GS4.4 cobrir
led-player/src/lib.rs. Por isso o TD-019 permanece `open`.

## Teste discriminante (criterio A)

required_test: o_sacn_rgbw_honra_os_128_px_por_universo_e_os_quatro_canais
  crates/led-daemon-bin/tests/wled_driver.rs

```
$ cargo test -p led-daemon-bin --test wled_driver
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.77s
```

## Facto medido que dita a forma da assercao

Os tres datagramas medem 638 B (126 de cabecalho + 512 canais PREENCHIDOS) com 4 canais
E com 3 — medido nas duas condicoes. Logo `d.len()` e CEGO a um colapso de formato, e
qualquer assercao de comprimento (exacta ou por limite) nao prova nada sobre canais.
O que discrimina e o PERIODO do padrao de pixel:

  Rgbw(Grb, MinSubtract) : [50, 150, 0, 50]   (W=min=50, residuo (150,50,0), GRB+W)
  Rgb(Grb) colapsado     : [100, 200, 50]

Primeiro byte de canal no datagrama sACN: 126 (medido).

## Controlos negativos — ambos reprovam

AVISO: o criterio B do ledger esta STALE. Manda repor `PX_PER_UNIVERSE = 170` no
led-player, constante que o daemon deixou de usar no C2b — mutar essa constante NAO pode
reprovar este teste. Os controlos validos sao os dois abaixo, no ponto que o daemon usa.

### M1 — output.rs:611 `pixels_per_universe: 170` (ignora o profile)
```
test o_sacn_rgbw_honra_os_128_px_por_universo_e_os_quatro_canais ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.00s
```

### M2 — output.rs:615 `color: ColorFormat::Rgb(self.rgb_order())` (colapsa RGBW)
```
test o_sacn_rgbw_honra_os_128_px_por_universo_e_os_quatro_canais ... FAILED
assertion `left == right` failed: datagrama 0, pixel 0: esperava os 4 canais [50, 150, 0, 50], veio [100, 200, 50, 100]
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.25s
```

negative_control: M2 e a prova de que o defeito existia. ANTES do commit 09c135e a mesma
mutacao passava os 18 testes — a assercao era `d.len() >= 126`, cega ao formato. O teste
afirmava no nome uma propriedade que o corpo nao verificava (KB-012).

## Restauro
Confirmado, nao assumido: `git status --porcelain` vazio apos cada mutacao.
