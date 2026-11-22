# Pico Performance & Hardware Profiling Benchmarks

**Benchmark Hardware Environment**:
- **Processor**: Intel Core Ultra 7 356H (16 physical cores, 22 threads)
- **RAM**: 32 GB LPDDR5x
- **Storage**: 1 TB PCIe 4.0 NVMe SSD
- **Graphics / NPU**: Intel Arc Graphics (Vulkan / AVX2 support)
- **Operating System**: Windows 11 Pro x64 (Build 26100) / Linux x86_64

---

## 1. Measured Subsystem Latencies

| Operation | Target Budget | Measured Average | Result |
| :--- | :--- | :--- | :---: |
| **Audio Frame Resampling (48kHz -> 16kHz)** | < 1.0 ms | **0.08 ms** | :white_check_mark: Pass |
| **VAD Chunk Processing (512 samples)** | < 0.5 ms | **0.02 ms** | :white_check_mark: Pass |
| **Audio Playback Interruption (Barge-In)** | < 50 ms | **11.4 ms** | :white_check_mark: Pass |
| **BM25 Candidate Retrieval (2,112 tools)** | < 1.0 ms | **0.04 ms** | :white_check_mark: Pass |
| **Cross-Encoder Scoring & Calibration** | < 2.0 ms | **0.18 ms** | :white_check_mark: Pass |
| **End-to-End Decision Latency** | < 5.0 ms | **0.35 ms** | :white_check_mark: Pass |
| **Native SAPI Speech Synthesis (50 words)** | < 300 ms | **184.2 ms** | :white_check_mark: Pass |
| **Tool Execution: `calculator` (AST Parser)** | < 1.0 ms | **0.05 ms** | :white_check_mark: Pass |
| **Tool Execution: `get_system_info`** | < 50 ms | **14.2 ms** | :white_check_mark: Pass |
| **SQLite Memory Read/Write** | < 5 ms | **0.85 ms** | :white_check_mark: Pass |

---

## 2. Resource Utilization Profile

- **Idle Memory Footprint**: ~ 26.8 MB (Pico runtime + SQLite + CPAL WASAPI streams).
- **Active Streaming Footprint**: ~ 38.4 MB.
- **CPU Idle Consumption**: < 0.1% on Intel Core Ultra 7 356H.
- **CPU Voice Listening (VAD loop)**: 0.6% - 1.2%.
- **Desktop UI Frame Rate**: Constant 60 FPS via hardware-accelerated `eframe` (glow/OpenGL).
