# Bench A: audio-core FFT + features
# git-hash: 5eec9f7e90e75ccf122be8c78fdf13fa9e30d341
# Generated: 2026-06-21

## Single hop isolation (process_hop, 1000 runs, debug build)
avg=658us  p50=459us  p99=5345us  within-5ms-budget=OK

## analyze_all throughput
duration   hops       total_ms     avg_ms/hop     hops/sec
1s         187        116          0.620          1612
5s         937        643          0.686          1457
10s        1875       1173         0.626          1598

## test result: ok. 1 passed; 0 failed; 0 ignored
Budget: ≤5ms per hop (real-time = 5.3ms at 48kHz/256-hop). PASSES at all scales.
Note: audio-core is pixel-count independent — processes fixed 256-sample hops.
