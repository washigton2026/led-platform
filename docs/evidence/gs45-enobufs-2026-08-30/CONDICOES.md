# Condições experimentais — GS4.5 ENOBUFS, 2026-08-30

git-hash: 0ad00942e3f75d103a100b6ac2d7e02a8056b46a
branch: baseline/f2-f71
working-tree: NÃO COMMITADO — 14 ficheiros

boot: { sec = 1788066741, usec = 765353 } Sun Aug 30 07:12:21 2026
uptime-no-arranque-da-corrida: ~41 min
cpus: 4

binario: target/release/led-player  mtime=2026-08-28 12:41
comando: led-player striptest.lumyx --ddp 192.168.2.162 --loop 150
braco: WILDCARD (sem --bind) — binario imprimiu 'origem: 0.0.0.0:0 (a tabela de rotas escolhe)'

ENOBUFS: REPRODUZIDO — abort pass 81, 93/94 frames falhados, exit 1
interface durante o surto: en7 CESSOU DE EXISTIR (ifnet_detach_final @ 08:00:42.290)
trafego em en0 durante o surto: NAO (en0_opkts estavel em 297)

amostradores paralelos: SIM (2x netstat @200ms) — tempo-ate-falha NAO comparavel com corridas limpas
load average no arranque: ~35.8 em 4 CPUs (Claude.app/Chrome/WindowServer)

## Validacao runtime do --bind (2026-08-30, apos o desanexo)

Loopback, porque en7 deixou de existir as 08:00:42 e nao voltou.

| corrida | flag | origem declarada | origem observada no receptor | veredito |
|---|---|---|---|---|
| bind | --bind 127.0.0.1:57692 | 57692 | **57692** | origem declarada honrada |
| wildcard | (sem flag) | (56850 nao declarada) | 59803 efemera | discrimina |

Binario imprime mensagens distintas: 'interface fixada pelo operador' vs
'a tabela de rotas escolhe'. 1471 bytes DDP lidos do socket.

**NAO MEDIDO:** que a origem *Ethernet* vence a tabela de rotas num host
multi-homed. Exige en7, que nao existe. O teste dedicado continua #[ignore].

## FASE 6-8 — validacao do --bind sobre Ethernet real (2026-08-30, router)

Topologia: Mac --Eth-- Router --Eth-- ESP32-POE; WiFi mantido ligado.
en0 = WiFi 192.168.2.32 active | en7 = USB 10/100/1000 LAN 192.168.2.163 active
en7 media 1000baseT full-duplex, Ierrs/Oerrs/Coll = 0. route get .162 -> en7.
ping en7 1.90ms/0.31 jitter vs en0 9.38ms/16.57 jitter.

Linha de base do WLED ANTES: live=False lm='' lip='' (campos vazios).

| corrida | flag | binario | receptor lip | frames |
|---|---|---|---|---|
| 1 | --bind 192.168.2.163 | interface fixada | **192.168.2.163** | 94/0 |
| 2 | --bind 192.168.2.32 | interface fixada | **192.168.2.32** | 94/0 |
| 3 | (sem flag) | tabela de rotas escolhe | **192.168.2.163** | 94/0 |

Corrida 2 e o discriminante: a rota resolve en7/.163 e o receptor observou .32.
O IP declarado VENCE a tabela de rotas. Observado no RECEPTOR (WLED lip),
nao na configuracao do CLI. live=True lm=DDP nas tres.

FASE 6 (#[ignore] multi-homed): NAO MEDIDO. A guarda da premissa disparou --
o binario de cargo test nao entrega em rede local (privacidade Local Network do
macOS, por binario; os error 35). O teste diz que nao exercitou o bind.
O binario de PRODUTO entrega: 81 passes DDP reais hoje + as 3 corridas acima.

## RE-VERIFICACAO independente das FASES 7-8 (2026-08-30 17:51)

Repetida do zero apos o burn-in de 150 passes, com en7 recuperada.
Linha de base lida ANTES de cada corrida: live=False lm='' lip='' (vazia nas tres).

| flag | binario imprime | lip observado no WLED |
|---|---|---|
| --bind 192.168.2.163 | interface fixada pelo operador | 192.168.2.163 |
| --bind 192.168.2.32  | interface fixada pelo operador | 192.168.2.32  |
| (sem flag)           | a tabela de rotas escolhe      | 192.168.2.163 |

Confere com a tabela original. A 2a corrida e o discriminante: rota -> en7/.163,
receptor observou .32. O IP declarado vence a tabela de rotas.
