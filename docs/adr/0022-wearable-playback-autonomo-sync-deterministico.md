# ADR-0022 — Traje de LED: playback autônomo com sincronização determinística

- **Status:** ✅ **aceito** (2026-08-03) — as 4 questões em aberto foram decididas; ver a seção
  final. Continua valendo: **nenhuma linha de firmware, rádio ou `bake`** antes do F2 abrir.
- **Data:** 2026-08-03
- **Fonte:** FASE F1 do roadmap (`docs/ROADMAP.md`) — trajes de LED para performance de dança
- **Relaciona-se com:** ADR-0001 (replay determinístico), ADR-0004 (Ed25519 com chave fixada),
  **ADR-0005 (WiFi proibido ao vivo — permanece íntegro)**, ADR-0010 (failover parcial),
  ADR-0013 (isolamento de falha), **ADR-0021 (efeito é função pura — condição *necessária*,
  não suficiente, para render a bordo: pureza ≠ paridade bit a bit; ver Q4)**. **Não** depende
  do ADR-0017: um traje autônomo não tem heartbeat (ver Q1)

---

## Contexto e problema

O LUMYX hoje é uma plataforma de **streaming**: `engine → HAL → DeviceDriver → fio →
controlador → pixel`, quadro a quadro, em tempo real.

Um traje de dança **não tem fio**. E o ADR-0005 proíbe WiFi ao vivo — com medição própria
que confirma o porquê: **jitter de 31 ms** com RSSI −44 (sinal forte), e uma falha de
`sendto` a cada ~6 min no burn-in (`CLAUDE.md`, 2026-07-20 e 2026-07-23). Pela nossa
própria evidência, streaming sem fio para palco é inviável.

Isso não é um impasse: é um sinal de que **a pergunta está errada**. "Como transmitir para
um traje?" pressupõe que o traje precisa receber quadros durante o número. Não precisa.

## Decisão

**O traje toca o show sozinho, a partir de um artefato assado localmente. O que o mantém
junto com os outros não é um fluxo de dados — é um relógio comum.**

Oito decisões derivadas:

### D1 — O traje é um *player*, não um *device*

Esta é a inversão que faz o resto encaixar.

No modelo atual, um alvo de saída é um `DeviceDriver` no fim do fan-out do HAL. Um traje
**não é isso**: ele é uma instância **par** do `led-player`, com o seu próprio show em
memória e a sua própria fita local.

Consequência forte e desejável: **nenhum seam Frozen é envolvido**. `ProtocolOutput`,
`DeviceDriver`, `IDevice`, `CompiledLayout` e `UniverseData` não aparecem no caminho do
traje, porque não há rede no caminho do traje. Um traje é ao rig o que um segundo computador
tocando o mesmo filme é ao primeiro — não um monitor do primeiro.

### D2 — Sincronismo é relógio, não fluxo

Trajes não trocam quadros. Compartilham uma **base de tempo** e cada um decide, sozinho,
qual quadro corresponde a `t`. O contrato de sincronismo é, portanto, um contrato **de
tempo**, não de transporte:

```
mesmo show assado  +  mesma base de tempo  ⇒  mesmo quadro no mesmo instante
```

O LUMYX já tem as duas metades e elas nunca foram ligadas uma na outra:
`SharedClock` (monotônico, offset assinado, `now_ms() = max(anterior, atual)` —
`crates/led-hal/src/shared_clock.rs:37-95`) e `net_time`
(`TimeServer`/`measure_offset`/`best_of`/`sync_to` — `crates/led-hal/src/net_time.rs:45-146`,
offset medido a ±10 ms).

### D3 — Pacing é por tempo **absoluto**; o pacing atual é incremental e **não serve**

> **Este é o achado que justifica o ADR existir agora, e não é hipótese.**

`led_player::play_instrumented` (`crates/led-player/src/lib.rs:112-120`) espaça quadros
assim:

```rust
let gap = r.timestamp_ms.saturating_sub(prev);
std::thread::sleep(Duration::from_micros(gap as f64 * 1000.0 / f as f64));
…
let t0 = std::time::Instant::now();
output.send_frame(&frame)?;          // t0.elapsed() é MEDIDO…
m.record_frame(t0.elapsed()…);       // …e registrado
```

Duas propriedades estruturais disso:

1. **É livre-corrente (free-running).** Cada `sleep` é do *intervalo*, não até um *instante
   alvo*. O erro de granularidade do escalonador não é corrigido no quadro seguinte —
   **acumula**.
2. **O tempo de envio não é descontado.** O período real é `gap + tempo_de_envio`, e
   `t0.elapsed()` é medido para métrica mas nunca subtraído do próximo `sleep`. Isso não é
   jitter: é um **viés sistemático de lentidão**, monotônico ao longo do número.

Para o rig cabeado isso nunca importou — um só player, e a percepção é relativa. Para N
trajes independentes, é exatamente o mecanismo que os separa.

**Decisão:** o playback sincronizado espaça por **instante alvo absoluto** contra um
`SharedClock` — `alvo = epoch_do_show + timestamp_ms` — dormindo até o alvo e **descartando
ou repetindo** quadro quando atrasado, em vez de empurrar o erro para a frente. Assim o
desvio fica limitado pela **exatidão do relógio**, não pelo acúmulo do escalonador.

O `Speed::Factor` atual **não é removido**: continua correto para bancada, teste e
re-verificação de integridade. O modo absoluto é **aditivo**.

### D4 — A janela de sincronismo é **antes** do número, nunca durante

Trajes sincronizam relógio e recebem o show por **caminho cabeado** (Ethernet/USB, na
montagem ou na passagem de som) e depois ficam **autônomos**. Durante o número não há
rádio, não há rede, não há recepção.

**Isto não abre exceção ao ADR-0005 — reforça-o.** O ADR-0005 governa *saída ao vivo por
rede* ("a saída roda em Ethernet cabeada (ou DMX/SPI com fio)"). Aqui não existe saída por
rede: o único caminho de dados durante o show é interno ao traje. **O ADR-0005 permanece
literalmente intacto e o `NetworkGuard` não é tocado.**

O que **é** proibido explicitamente por este ADR: usar rádio (WiFi, ESP-NOW, ISM, W-DMX)
para **transportar quadros** durante o número — porque um pacote de tempo perdido degrada
precisão, enquanto um quadro perdido **apaga o corpo do bailarino**.

Um sinal de rádio de **disparo/tempo** foi **decidido na Q2: também não durante o número.**
O show tem de estar correto com o rádio ausente, interferido ou hostil; o sincronismo é
100 % pré-show e cabeado, e o abortamento é físico/local. Um canal remoto de aborto
autenticado fica para **ADR próprio** — ver a justificativa em Q2.

### D5 — Deriva é orçamento declarado, medido, e com gatilho

Depois do sincronismo pré-show, cada traje corre com o **seu próprio oscilador**. Deriva é
inevitável; o que não pode é ser **desconhecida**.

Este ADR **não afirma nenhum número de ppm**. Osciladores de MCU derivam na casa de dezenas
de ppm e variam com temperatura — e um traje fica **contra um corpo quente**, que é
justamente o pior caso térmico. Qualquer número aqui seria inventado.

**Decisão:** o orçamento de deriva é um valor **declarado** no perfil do show, e a deriva
real é **medida em bancada antes de qualquer uso em palco** (gate G6). Enquanto não houver
medição, o número correto a escrever em qualquer documento é *não medido*.

### D6 — O invariante do heartbeat **não se aplica**, e isso precisa ser dito

O heartbeat existe porque um controlador **alimentado por streaming** entra em safe-mode
após ~2,4 s de silêncio (`LUMYX_GOSL.md`, Hardware Rules; `crates/led-hal/src/heartbeat.rs`).
Um traje autônomo aciona a própria fita localmente — **não há lacuna de rede para cobrir**.

O risco não desaparece: ele **muda de forma**. No streaming, "silêncio" era a falha; no
autônomo, a falha é o **player parar** — e nesse caso não há ninguém do outro lado para
notar. Por isso D7.

Escopo declarado: o invariante 2,4 s continua valendo **integralmente** para todo alvo
alimentado por rede. Este ADR não o afrouxa; declara que ele não governa um caminho onde
não existe rede.

### D7 — Falha de um traje não pode derrubar o número, e tem de ser **visível antes**

Precedente do ADR-0010: entrega parcial é melhor que nenhuma. Mas há uma inversão cruel —
**sem rede, ninguém fica sabendo que um traje falhou**. Não há `SegmentHealth` para
observar de fora.

Portanto:
- A detecção é **pré-show**, não em runtime: um gate de prontidão (show verificado, relógio
  sincronizado, bateria, integridade da fita) que **reprova o traje antes de ele subir**.
- A falha em cena tem **estado definido e seguro**, decidido por quem monta o show — não um
  comportamento emergente. **Decidido na Q1:** o traje **apaga o conteúdo do show**, e o
  estado de falha é um **parâmetro declarado pelo show cujo padrão é preto**. Isto **não**
  depende do ADR-0017: aquele ADR trata do que o *heartbeat* reenvia num rig transmitido, e
  aqui não há heartbeat nem rede.

### D8 — Autenticidade viaja com o show

`verify_manifest_pinned` (`crates/led-show-recorder/src/signing.rs:132`) já resolve o
problema difícil: verificação Ed25519 contra chave **pré-confiada**, não contra a chave
embutida no blob (foi o achado CRITICAL **RT-001**).

Um traje é fisicamente acessível nos bastidores por qualquer pessoa — **mais exposto** que
um controlador em rack. Portanto: o traje **verifica com chave fixada antes do primeiro
quadro** e recusa tocar o que não verificar. Não é recurso novo; é aplicar o que já existe
num lugar onde o risco é maior.

---

## O que **não** está sendo decidido aqui

Explicitamente fora de escopo, e **não** deve ser implementado ou presumido:

| Fora de escopo | Por quê |
|---|---|
| Plataforma embarcada (firmware próprio × preset WLED × outro) | Decisão de engenharia com trade-off real; merece ADR próprio depois de medida |
| Meio físico do sincronismo pré-show | O ADR fixa o **contrato de tempo**, não o fio |
| Rádio de disparo/tempo durante o número | **Decidido na Q2: não.** Canal de aborto remoto autenticado → **ADR próprio** |
| Bateria, peso, dissipação, segurança de contato | Engenharia elétrica; entra depois do orçamento medido |
| Formato do artefato assado | `.lumyx` é o candidato natural (`LUMX`/v1, header 16 B — `crates/led-show-recorder/src/lib.rs:56-58`), mas 73 MB para 6.200 px × 3.925 quadros não cabe num traje sem compressão ou redução de escopo. **Dimensionar antes de decidir.** |

---

## Critérios de validação

Um critério só conta se houver uma execução descrita que o faça **reprovar** (KB-012).

| # | Critério | Aceite |
|---|---|---|
| V1 | **Pacing absoluto não acumula erro** | Desvio entre o instante alvo e o real permanece limitado ao longo de um número inteiro, sem tendência crescente |
| V2 | **O pacing atual reprova o mesmo teste** | `Speed::Factor` (incremental) exibe desvio com tendência monotônica — provando que V1 mede algo real |
| V3 | **Assar é determinístico** | Assar o mesmo show duas vezes produz bytes idênticos |
| V4 | **Assar preserva o pixel** | `pixel_hash` do show assado = `pixel_hash` do render do engine (FNV-1a, `lib.rs:311`) |
| V5 | **Show não autenticado é recusado** | Chave errada / manifesto adulterado ⇒ recusa antes do 1º quadro (`verify_manifest_pinned`) |
| V6 | **Deriva real medida**, não presumida | Número com origem, condição e temperatura; enquanto não existir, o valor é *não medido* |
| V7 | **ADR-0005 intacto** | Nenhuma variante nova de `OutputInterface` autoriza rádio ao vivo; `NetworkGuard` inalterado |
| V8 | **Nenhum seam Frozen tocado** | Superfície de seam inalterada no guardião |
| V9 | **Estado de falha é definido** | Um traje que para **apaga o conteúdo do show**; o estado é parâmetro declarado, padrão preto (Q1) — **não** bloqueado pelo ADR-0017 |
| V10 | **Prontidão é pré-show** | Traje não pronto é reprovado antes de subir, nunca descoberto em cena |

---

## Gates

Executáveis, na disciplina do repositório. **Nenhum destes deve ser escrito antes do aceite
deste ADR** — estão aqui para que o aceite seja informado.

| Gate | O que roda | Reprova quando |
|---|---|---|
| **G1** — deriva do pacing | N players simulados sobre `SharedClock`, show de duração realista; mede desvio alvo↔real por quadro | O desvio tem tendência crescente, ou excede o orçamento declarado |
| **G2** — controle negativo de G1 | O mesmo teste com o `Speed::Factor` atual | **G2 tem de FALHAR.** Se passar, G1 não está medindo acúmulo e é um gate falso-verde |
| **G3** — determinismo do bake | Assa 2×, compara bytes | Qualquer diferença |
| **G4** — fidelidade do bake | `pixel_hash(assado)` vs `pixel_hash(render)` | Hashes diferentes |
| **G5** — autenticidade | Show re-assinado por atacante contra chave do estúdio | Se **não** recusar (espelha `pinned_verify_rejects_resigned_tamper`) |
| **G6** — deriva de hardware | Bancada: 2+ nós, sincronizados, livres por ≥ a duração do número, temperatura registrada | **Bloqueia palco enquanto não executado.** Sem este número, o orçamento de D5 é ficção |
| **G7** — não-regressão do ADR-0005 | `NetworkGuard` + presets + `OutputInterface` | Qualquer caminho novo que permita quadros por rádio ao vivo |
| **G8** — seams | `scripts/lumyx_guardian.sh` | Superfície de seam alterada sem bump |

**G6 é o gate que separa este ADR de um plano de slides.** Todos os outros rodam em
software; G6 exige duas placas numa bancada. Enquanto ele não rodar, nada disto está
provado em cima de nada físico.

---

## Questões em aberto — **resolvidas** (2026-08-03)

### Q1 — falha em cena: **apaga**, com estado declarado ✅ decidido

**Correção de uma imprecisão da primeira redação deste ADR.** A versão original disse que a
Q1 dependia do ADR-0017. **Não depende.** O ADR-0017 pergunta o que o *heartbeat* reenvia
quando o blackout é comandado; um traje autônomo **não tem heartbeat e não tem rede**. As
duas perguntas são independentes, e a Q1 foi decidida sem esperar aquela.

**Decisão:** o traje que falha **apaga o conteúdo do show**. Congelar é pior — um corpo
parado num quadro estático enquanto os outros animam **denuncia a falha**; escuro se lê como
saída de cena.

**Mas o estado de falha é um parâmetro declarado pelo show, cujo padrão é preto — não uma
constante.** A razão é física, não estética: num número de trajes o palco é preto e **as
roupas são a única fonte de luz**. Um traje totalmente apagado torna o bailarino invisível
para os outros bailarinos, que se movem rápido no escuro — risco de colisão. Quem decide se
um corpo pode sumir é a coreografia, e para decidir precisa que o parâmetro exista.

### Q2 — rádio: **nenhum no caminho crítico** ✅ decidido

**Quadros por rádio: proibido, permanentemente.** É a linha do ADR-0005 e não se negocia.

**O show tem de estar correto com o rádio ausente, interferido ou hostil.** Sincronismo é
100 % pré-show e cabeado (D4). Abortamento durante o número é **físico/local**.

Um canal remoto de aborto **não entra neste ADR** e vai para ADR próprio, por duas razões
que se opõem e precisam ser pesadas juntas: sem ele **não existe parada de emergência** (se
um bailarino cai, os trajes tocam até o fim); com ele mal projetado, **um kill-switch por
rádio não autenticado é superfície de ataque** — um SDR barato apaga um show de visibilidade
alta, e sem contador/nonce um comando capturado pode ser **reproduzido**. O F2 não deve
herdar uma superfície de segurança ainda não desenhada.

### Q3 — duração sem re-sincronizar: **quociente, não escolha** ✅ forma definida

Continua **sem número**, e isso é honesto: o denominador vem do G6. O que fica definido é a
**forma** do orçamento, para o G6 saber o que medir:

> A tolerância é **menos de um quadro à taxa do show** — a 40 fps, **25 ms**. Acima disso
> dois trajes exibem literalmente quadros diferentes.

O limite é derivado da taxa do show, **não** de psicofísica (o limiar perceptual de
assincronia é outra pergunta, não respondida aqui). Daí:

```
duração_máxima = orçamento_de_desvio ÷ taxa_de_deriva_medida (G6)
```

### Q4 — tamanho do artefato: **a duração domina, e o E1 abriu uma saída melhor** ✅ enquadrada

**Erro corrigido:** a primeira redação tratou "73 MB" como se o show inteiro fosse para cada
traje. Não vai — pela D1 cada traje é um player **dos seus próprios pixels**. Mas a correção
seguinte é mais importante: os 73 MB são de um show de **98 s**, e um número de dança tem
3–5 min. **A duração é o termo dominante.**

Formato atual (cru, 3 B/px + 9 B/quadro + 16 B de cabeçalho — verificado ao byte contra
`robot_sequence.lumyx`: 73 005 000 crus + 35 341 de overhead = 73 040 341):

| flash | teto de pixels (número de **4 min** @ 40 fps) |
|---|---|
| 4 MB | 142 px |
| 8 MB | **288 px** |
| 16 MB | 579 px |
| 32 MB | 1 162 px |

**Envelope de projeto** (derivado do rig do usuário: 430 modelos ≈ 20 px cada, 1 240 px por
robô; silhueta humana em fita ≈ 4–5 m a 60–144 px/m): **150–750 px por traje**. O número real
é entrada de gate, não bloqueio do F2.

**Três alavancas, em ordem de custo — compressão é a última, não a primeira:**

1. **Taxa de quadros.** 400 px / 4 min: 40 fps = 11,1 MB → **25 fps = 6,9 MB** (−38 %).
   Grátis; preserva seek O(1); zero CPU; zero classe de falha nova.
2. **Render a bordo em vez de replay de quadros** — *direção candidata, **não** aprovada.*
   O **ADR-0021** tornou todo efeito uma **função pura do tempo**, então um traje poderia
   guardar a *timeline* (spans + parâmetros) e renderizar: **kilobytes, não megabytes**.
   Pureza, porém, **não é o mesmo que paridade bit a bit** — ver a ressalva abaixo, que é
   bloqueante.
3. **Compressão.** Só se 1 e 2 não bastarem. Compra tamanho pagando CPU, bateria e um modo
   de falha inédito num dispositivo que não pode falhar no meio do número.

### A ressalva que bloqueia o render a bordo

> **Correção de uma afirmação forte demais numa redação anterior deste ADR.** Chegou a estar
> escrito que a divergência de `f32` "não se aplica entre trajes, que são hardware idêntico".
> **Isso não tem evidência e está retirado.**

`integration-tests/tests/determinism_vector.rs:9-12` atribui a divergência a
**implementações de libm** — não a arquiteturas diferentes. Hardware idêntico remove *uma*
fonte de variação e **não remove as outras**: toolchain e versão do compilador, flags de
otimização, `libm`/`compiler-builtins` do alvo, versão de firmware, presença e modo da FPU,
configuração de arredondamento do MCU, contração de multiply-add. Dois trajes "iguais" podem
ter sido gravados em semanas diferentes com toolchains diferentes — e ninguém notaria até o
palco.

E o modo de falha é assimétrico: no replay de quadros, uma divergência é **impossível** (os
bytes já estão assados); no render a bordo, ela aparece como **dois bailarinos com cores
levemente diferentes**, sem erro, sem log, sem ninguém a bordo para notar (D6/D7).

**Portanto, condição de adoção — não negociável:**

1. **Contrato de determinismo explícito**, escrito antes de qualquer código: o que
   exatamente é garantido idêntico, sob quais versões de toolchain e firmware fixadas.
2. **Gate executável** que compare artefatos/quadros renderizados entre **no mínimo dois
   dispositivos físicos com firmware idêntico**, e reprove com divergência de **um** byte.
   Sem essa comparação em hardware real, o render a bordo permanece **não adotado**.

**Direção preferencial** — registrada como direção, **não** como implementação pronta e
**não** como solução verificada: onde a paridade byte a byte for exigida, usar **aritmética
inteira/ponto-fixo ou LUTs determinísticas** em vez de trigonometria em `f32`. É a mesma
mitigação que o próprio `determinism_vector.rs:12` já aponta ("table-based or fixed-point
trig in kernels"), e a mesma disciplina do ADR-0019 (gamma e brilho dobrados numa LUT de 256
entradas, sem `powf` no caminho quente). Nenhum número de custo ou de erro é afirmado aqui:
**não medido**.

**Consequência para o F2:** o fork real **não é "comprimir ou não"** — é **replay de quadros
× render a bordo**. E os dois lados **não estão empatados**: o replay é o caminho
conservador e já provado (bytes assados, divergência impossível); o render a bordo é a
opção de tamanho, e **começa reprovado** até existirem o contrato e o gate de dois
dispositivos acima. Essa é a primeira decisão do F2, não um detalhe de formato.

---

## Alternativas descartadas

**Streaming sem fio com rádio dedicado (ESP-NOW / ISM / W-DMX).** Trocaria WiFi por um rádio
melhor, mas mantém a propriedade que a nossa própria medição condenou: o corpo do bailarino
depende, quadro a quadro, de um enlace sem fio, num ambiente de palco cheio de rádio e de
corpos (água) entre transmissor e receptor. O ADR-0005 nasceu de 31 ms de jitter medidos;
mudar o rádio muda o número, não a categoria de risco.

**Streaming com buffer grande no traje.** É o argumento já rejeitado pelo ADR-0005 em outra
roupagem: buffer troca jitter por latência, e para show sincronizado com música latência
variável é tão ruim quanto perda. Além disso, um buffer grande o bastante para cobrir uma
falha de rádio **já é** o show inteiro — ou seja, já é playback autônomo, só que com um
rádio desnecessário ligado.

**Cabo até o traje.** Resolve tudo tecnicamente e é incompatível com dança. Registrado para
que a rejeição seja explícita, não esquecida.

**Tratar o traje como `DeviceDriver` remoto.** Forçaria os seams Frozen a atravessar um
enlace sem fio e arrastaria `CompiledLayout`/`UniverseData` para dentro do traje sem
necessidade. D1 evita isso inteiramente.
