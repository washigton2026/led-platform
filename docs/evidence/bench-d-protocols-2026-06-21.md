# Bench D: led-protocols sACN serialization
# git-hash: 5eec9f7e90e75ccf122be8c78fdf13fa9e30d341
# Generated: 2026-06-21

## sACN packet serialization (100 runs, debug build)
pixels       universes    avg_total_us   per_univ_us    budget
10000        59           35             0              OK <=1ms
50000        295          211            0              OK <=1ms
100000       589          410            0              OK <=1ms

## test result: ok. 1 passed; 0 failed; 0 ignored
Budget: ≤1ms for full universe batch. PASSES at all scales (410us at 100k px).
Note: pure CPU serialization — network I/O not included.
