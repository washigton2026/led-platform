# TD-022 — O `writeln!` interrompido devolve `BrokenPipe` em Linux (medido), e as três falsificações
git-hash: 57cf21d
source_files: crates/led-console-bin/tests/ipc_contra_o_daemon.rs
required_test: o_daemon_recusa_a_linha_longa_por_si_proprio
data: 2026-09-26

# O QUE ESTE ARTEFACTO FECHA
#
# O `pending_gate` do TD-022 (:97–100) não perguntava se o job ubuntu fica verde: perguntava
# QUE ERRNO devolve em Linux o `writeln!` interrompido pelo fecho do daemon. Um verde não
# responde — com buffers de socket grandes a escrita pode acabar antes do fecho e o ramo
# `if let Err` nunca corre. A resposta veio de uma SONDA DETERMINÍSTICA em Linux, numa branch
# descartável que nunca é mergeada, e as três falsificações do `falsification_required` foram
# RE-EXECUTADAS hoje (nenhum resultado transcrito de sessões anteriores — não havia registo).

## 1. Linux — a sonda (resposta ao `pending_gate`)

Branch descartável `probe/td-022-linux` @ `478681b` (= `57cf21d` + a sonda; **nunca para merge**,
usa `thread::sleep` — classe TD-003). Workflow `probe-td-022`, **só `ubuntu-latest`**, run
**`36245228354`**, lido **no log** (não no ✔️). O `ci.yml` não disparou nesse push.

A sonda escreve a linha gigante em pedaços de 4 KiB até passar `MAX_LINE` (sem `\n`), dorme
500 ms (o daemon lê o excesso, escreve a recusa, faz `flush` e fecha — `server.rs:264-274`), e
continua a escrever até o `write` falhar. **Regista** o erro em vez de o afirmar.

```
=== passagem 1: exit 0
SONDA-TD022 os=linux fase=fase3 kind=BrokenPipe raw_os_error=Some(32) display=Broken pipe (os error 32) escritos_antes=73728 escritos_depois=0 recusa_lida=Ok(93) recusa="{\"v\":1,\"id\":null,\"ok\":false,\"error\":{\"code\":\"bad_request\",\"detail\":\"linha demasiado longa\"}}\n"
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.50s
=== passagem 2: exit 0
SONDA-TD022 os=linux fase=fase3 kind=BrokenPipe raw_os_error=Some(32) ... recusa_lida=Ok(93) ...
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.50s
=== passagem 3: exit 0
SONDA-TD022 os=linux fase=fase3 kind=BrokenPipe raw_os_error=Some(32) ... recusa_lida=Ok(93) ...
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.50s
```

**Veredito do `pending_gate`: `BrokenPipe` (errno 32), 3/3.** O mesmo valor que em macOS
(medido localmente com a mesma sonda: `kind=BrokenPipe raw_os_error=Some(32)`). A recusa já está
no buffer de recepção quando o EPIPE chega (`recusa_lida=Ok(93)`), nas duas plataformas — a
premissa de ordenação do TD-022 confirma-se em Linux. **O conjunto aceite (só `BrokenPipe`) não
foi alargado.**

**Controlo negativo da sonda** (macOS, local): com a fase 3 anulada (`for _ in 0..0`) a sonda
**reprova** — `SONDA-TD022 os=macos INTERRUPCAO_NAO_OBSERVADA`, `panicked at …:287:5`, exit 101,
0 `error[E` (vermelho de teste, não de compilação). Revertida: verde, exit 0. A sonda não passa
sem medir.

## 2. As três falsificações — re-executadas hoje sobre `57cf21d` (macOS local)

Comando por execução (exit lido do processo, sem pipe — KB-013):
`cargo test -p led-console-bin --test ipc_contra_o_daemon --locked -- --exact o_daemon_recusa_a_linha_longa_por_si_proprio`

Mutações temporárias, **restauradas** no fim (`git diff --quiet` → 0). «Forçada» significa: antes
do `writeln!`, enviar exactamente `MAX_BODY + 1` bytes sem `\n` (o daemon consome-os **todos**
antes de fechar, logo essa escrita nunca apanha EPIPE) e dormir 500 ms — a interrupção do
`writeln!` deixa de depender do escalonador.

| condição | mutação | exit ×3 | `error[E` | `panicked` | leitura |
|---|---|---|---|---|---|
| **C0** | nenhuma (código actual) | `0 0 0` | 0 | 0 | `1 passed` ×3 |
| **F1c** controlo | interrupção **forçada**, código tolerante actual | `0 0 0` | 0 | 0 | `1 passed` ×3 — a correcção sobrevive a uma interrupção **garantida** |
| **F1** | interrupção forçada **+ `unwrap()` cru reposto** | `101 101 101` | 0 | 1 | vermelho **com `BrokenPipe`** |
| **F2** | o `writeln!` substituído por um erro `ConnectionReset` | `101 101 101` | 0 | 1 | erro não-EPIPE **continua a reprovar** |
| **F3** | daemon (`server.rs`) deixa de recusar a linha longa (`if false && n > MAX_LINE …`) | `101 101 101` | 0 | 1 | a asserção de recusa **discrimina** |

```
C0  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
F1c test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.53s

F1  panicked at crates/led-console-bin/tests/ipc_contra_o_daemon.rs:191:82:
    called `Result::unwrap()` on an `Err` value: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out

F2  panicked at crates/led-console-bin/tests/ipc_contra_o_daemon.rs:191:9:
    assertion `left == right` failed: so o fecho do daemon e esperado nesta escrita, e este erro nao e um: Kind(ConnectionReset)
      left: ConnectionReset
     right: BrokenPipe
    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out

F3  panicked at crates/led-console-bin/tests/ipc_contra_o_daemon.rs:206:5:
    {"v":1,"id":null,"ok":false,"error":{"code":"bad_request","detail":"JSON inválido na posição 65537: string não terminada"}}
    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out
```

(Os números de linha são do ficheiro **mutado**; no ficheiro real a escrita tolerante está em
`:189` e a asserção `"demasiado longa"` em `:206`.)

**Achado da F3, registado:** o daemon mutado responde **ainda com `bad_request`** (JSON truncado),
por isso a asserção `bad_request` **sozinha não discriminaria**. Quem apanha o daemon que aceita a
linha é a asserção `"demasiado longa"` (`:206`). As duas ficam; a segunda é a que tem dentes.

## 3. O que isto NÃO mede

- **A corrida original sob carga de workspace** não foi reproduzida em Linux — foi **forçada**
  (sonda e F1). O que se afirma é o errno da escrita interrompida, não a frequência da corrida.
- **Tempos**: os `finished in` são informativos (load 1-min ~1,5 na corrida local).
- **Nada foi validado em hardware** — o TD-022 é de teste, não de fio.
