# TD-020 — A guarda de monotonia do `SharedClock` passa a ser atómica (`fetch_max`)
git-hash: 13d2f41
source_files: crates/led-hal/src/shared_clock.rs
required_test: concurrent_readers_never_see_rewind_during_correction
data: 2026-09-17

# NOTA DE MÉTODO — porque tudo aqui foi RE-EXECUTADO.
#
# As medições originais desta fatia viviam no scratchpad (`/private/tmp/...`), que foi
# recriado vazio quando a sessão reiniciou. Transcrevê-las a partir do registo da conversa
# daria números certos e evidência inverificável — que é exactamente o que este gate existe
# para impedir. Tudo abaixo foi corrido de novo, contra `13d2f41`, incluindo os dois
# controlos negativos.

## O defeito

`now_ms()` protegia a monotonia com **três** operações:

```rust
let prev = self.last_now.load(Ordering::Acquire);
let next = adjusted.max(prev);
self.last_now.store(next, Ordering::Release);
```

Entre o `load` e o `store` outra thread pode publicar um valor mais alto. O `store` desta
repõe o `prev` obsoleto e **`last_now` recua** — o leitor seguinte vê o relógio do show a
andar para trás. É uma perda de actualização clássica.

## A correcção

Uma operação read-modify-write atómica. O máximo é calculado **dentro** dela, portanto não
há janela onde uma actualização se perca:

```rust
let anterior = self.last_now.fetch_max(adjusted, Ordering::AcqRel);
anterior.max(adjusted)
```

Semântica idêntica à anterior — `fetch_max` devolve o valor **anterior**, logo o valor novo
(o que `now_ms` deve devolver) é `max(anterior, adjusted)`. **Assinatura pública inalterada**,
**semântica de correcção de offset inalterada**.

## O detector foi ENDURECIDO, não substituído

A versão anterior aplicava uma correcção de −500 ms **uma vez**, a meio do laço, e reprovava
em ~**1 de 30** execuções. Um verde não provava nada — o defeito era real e o gate deixava-o
passar 29 vezes em 30 (KB-012 na forma mais cara).

| | antes | depois |
|---|---|---|
| threads leitoras | 8 | 8 |
| iterações por thread | 5 000 | 20 000 |
| rondas | 1 | 20 |
| **leituras totais** | **40 000** | **3 200 000** |
| correcção | −500 ms, uma vez em `i==2500` | thread dedicada, alternância contínua |
| asserção | booleano (`ok`) | **magnitude** do maior recuo, impressa na falha |

Nenhuma asserção foi removida nem afrouxada.

## Resultados — RED / GREEN, 10 execuções cada

| condição | vermelho / 10 |
|---|---|
| detector endurecido, **código de 3 operações** | **10** |
| detector endurecido, **`fetch_max`** | **0** (verde 10/10) |

Mensagem verbatim de uma reprovação pré-correcção:

```
thread 'shared_clock::tests::concurrent_readers_never_see_rewind_during_correction'
panicked at crates/led-hal/src/shared_clock.rs:268:13:
assertion `left == right` failed: ronda 2: um leitor viu o relógio RECUAR 1 ms sob correcção
concorrente (8 leitoras × 20000 leituras). A guarda de monotonia não é atómica —
load/calcula/store perde actualizações (TD-020).
  left: 1
 right: 0

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 101 filtered out
```

## CONTROLO NEGATIVO — e é ele que impede o detector de ser teatro

Com **o defeito presente** e `AMPLITUDE_MS = 0` (a thread de correcção continua a girar, mas
o offset nunca muda):

```
CONTROLO (3-op + AMPLITUDE=0): RED=0 GREEN=10
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out
```

**0 de 10.** Sem a alternância do offset o detector não dispara, mesmo com o defeito lá.
Isto prova que o vermelho de 10/10 vem do **mecanismo** e não do volume de iterações — e que
a thread de correcção ganha o seu lugar por medição, não por decoração.

## O mecanismo, medido — e uma previsão minha que foi FALSIFICADA

A primeira redacção deste raciocínio previa que o recuo teria a magnitude de `AMPLITUDE_MS`.
**Falso: o recuo medido é de 1 ms**, a granularidade de `wall`.

O que a alternância faz não é criar magnitude — é criar a condição em que a escrita obsoleta
deixa de ser mascarada:

```
offset alto:  T1 calcula a1 = W+A ; carrega prev = W+A   (preemptada antes do store)
              T2 calcula a2 = W+1+A ; store ⇒ last_now = W+1+A ; devolve W+1+A
              T1 store ⇒ last_now RECUA para W+A
offset baixo: T2 calcula a2 = W+1 (NÃO domina) ; prev = W+A ⇒ devolve W+A
              ...que é 1 ms MENOS do que o W+1+A que ela própria já devolveu
```

Com offset **constante**, `adjusted` acompanha o `wall` e volta sempre a dominar `prev`: a
perda existe e é imediatamente encoberta. É por isso que o controlo `AMPLITUDE_MS = 0` fica
verde, e é por isso que o detector antigo era 1-em-30.

## Gates

```
cargo test -p led-hal --lib
  running 102 tests
  test shared_clock::tests::concurrent_readers_never_see_rewind_during_correction ... ok
  test result: ok. 102 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
  EXIT_PROCESSO=0          (lido do processo, sem pipe — KB-013)

cargo test --workspace --no-fail-fast
  103 binários · 1135 passed; 0 failed; 9 ignored
  EXIT_TEST=0              (lido do ficheiro, sem pipe)
  — idêntico à baseline de 54b9fa8: endurecer um teste existente não altera a contagem

cargo clippy --workspace --all-targets -- -D warnings
  EXIT_CLIPPY=0 · 0 warnings · 0 `#[allow]` introduzidos
```

## Isolamento

```
cargo tree -p led-hal --depth 1
  led-hal v0.1.0
  └── led-core v0.1.0
```

`led-hal` alcança **apenas** `led-core`. Zero ocorrências de `led-triple` ou
`led-pixel-engine` no grafo — o trabalho parqueado em `wip/d4-f1b-triple-leaf` (`0a3832c`)
não tem caminho até esta fatia, e portanto não pode ter causado nem mascarado nada aqui.

## NÃO MEDIDO — declarado em vez de arredondado

**A ausência de alocação é argumento estrutural, não medição.** Verificado:
`crates/led-hal/tests/no_alloc.rs` **não exercita o `SharedClock`** — nenhum gate deste
repositório mede alocação em `now_ms`. `fetch_max` não toca no heap, e em x86-64 baixa para
um laço CAS que continua atómico (a ausência de perda de actualização não depende da
lowering) — mas isso é raciocínio, e fica rotulado como tal.

**Nada foi validado em hardware.** O consumidor de produção que mais importa é
`crates/led-player/src/stream.rs:142` (`play_streaming_unverified`), o caminho de reprodução
do show; os outros são `cluster_sync.rs:77` e `net_time.rs:52`. O rig está offline.
