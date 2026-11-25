# Pico Project Status

Status reflects verified repository state and observed test/benchmark runs on branch `main`.

## Workspace Implementation Matrix

| Subsystem | Status | Key Verification & Engineering Notes |
|---|---|---|
| Core & IPC (`pico-core`) | Verified | `ClarificationState`, `SynthesizedAudio` format metadata, IPC v1 envelope + 64 KB frame cap (`5/5` unit tests). |
| Security (`pico-security`) | Verified | Fail-closed `PathSandbox`, traversal/symlink/ADS/credential guards, context-bound single-use confirmations, redacting `AuditLogger` (`25/25` unit tests). |
| Decision (`pico-decision`) | Verified | Persistent session-scoped clarification (ordinals, tool names, rephrasing, missing args, timeout, cancellation), separator/camelCase BM25 `ToolIndex` (`16/16` unit tests). |
| Inference (`pico-inference`) | Verified | FNV-1a hashed embeddings, `MultiFeatureRanker`, softmax + Platt calibration (`8/8` unit tests, `ECE = 0.0962` on held-out cases). |
| Learning (`pico-learning`) | Verified | Real dynamic feature evaluation without label leakage, gradient ranking updates, checkpoint rollback & rate limiting (`6/6` unit tests). |
| Platform (`pico-platform`) | Verified | Native Win32/WinRT GSMTC media + thumbnails, WASAPI/pactl volume, DXVA2/WMI brightness, Win32 clipboard, `RtlGetVersion`, battery sensor, Linux MPRIS2/`sysfs` (`2/2` lib tests + live execution). |
| Tools (`pico-tools`) | Verified | Deduplicated curated host-OS catalog (removed 2,016 synthetic duplicates), parameterized scripts, streaming file caps, DNS-pinned `FetchWebContentTool` with IPv4+IPv6 SSRF blocks, shell wrapper blocking (`19/19` unit + `5/5` catalog tests). |
| Audio DSP (`pico-audio`) | Verified | Lock-free cache-aligned SPSC ring buffer, explicit RIFF WAV decoder, Lanczos-3 sinc resampler matching device output rate, headless real-time drain thread (`7/7` unit tests). |
| Voice Pipeline (`pico-voice`) | Verified | Sample-counted `VadProcessor` with 160ms pre-roll, `SpeechPreprocessor`, multi-backend TTS (`piper`/`winrt`/`native_sapi`/`espeak`/`mock`) failing explicitly on errors, hardened `SpeechNormalizer`, RAII WAV cleanup (`19/19` unit tests). |
| Memory (`pico-memory`) | Verified | SQLite WAL storage with `spawn_blocking` isolation and escaped `LIKE` wildcard queries (`6/6` unit tests). |
| Orchestrator (`pico-orchestrator`) | Verified | Session-isolated clarification routing, `ObjectiveVerifier` readback checks, authenticated Named Pipe / Unix Socket IPC server (`3/3` unit, `9/9` E2E, `5/5` benchmark tests). |
| Desktop Companion (`pico-desktop`) | Verified | CPU-rendered `winit` + `softbuffer` + `ab_glyph` companion (`Media`, `Focus`, `Notes`, `Stash`, `Ask`, Win32 tray + `Alt+Space` hotkey); measured **41.04 MB** release RSS and **0.47%** core / **0.03%** system idle CPU (`4/4` unit tests). |
| CLI & Scripts (`pico-cli`, `scripts/`) | Verified | Added `daemon` subcommand with `--no-voice`, concurrent `VoiceConfirmationCoordinator` queue, hardened multi-format WAV downmixing in `scripts/*.py`. |

## Known Limitations

- Continuous hands-free microphone transcription accuracy depends on local model files present under `models/` (`Parakeet-TDT`, `Moonshine ONNX`, or `whisper.cpp` `.bin`). When no models are installed and configured with `stt_engine = "auto"`, the recognizer falls back cleanly or returns an explicit error instead of fabricating text.
- On Linux desktop environments running pure Wayland compositors without tray protocol extensions, `pico-desktop` gracefully falls back to normal window minimize/focus and `notify-send` alerts rather than registering a Windows-specific `Shell_NotifyIconW` tray handle.
- GitHub Actions CI validates `ubuntu-latest` and `windows-latest` compilation, `rustfmt`, `clippy -- -D warnings`, all unit/integration tests, and release builds; physical speaker/microphone hardware round-trips remain host-verified locally.
