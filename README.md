# Pico

Pico is a local-first, non-generative desktop assistant and utility companion written in Rust for Windows 11 and Linux.

Instead of shipping a heavy multi-gigabyte large language model or an Electron browser wrapper, Pico combines a deterministic BM25 tool retriever, multi-feature candidate ranking, Platt probability calibration, persistent session-scoped clarification, strict zero-trust execution sandboxing, real audio DSP resampling, and a 2.08 MB CPU-rendered desktop companion communicating with the assistant runtime over authenticated local IPC.

## Key Capabilities

- **Non-Generative Action Pipeline (`pico-decision`, `pico-inference`, `pico-orchestrator`)**: Classifies intents locally, retrieves relevant platform tools via BM25, ranks candidates with calibrated softmax + Platt joint executability probabilities (`95.2%` Top-1, `100.0%` Top-3 across the 21-case benchmark suite), maintains persistent session-scoped `ClarificationState`, and verifies outcomes via `ObjectiveVerifier` before reporting success.
- **Voice & Audio Pipeline (`pico-audio`, `pico-voice`)**: Uses cache-aligned lock-free SPSC ring buffers, Lanczos-3 sinc sample-rate conversion tuned to the actual audio endpoint rate, explicit RIFF WAV container decoding (never playing WAV headers as PCM), sample-counted VAD with a 160ms pre-roll buffer, multi-tier TTS (`piper`, `winrt`, `native_sapi`, `espeak`, `mock`), and local ASR (`Parakeet-TDT`, `Moonshine ONNX`, `whisper-cli`, `faster-whisper`) with `SpeechNormalizer`.
- **Verified Cross-Platform Tooling (`pico-tools`, `pico-platform`)**: Registers 11 core workspace/system tools plus curated, non-duplicated host-OS catalog tools across Windows and Linux using native OS APIs (WASAPI, DXVA2/WMI, Win32 Clipboard, `CreateToolhelp32Snapshot`, and parameterized PowerShell / Linux `sysfs`/`wpctl`/`pactl` execution).
- **Zero-Trust Security Enforcement (`pico-security`, `pico-memory`)**: Enforces fail-closed `PathSandbox` boundaries (UNC, ADS, DOS device names, sensitive dotfiles/credentials, symlink chains, traversal segments), SSRF DNS pinning via `resolve_to_addrs` and private/tunnel IPv4+IPv6 blocking, context-bound single-use confirmations (`request_id`, session, tool, SHA-256 argument digest, expiry), escaped SQLite `LIKE` queries, and regex-redacted audit logs.
- **Lightweight Desktop Companion & Local IPC (`pico-desktop`, `pico-cli daemon`)**: Separates the headless assistant runtime (`pico-cli daemon`) from the thin CPU-rendered companion window (`pico-desktop`, `winit` + `softbuffer` + `ab_glyph`). Measured on Windows 11 release builds: **41.04 MB** steady-state Working Set RSS, **11.19 MB** private commit, **0.47%** single-core (`0.03%` system-wide) idle CPU, and **2.08 MB** executable size.

## Architecture Overview

```text
apps/
  pico-cli/           Headless voice loop, diagnostics, and authenticated local IPC daemon
  pico-desktop/       Lightweight CPU-rendered companion (Media, Focus, Notes, Stash, Ask)
crates/
  pico-core/          Config, errors, session/clarification state, traits, and IPC wire framing
  pico-audio/         CPAL capture/playback, lock-free SPSC ring buffer, WAV decoder, Lanczos-3 DSP
  pico-voice/         Sample-counted VAD, SpeechPreprocessor, multi-backend TTS, STT + SpeechNormalizer
  pico-inference/     FNV-1a hashed embeddings, MultiFeatureRanker, Platt calibration
  pico-decision/      Intent classification, BM25 ToolIndex, argument extraction, ClarificationState
  pico-learning/      SQLite experience store, held-out evaluator, checkpoint trainer + poisoning guard
  pico-security/      PathSandbox, InputSanitizer, AuditLogger, Policy + VoiceAuthorizer
  pico-platform/      Native Win32/WinRT + Linux media (GSMTC/MPRIS), audio, display, clipboard, battery
  pico-tools/         AST calculator, streaming filesystem, DNS-pinned fetch, curated OS catalog
  pico-memory/        SQLite WAL storage with escaped LIKE search and token budgeting
  pico-orchestrator/  PicoRuntime coordinator, ObjectiveVerifier, and authenticated local IPC server
```

## Quick Start

### Prerequisites

- Rust stable toolchain (`1.88+` with `clippy` and `rustfmt`)
- **Linux**: `libasound2-dev` (plus `espeak-ng` or `piper`, and optionally `playerctl` / `notify-send`)
- **Windows 11**: MSVC build tools (`cargo build --release`), optional Python 3.10+ with `sherpa-onnx` / `faster-whisper` and local models under `models/` for offline neural ASR

### Build and Run

```bash
# Run full formatting, Clippy (-D warnings), and workspace test suite
cargo test --workspace --all-features

# Build optimized release binaries
cargo build --workspace --release

# Start the headless assistant runtime + authenticated local IPC server
./target/release/pico-cli daemon

# In another terminal, launch the lightweight floating companion (Alt+Space to toggle)
./target/release/pico-desktop
```

On Windows, double-click `Run-Pico.bat` or run:

```powershell
cargo build --release -p pico-cli -p pico-desktop
.\Run-Pico.bat
```

### CLI Commands

```text
pico-cli voice         Start hands-free microphone voice loop
pico-cli daemon        Run assistant runtime + authenticated local IPC endpoint
pico-cli daemon --no-voice
                       Run IPC assistant endpoint without microphone capture
pico-cli tools         List verified active host tools and permission tiers
pico-cli devices       Inspect audio input/output devices
pico-cli doctor        Verify host OS, audio devices, security policy, and core subsystems
pico-cli learn status  Inspect continuous-learning observation dataset & checkpoint metrics
pico-cli init-config   Generate default configs/pico.toml
```

## Configuration Notes

`configs/pico.toml` selects `"auto"` for both `stt_engine` and `tts_engine` by default:

- **TTS priority (`tts_engine = "auto"`)**: `piper` (if `piper` binary + `.onnx` model exist under `models/piper/` or `VoiceConfig`), then **WinRT OneCore (`winrt`) / SAPI5 (`native_sapi`)** on Windows or **`espeak-ng` / `espeak`** on Linux. Tests can set `"mock"`.
- **STT priority (`stt_engine = "auto"`)**: Checks persistent local ASR service (`scripts/asr_service.py` with `parakeet` or `moonshine` models in `models/`), then `whisper-cli` + `.bin` model in `models/whisper/`, then one-shot ASR script execution. Explicit binary/model overrides (`piper_binary`, `piper_model`, `whisper_binary`, `whisper_model`, `python_binary`) are optional in `[voice]`.

## Documentation

- [System Architecture](docs/ARCHITECTURE.md)
- [Audio & Voice Pipeline](docs/VOICE_PIPELINE.md)
- [Tools & Host Verification Guide](docs/TOOLS_GUIDE.md)
- [Measured Benchmarks](docs/BENCHMARKS.md)
- [Security Policy](SECURITY.md)
- [Contributing Guide](CONTRIBUTING.md)
- [Project Status](PROJECT_STATUS.md)

## License

Pico is open-source software licensed under the GNU General Public License v3.0 (`GPL-3.0-only`). See [LICENSE](LICENSE).

---

## Website

[Visit my website](https://tirthsaraiya.vercel.app)
