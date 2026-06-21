# Bench C: led-hal layout apply
# git-hash: 5eec9f7e90e75ccf122be8c78fdf13fa9e30d341
# Generated: 2026-06-21

## Layout apply latency (200 runs, debug build, via Hal::send_frame)
pixels       universes    avg_us     p50_us     p95_us     budget
10000        59           931        802        1474       OK <=1ms
50000        295          7071       4886       11885      OVER budget
100000       589          10017      9055       15103      OVER budget

## test result: ok. 1 passed; 0 failed; 0 ignored
GARGALO: layout apply exceeds 1ms budget at 50k px (7ms avg) and 100k px (10ms avg).
This is the FIRST stage to exceed budget — identified as the scaling bottleneck.
