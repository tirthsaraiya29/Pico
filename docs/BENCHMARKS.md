# Measured Benchmarks

All metrics below were directly measured on Windows 11 x64 (`cargo test -p pico-orchestrator --test benchmark_test -- --nocapture` debug profile and `target/release/pico-desktop.exe` release binary).

## Decision, Retrieval, Calibration & Execution (`benchmark_test`, debug build)

| Benchmark Metric | Observed Result | Budget / Target | Source |
|---|---|---|---|
| Audio Resampling (`48 kHz stereo -> 16 kHz mono`, 20ms frame) | `0.120 ms` | `< 1.0 ms` | `benchmark_test` |
| BM25 Candidate Retrieval (70 host tools) | `0.091 ms` | `< 1.0 ms` | `benchmark_test` |
| MultiFeature Ranking & Joint Calibration | `0.134 ms` | `< 2.0 ms` | `benchmark_test` |
| End-to-End Decision Latency (mean / 100 calls) | `4.876 ms` | `< 15.0 ms` (debug) | `benchmark_test` |
| Calculator AST Execution | `0.011 ms` | `< 1.0 ms` | `benchmark_test` |
| System Info Execution (`PlatformSysInfo`) | `0.015 ms` | `< 50.0 ms` | `benchmark_test` |
| SQLite Memory Store (Write / Read) | `0.177 ms / 0.144 ms` | `< 5.0 ms` | `benchmark_test` |
| Top-1 Decision Accuracy (21 held-out multi-domain queries) | `95.2% (20/21)` | `>= 85.0%` | `benchmark_test` |
| Top-3 Recall (21 held-out multi-domain queries) | `100.0% (21/21)` | `>= 95.0%` | `benchmark_test` |
| Fitted Platt Calibration (`a=6.81, b=-3.78`) | `ECE = 0.0962`, `Brier = 0.0918` | `< 0.15` ECE | `benchmark_test` |
| Sustained Runtime Throughput (500 turns) | `231.2 QPS` (`2.16s`), `42.75 MB` WS | Bounded growth | `benchmark_test` |

## Desktop Companion Resource Footprint (`pico-desktop.exe`, release build)

Measured directly on Windows 11 by spawning `target/release/pico-desktop.exe` cleanly and sampling process memory and CPU counters:

| Metric | Legacy Monolith (`eframe/glow`) | Rebuilt Companion (`winit/softbuffer`) | Budget |
|---|---|---|---|
| Release Binary Size | `10.56 MB` | **`2.08 MB`** | Minimized |
| T+2s Working Set RSS | n/a | **`36.14 MB`** | `< 50 MB` |
| Steady-State Working Set RSS (T+12s) | `156.9 MB` (baseline T+3s) | **`41.04 MB`** | `< 50 MB` |
| Private Commit Memory | `121.0 MB` (baseline T+3s) | **`11.19 MB`** | Minimized |
| 10s Steady-State Idle CPU (Single Core %) | n/a | **`0.47%`** | `< 1.0%` |
| 10s Steady-State Idle CPU (System-Wide %) | n/a | **`0.03%`** | `< 1.0%` |
