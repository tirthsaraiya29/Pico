# PICO

<div align="center">

**Autonomous, Real-Time, Locally Capable AI Voice Assistant in Rust**

[![CI](https://github.com/tirthsaraiya/Pico/actions/workflows/ci.yml/badge.svg)](https://github.com/tirthsaraiya/Pico/actions/workflows/ci.yml)
[![CodeQL](https://github.com/tirthsaraiya/Pico/actions/workflows/codeql.yml/badge.svg)](https://github.com/tirthsaraiya/Pico/actions/workflows/codeql.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Windows%2011%20%7C%20Linux-lightgrey.svg)](https://github.com/tirthsaraiya/Pico)

*Extremely low latency. Zero-trust OS execution. Native hardware audio pipeline. Fully local.*

[Key Features](#key-features) •
[Architecture](#architecture) •
[Quickstart](#quickstart) •
[Interactive 3D UI](#desktop--3d-web-interface) •
[Benchmarks](#hardware-benchmarks) •
[Documentation](docs/ARCHITECTURE.md)

</div>

---

## Overview

**Pico** is a production-quality, real-time AI assistant engineered from the ground up in idiomatic Rust. Designed for responsive human-machine voice communication, Pico operates locally on your hardware without transmitting telemetry, audio recordings, or conversational transcripts to external servers.

Unlike simple wrappers around cloud LLM APIs, Pico features a custom async runtime, low-latency audio capture and playback with sub-15ms barge-in interruption, a zero-trust OS tool execution framework, persistent SQLite memory with context budgeting, and interchangeable local inference backends (Ollama, Vulkan-accelerated `llama-server`, or OpenAI-compatible streaming endpoints).

---

## Key Features

- **Microphone Pipeline & WASAPI Capture**: Direct native device audio streaming, downmixed and resampled on-the-fly to 16,000 Hz 16-bit mono PCM.
- **Adaptive Energy Voice Activity Detection (VAD)**: Dynamically adjusts to background noise floors ($\alpha = 0.05$) to discern speech from ambient room noise.
- **Sub-15ms Barge-In Interruption**: Instantly halts playback ring buffers and cancels model generation the millisecond user speech is detected.
- **Zero-Download Native SAPI & Piper TTS**: Ultra-fast Windows SAPI speech synthesis (< 185ms time-to-audio) with pluggable neural Piper TTS options.
- **Interchangeable Inference Engine**:
  - Native local Ollama API integration with streaming token delivery.
  - Vulkan-accelerated `llama.cpp` GGUF runner (`llama-server.exe`).
  - Remote OpenAI-compatible endpoints with backoff retries and structured decision recovery.
- **Zero-Trust Tool Execution Framework**:
  - 4 security tiers: `ReadOnly`, `LowRisk`, `ConfirmationRequired`, and `Restricted`.
  - Canonical path sandbox preventing directory traversal (`..`, symlinks, junction points) and protecting credentials (`.ssh`, `.aws`, `System32`).
  - Pre-registered system tools: application launcher, process manager, system hardware inspector, file reader/writer/searcher, calculator, timers, clipboard, and web content reader.
- **Long-Term Memory & Context Budgeting**:
  - Persistent SQLite storage with Write-Ahead Logging (WAL).
  - BM25 keyword relevance scoring and automatic context budgeting to prevent token window overflow.
- **High-Performance CLI & Modern 3D Desktop UI**:
  - Interactive terminal REPL with colored tool audits and system health doctor (`pico doctor`).
  - Real-time Axum/WebSocket desktop interface featuring a responsive WebGL 3D neural sphere in Three.js, live VU meters, and oscilloscope.

---

## Architecture

```
Pico/
├── crates/
│   ├── pico-core/           # Domain models, session structs, config schemas, universal error
│   ├── pico-security/       # Multi-tiered policy engine, path sandbox, audit logging, input sanitizer
│   ├── pico-platform/       # Windows native integrations (apps, processes, clipboard, sysinfo)
│   ├── pico-inference/      # Ollama, LlamaCpp, and OpenAI-compatible streaming provider implementations
│   ├── pico-tools/          # Tool registry, JSON Schema generation, OS tools, timeout & cancellation
│   ├── pico-audio/          # CPAL audio capture/playback, polyphase 16kHz resampler, thread-safe handles
│   ├── pico-voice/          # Adaptive VAD, Windows Speech STT, native SAPI TTS, barge-in coordinator
│   ├── pico-memory/         # SQLite persistent storage, context window budgeter, relevance ranking
│   └── pico-orchestrator/   # Central runtime coordinating event bus, multi-turn reasoning, and tools
├── apps/
│   ├── pico-cli/            # Terminal interface (chat, voice, doctor, tools, devices)
│   └── pico-desktop/        # Axum WebSocket server, static asset host, Three.js 3D landing interface
├── configs/
│   └── pico.toml            # Central system configuration
├── docs/                    # Architectural specs, voice pipeline, benchmarks, tools guide
├── tests/                   # Integration and end-to-end test suites
├── LICENSE                  # GNU General Public License v3.0
├── SECURITY.md              # Zero-trust security policy & vulnerability disclosure
└── CONTRIBUTING.md          # Contribution standards & development setup
```

---

## Quickstart

### Prerequisites

- **Rust**: Version 1.80+ (MSVC toolchain on Windows or GCC/Clang on Linux).
- **C++ Build Tools**: Visual Studio 2022+ C++ Build Tools (Windows) or `build-essential` (Linux).
- **Local Inference (Recommended)**: [Ollama](https://ollama.com) running locally with any tool-capable model (e.g. `ollama run granite4.2` or `llama3.2`).

### 1. Clone & Build

```bash
git clone https://github.com/tirthsaraiya/Pico.git
cd Pico

# Run unit and integration tests
cargo test --workspace --all-features

# Build release binaries
cargo build --workspace --release
```

### 2. Run Diagnostic Health Check

Verify your hardware, audio devices, native speech synthesis, SQLite database, and local model inference:

```bash
.\target\release\pico-cli.exe doctor
```

```
==================================================
           PICO DIAGNOSTIC SYSTEM DOCTOR          
==================================================

[1/5] Checking Platform & Environment...
  [OK] Host OS: Windows 11 x64
  [OK] CPU Cores: 16
  [OK] Total Memory: 31963 MB (Available: 19363 MB)

[2/5] Checking Audio Capture & Playback Subsystem...
  [OK] Microphones found: 1
  [OK] Audio output devices found: 1

[3/5] Testing Native Speech Synthesis...
  [OK] Native SAPI generated 91840 PCM audio bytes

[4/5] Testing SQLite Persistent Memory...
  [OK] SQLite database initialization and WAL journaling verified

[5/5] Checking Local Inference Provider (Ollama / Llama.cpp)...
  [OK] Ollama service reachable at http://127.0.0.1:11434
  [OK] Target model configured: granite4.2:latest

Diagnostic pass completed.
```

### 3. Launch Interactive Terminal Chat

```bash
# Chat in terminal with tool calling
.\target\release\pico-cli.exe chat

# Chat with spoken voice responses enabled
.\target\release\pico-cli.exe chat --speak
```

### 4. Hands-Free Voice Mode

Continuous microphone capture with real-time VAD and barge-in interruption:

```bash
.\target\release\pico-cli.exe voice
```

### 5. Launch Real-Time 3D Desktop UI

```bash
.\target\release\pico-desktop.exe
```

Access the desktop interface at `http://127.0.0.1:3124` to interact with the Three.js 3D neural core, audio oscilloscope, live metrics, and settings drawer.

---

## Desktop & 3D Web Interface

The Desktop interface connects directly to the Pico runtime via low-latency WebSockets (`/ws`):
- **Dynamic 3D Neural Core**: Visualizes assistant state (`IDLE`, `LISTENING`, `THINKING`, `TOOL_EXECUTION`, `SPEAKING`) with smooth geometric morphing and mouse raycast tracking.
- **Audio Radar & Oscilloscope**: Real-time canvas waveform and VAD sensitivity VU meter.
- **Live Tool Cards**: Inspect parameter inputs, execution times, and return values in real time.
- **Configuration Drawer**: Switch inference backends, adjust VAD thresholds, inspect audio devices, and manage security tiers.

---

## Hardware Benchmarks

Measured on **Intel Core Ultra 7 356H** (16 cores), 32 GB RAM, Intel Arc Graphics, Windows 11 x64:

| Metric | Target Budget | Measured Average |
| :--- | :--- | :--- |
| **Microphone Resampling (48kHz -> 16kHz)** | < 1.0 ms | **0.08 ms** |
| **VAD Frame Calculation (512 samples)** | < 0.5 ms | **0.02 ms** |
| **Barge-In Playback Cancellation** | < 50 ms | **11.4 ms** |
| **Native SAPI Speech Synthesis (50 words)** | < 300 ms | **184.2 ms** |
| **Tool Execution (`calculator`)** | < 50 ms | **12.1 ms** |
| **SQLite Memory Transaction** | < 5 ms | **0.85 ms** |
| **Local Model TTFT (`granite4.2:latest`)** | < 2500 ms | **1480 ms** |
| **Idle Memory Consumption** | < 50 MB | **28.4 MB** |

---

## Tool Execution & Safety

Pico validates all LLM tool requests against an extensible registry:

```bash
.\target\release\pico-cli.exe tools
```

| Tool | Permission Tier | Description |
| :--- | :---: | :--- |
| `get_system_info` | `ReadOnly` | OS, CPU cores, RAM statistics, battery status |
| `list_processes` | `ReadOnly` | Active processes sorted by working memory |
| `kill_process` | `ConfirmationRequired` | Terminates process by PID |
| `open_application` | `LowRisk` | Launches applications (`notepad`, `calc`, browser) |
| `read_file` | `ReadOnly` | Reads files within allowed sandbox boundaries |
| `write_file` | `ConfirmationRequired` | Writes files within allowed sandbox boundaries |
| `search_files` | `ReadOnly` | Recursive pattern match within workspace |
| `calculator` | `ReadOnly` | Evaluates arithmetic expressions safely |
| `manage_timer` | `LowRisk` | Sets countdown timers and voice reminders |
| `clipboard` | `LowRisk` | Reads or writes operating system clipboard |
| `fetch_web_content` | `ReadOnly` | HTTP/HTTPS text reader with SSRF guards |
| `execute_shell_command` | `ConfirmationRequired` | Validated command executor with argument sanitization |

---

## Configuration (`configs/pico.toml`)

```toml
[inference]
provider = "ollama"                         # "ollama", "llamacpp", "openai"
endpoint = "http://127.0.0.1:11434"
model = "granite4.2:latest"
temperature = 0.2
context_window = 8192
timeout_seconds = 180

[voice]
vad_threshold = 0.015                       # VAD energy sensitivity
silence_duration_ms = 700                   # Silence required to finalize utterance
stt_engine = "whisper"
tts_engine = "native_sapi"
speech_rate = 1.0
enable_barge_in = true                      # Instant interruption on speech

[security]
policy_mode = "ConfirmationPrompt"          # "Strict", "ConfirmationPrompt", "Permissive"
allowed_directories = ["."]
blocked_directories = ["C:\\Windows", "C:\\Windows\\System32"]
require_confirmation_for_shell = true
audit_log_path = "pico_audit.log"
```

---

## Security Policy

Security is a primary design pillar. Pico adheres to a **Zero-Trust Principle for Model Outputs**:
- All tool parameters and file paths are canonicalized and boundary-checked.
- Traversal attempts (`..`, hidden dotfiles, `.ssh`, `.aws`, credential stores) are immediately blocked.
- Commands are sanitized against injection metacharacters (`|`, `&`, `;`, `$()`, backticks).
- Destructive actions require interactive confirmation.
- Audit records are appended locally with sensitive keys masked.

See [SECURITY.md](SECURITY.md) for full vulnerability disclosure procedures.

---

## License

This project is licensed under the **GNU General Public License v3.0** — see the [LICENSE](LICENSE) file for details.
