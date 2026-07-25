# Runbook — Controlador não responde (offline)

Um controlador esperado não responde ao discovery pré-show. Este runbook cobre o
diagnóstico e a recuperação usando o `--discover`/`--require-all` que já existe no
`led-player` (`crates/led-player/src/main.rs`).

> Documento apenas. Nenhum comando altera o código da plataforma.
>
> **Escopo.** Isto trata da **falha na partida** (controlador offline **antes** do
> show). **Não** é um runbook de *cluster-failover* (nó que morre **no meio** do
> show) — esse só será escrito depois da validação física do rig.
>
> **Legenda de validação**
> - ✅ **VALIDADO** — caminho já exercido contra o rig real **desligado**.
> - ⚠ **NÃO VALIDADO EM HARDWARE** — caminho que depende de um controlador **vivo**;
>   nunca observado (rig offline).

---

## 1. Sintoma

Na partida (ver [show-startup.md](./show-startup.md), passo 5/6) aparece:

```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ⚠ SEM resposta
ABORT: --require-all e 192.168.2.156 não respondeu ao ArtPoll (desligado? WiFi morto? subnet errada?)
```
exit 1 — o show **não inicia** (✅ este ABORT foi provado contra o rig offline).

O objetivo deste runbook é transformar esse `⚠ SEM resposta` em `✅ respondeu`.

---

## 2. Pré-condições

- Máquina de controle na LAN de palco (192.168.2.10), rede cabeada.
- Lista de IPs esperados: **192.168.2.156–160** (robô led 1–5).
- Porta UDP **6454** livre na máquina de controle (o ArtPoll usa ela).

---

## 3. Isolar qual controlador está mudo  ✅ VALIDADO (caminho de aborto)

O `--discover` sonda **um** alvo por vez (o de `--artnet`/`--ddp`). Rode um por um
para achar o(s) mudo(s). `--info` evita qualquer playback — é só o probe.

**Comando (repita trocando o IP .156 → .160):**
```
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --require-all --info
```

**Saída — controlador vivo** (⚠ NÃO VALIDADO EM HARDWARE):
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ✅ respondeu
```
(segue para o `--info`, exit 0.)

**Saída — controlador mudo** (✅ VALIDADO):
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ⚠ SEM resposta
ABORT: --require-all e 192.168.2.156 não respondeu ao ArtPoll (desligado? WiFi morto? subnet errada?)
```

Anote quais IPs deram `⚠ SEM resposta`. Esses são os alvos da recuperação (§4).

**Se o probe nem roda:**
```
discovery falhou (socket ArtPoll :6454 — precisa de porta livre): ...
```
→ outro processo segura a porta 6454 (outro `led-player`, xLights, um controlador
de luz aberto). Feche-o e repita. **Não é o controlador** — é a máquina de controle.

---

## 4. Árvore de recuperação (para cada IP mudo)

A própria mensagem lista as três causas mais comuns — *desligado? WiFi morto?
subnet errada?*. Cheque nesta ordem (mais barato primeiro):

### 4a. Energia / boot
- [ ] O controlador está ligado? LED de power aceso?
- [ ] Se PoE (Opção A da [migração](./wifi-to-ethernet-migration.md)): a porta do
      switch está energizada? LED de link do switch aceso?
- [ ] Se ligou agora, espere ~10 s pelo boot do WLED e repita o §3.

### 4b. Link físico (Ethernet)
- [ ] Cabo Ethernet firmemente conectado nas duas pontas?
- [ ] LED de link/atividade aceso na porta do switch **e** no board?
- [ ] Troque o cabo/porta por um sabidamente bom se estiver na dúvida.

### 4c. IP / subnet
- [ ] O board está no IP estático certo (.156–.160)?
- [ ] Pingável da máquina de controle?
      ```
      ping -c 3 192.168.2.156
      ```
      - Ping **responde** mas ArtPoll não → §4e (WiFi vs ETH) ou porta/firewall.
      - Ping **falha** → IP/subnet/cabo. Confira a config estática do WLED.
- [ ] Sem DHCP no palco: um board que pegou IP errado "some". Reforce o IP fixo
      (ver [migração §3](./wifi-to-ethernet-migration.md)).

### 4d. Subnet da máquina de controle
- [ ] A máquina de controle está em `192.168.2.10/24`, na **mesma** LAN de palco,
      sem sair por outra interface (WiFi de casa competindo)?
- [ ] O `NetworkGuard` recusa iniciar com WiFi ativo — desligue o WiFi da máquina
      de output e use só o cabo.

### 4e. Interface do controlador: Ethernet, não WiFi
- [ ] Na UI do WLED, a interface ativa é **Ethernet** (não WiFi)?
- [ ] Firmware com suporte a Ethernet e board selecionado corretamente
      (ex.: "Olimex ESP32-POE")? Ver [migração §4](./wifi-to-ethernet-migration.md).
- [ ] WiFi ao vivo é proibido (jitter 5–50 ms) — ver [ADR-0005](../adr/0005-wifi-proibido-producao.md).

---

## 5. Re-validar  ⚠ NÃO VALIDADO EM HARDWARE (caminho ✅ respondeu)

Depois de corrigir, repita o §3 para o IP recuperado até obter:
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ✅ respondeu
```
(Este caminho depende de um controlador vivo — nunca observado no rig offline;
a linha acima é a que o código emite, presumida no metal.)

Quando **todos** os 5 responderem individualmente, faça o discovery final na
ordem de partida e só então prossiga para o show:
```
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --require-all --info
```
> Nota: o `--require-all` de hoje aborta no **primeiro** alvo mudo do comando (um
> IP por invocação). Um discovery multi-IP numa passada é evolução futura; por ora,
> a checklist um-por-um do §3 é o gate dos 5.

---

## 6. O que NÃO fazer

- **Não** inicie o show sem `--require-all` só para "furar" o ABORT — um controlador
  mudo com `--discover` sozinho vira só um aviso
  (`aviso: seguindo mesmo assim (sem --require-all); o palco pode ficar escuro`) e
  o palco fica parcialmente apagado sem erro. No palco, `--require-all` é
  obrigatório.
- **Não** atualize firmware do controlador durante um show ao vivo (regra do
  projeto: firmware nunca é atualizado ao vivo). Recupere pela rede/energia; troca
  de firmware é fora de show.
- **Não** volte para WiFi para "resolver rápido". WiFi ao vivo é proibido
  ([ADR-0005](../adr/0005-wifi-proibido-producao.md)); um controlador que só
  responde em WiFi ainda está reprovado para o show.

---

## 7. Se um controlador cai NO MEIO do show

Fora do escopo deste runbook. O código tem histerese de saúde por segmento
(`SyncedCluster`: Healthy → Degraded após 3 falhas → Failed após 10; envio parcial
é `Ok`; ver [ADR-0010](../adr/0010-cluster-failover.md)), mas o runbook operacional
de *cluster-failover ao vivo* **só será escrito após a validação física** — não
inventar procedimento de palco para um caminho nunca exercido no metal.
