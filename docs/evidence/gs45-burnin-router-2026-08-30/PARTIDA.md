# Burn-in DDP sob topologia router — condições de partida

```
Inicio         2026-08-30 16:48:34
HEAD           0ad00942e3f75d103a100b6ac2d7e02a8056b46a
Branch         baseline/f2-f71
Binario        target/release/led-player  mtime=Aug 28 12:41:57 2026  NAO recompilado
Mac boot       2026-08-30 09:57:16   (uptime  6:51)
en7 idade      enumerado 16:28:31, ou seja 20 min  <-- CONDICAO CRITICA
en7            192.168.2.163  status=active  media=1000baseT full-duplex  MAC=00:e0:4c:30:45:80
en0 WiFi       192.168.2.32   status=active  (multi-homed presente)
rota .162      en7
en7 counters   Ipkts=302244 Ierrs=0 Opkts=110814 Oerrs=0 Coll=0
load           { 10.13 66.36 84.75 }
WLED           ver=16.0.1 uptime=1186s freeheap=146752 live=False lm='' lip='' ap=False bssid=''
ping .162      round-trip min/avg/max/stddev = 1.098/1.228/1.405/0.142 ms
```

Braço: **SEM `--bind` (wildcard)** — o braço histórico de referência do Modo A.
Comando: `./target/release/led-player striptest.lumyx --ddp 192.168.2.162 --loop 150`

**Instrumentação:** nenhum amostrador paralelo (directiva §3 proíbe-o). O burn-in de 08-30
correu **com** um amostrador `netstat` a 200 ms; esta corrida não. É uma diferença de carga
deliberada e dirigida, e limita a comparação — fica nomeada, não escondida.
sleep settings:  displaysleep         3  sleep                1  disksleep            10 
