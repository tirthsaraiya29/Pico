# Pico Project Status

Status reflects verified repository state and observed test/benchmark runs on branch `main`.

## Workspace Implementation Matrix

| Subsystem | Status | Key Verification & Engineering Notes |
|---|---|---|
| Core & IPC (`pico-core`) | Verified | `PicoErrorCode` machine-readable error codes, `session_id` attribution across `PicoEvent` variants, `ClarificationState`, bounded `ToolResult` history truncation, `SynthesizedAudio` format metadata, IPC v1 envelope + 64 KB frame cap (`6/6` unit tests). |
| Security (`pico-security`) | Verified | Fail-closed `PathSandbox`, traversal/symlink/ADS/credential guards, context-bound single-use confirmations, redacting `AuditLogger` (`25/25` unit tests). |
| Decision (`pico-decision`) | Verified | Persistent session-scoped clarification (ordinals, tool names, rephrasing, missing args, timeout, cancellation), separator/camelCase BM25 `ToolIndex` (`16/16` unit tests). |
| Inference (`pico-inference`) | Verified | FNV-1a hashed embeddings, `MultiFeatureRanker`, softmax + Platt calibration (`8/8` unit tests, `ECE = 0.0962` on held-out cases). |
| Learning (`pico-learning`) | Verified | Real dynamic feature evaluation without label leakage, gradient ranking updates, checkpoint rollback & persisted tamper-checked rate limiting (`7/7` unit tests). |
| Platform (`pico-platform`) | Verified | Native Win32/WinRT GSMTC media + thumbnails, WASAPI/pactl volume, DXVA2/WMI brightness, Win32 clipboard, `RtlGetVersion`, battery sensor, Linux MPRIS2/`sysfs` (`2/2` lib tests + live execution). |
| Tools (`pico-tools`) | Verified | Deduplicated curated host-OS catalog, parameterized scripts, streaming file caps, configurable `SearchFilesTool` exclusions (`.venv`, `__pycache__`, `models`), bounded calculator lexer, `ToolRegistry::validate_schema`, DNS-pinned `FetchWebContentTool` with IPv4+IPv6 SSRF blocks (`22/22` unit + `5/5` catalog tests). |
| Audio DSP (`pico-audio`) | Verified | Lock-free cache-aligned SPSC ring buffer, explicit RIFF WAV decoder, Lanczos-3 sinc + 3x polyphase resampler matching device output rate, persistent COM STA worker (`host_com.rs`), headless real-time drain thread (`7/7` unit tests). |
| Voice Pipeline (`pico-voice`) | Verified | Sample-counted `VadProcessor` with 160ms pre-roll + ZCR/noise-floor calibration, `SpeechPreprocessor`, multi-backend TTS (`piper`/`winrt`/`native_sapi`/`espeak`/`mock`) failing explicitly on errors, multi-word stutter `SpeechNormalizer`, `VoiceBackendStatus` diagnostics, session-attributed `speak_in_session`/`transcribe_in_session`/`interrupt_playback_in_session`/`start_listening_in_session`, and `VoicePipeline::shutdown()` (`22/22` unit tests). |
| Memory (`pico-memory`) | Verified | SQLite WAL storage with `spawn_blocking` isolation, escaped `LIKE` wildcard queries, relevance-ranked search with `decay_half_life_days`, Unicode/CJK/emoji-aware `ContextBudgeter` with system preservation & truncation reporting (`8/8` unit tests). |
| Orchestrator (`pico-orchestrator`) | Verified | Session-isolated clarification routing (all selection, missing-arg, cancel, timeout, unavailable/failed tool paths), `PicoRuntime::shutdown()` (cancels turns, stops voice workers/playback, clears clarifications), `ObjectiveVerifier` readback checks, authenticated Named Pipe / Unix Socket IPC server (`3/3` unit, `11/11` E2E, `5/5` benchmark tests with JSON export). |
| Desktop Companion (`pico-desktop`) | Verified | Compact-first (`368x102` default, `--expanded` / smooth toggle) CPU-rendered `winit` + `softbuffer` + `ab_glyph` companion with non-blocking background `PlatformCommand` worker, explicit error toasts, drag-hover + multi-file stash + search/previews; measured over 10s release run: **40.63 MB** median / **40.68 MB** peak RSS and **0.306%** core / **0.019%** system idle CPU (`5/5` unit tests). |
| CLI & Scripts (`pico-cli`, `scripts/`) | Verified | `daemon` subcommand with `--no-voice`, expanded `doctor --json` (audio resampling, IPC/daemon ping, STT/TTS checks), concurrent `VoiceConfirmationCoordinator` queue, hardened multi-format WAV downmixing in `scripts/*.py`. |

## Known Limitations

- Continuous hands-free microphone transcription accuracy depends on local model files present under `models/` (`Parakeet-TDT`, `Moonshine ONNX`, or `whisper.cpp` `.bin`). When no models are installed and configured with `stt_engine = "auto"`, the recognizer falls back cleanly or returns an explicit error instead of fabricating text.
- On Linux desktop environments running pure Wayland compositors without tray protocol extensions, `pico-desktop` gracefully falls back to normal window minimize/focus and `notify-send` alerts rather than registering a Windows-specific `Shell_NotifyIconW` tray handle.
- GitHub Actions CI validates `ubuntu-latest` and `windows-latest` compilation, `rustfmt`, `clippy -- -D warnings`, all unit/integration tests, and release builds; physical speaker/microphone hardware round-trips remain host-verified locally.
