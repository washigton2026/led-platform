# Bench B: led-pixel-engine render
# git-hash: 5eec9f7e90e75ccf122be8c78fdf13fa9e30d341
# Generated: 2026-06-21

## Render latency (500 runs per config, debug build)
pixels       effect         avg_us     p50_us     p95_us     budget
10000        SolidColor     91         72         122        OK <=5ms
10000        BandPulse      85         69         108        OK <=5ms
50000        SolidColor     409        331        557        OK <=5ms
50000        BandPulse      410        327        571        OK <=5ms
100000       SolidColor     1183       721        3548       OK <=5ms
100000       BandPulse      1242       834        4761       OK <=5ms

## test result: ok. 1 passed; 0 failed; 0 ignored
Budget: ≤5ms avg per frame. PASSES at all scales including 100k px (1.2ms avg).
Note: p95 at 100k reaches 3.5-4.8ms — headroom tight but within budget.
