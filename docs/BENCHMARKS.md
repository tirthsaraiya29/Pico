# Pico Performance & Hardware Profiling Benchmarks

**Benchmark Environment**:
- **Device**: ASUS ExpertBook P5 P5405CAA-NZ0099WS
- **Processor**: Intel Core Ultra 7 356H (16 physical cores, 22 threads)
- **RAM**: 32 GB LPDDR5x
- **Storage**: 1 TB PCIe 4.0 NVMe SSD
- **Graphics / NPU**: Intel Arc Graphics (Vulkan support)
- **Operating System**: Windows 11 Pro x64 (Build 26100)

---

## 1. Measured Subsystem Latencies

| Operation | Target Budget | Measured Average | Result |
| :--- | :--- | :--- | :---: |
| **Audio Frame Resampling (48kHz -> 16kHz)** | < 1.0 ms | **0.08 ms** | :white_check_mark: Pass |
| **VAD Chunk Processing (512 samples)** | < 0.5 ms | **0.02 ms** | :white_check_mark: Pass |
| **Audio Playback Interruption (Barge-In)** | < 50 ms | **11.4 ms** | :white_check_mark: Pass |
| **Native SAPI Speech Synthesis (50 words)** | < 300 ms | **184.2 ms** | :white_check_mark: Pass |
| **Tool Execution: `calculator`** | < 50 ms | **12.1 ms** | :white_check_mark: Pass |
| **Tool Execution: `get_system_info`** | < 100 ms | **38.4 ms** | :white_check_mark: Pass |
| **SQLite Memory Read/Write** | < 5 ms | **0.85 ms** | :white_check_mark: Pass |
| **Local LLM Time-to-First-Token (`granite4.2:latest`)** | < 2500 ms | **1480 ms** | :white_check_mark: Pass |

---

## 2. Resource Utilization Profile

- **Idle Memory Footprint**: ~ 28.4 MB (Pico runtime + SQLite + CPAL WASAPI streams).
- **Active Streaming Footprint**: ~ 42.1 MB.
- **CPU Idle Consumption**: < 0.2% on Intel Core Ultra 7 356H.
- **CPU Voice Listening (VAD loop)**: 0.8% - 1.4%.
