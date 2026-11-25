# Pico System Architecture

Pico is structured as a modular Rust workspace split cleanly into a headless assistant runtime (`pico-cli daemon`) and an isolated ultra-light desktop companion (`pico-desktop`).

## End-to-End Request Lifecycle

```text
Microphone (CPAL) -> Lock-Free SPSC Ring Buffer -> AudioConverter (Lanczos-3 -> 16 kHz mono)
  -> VadProcessor (sample-counted clock + 160ms pre-roll)
  -> SpeechRecognizer (Parakeet-TDT / Moonshine / whisper-cli -> SpeechNormalizer)
  -> PicoRuntime::process_turn_in_session(session_id, text)
     ├─ Active ClarificationState check (ordinals, option/tool selection, rephrase, missing arg, cancel)
     ├─ IntentClassifier + BM25 ToolIndex retrieval (host-OS filtered)
     ├─ MultiFeatureRanker + Softmax * Platt Calibrator
     ├─ ArgumentExtractor + Schema/Missing-Arg validation
     ├─ SecurityPolicy + ChannelConfirmationManager (session + tool + args SHA-256 + expiry)
     ├─ Tool Execution (native platform API or parameterized script)
     ├─ ObjectiveVerifier (post-action readback verification when observable)
     ├─ MemoryStore (SQLite WAL) + ExperienceStore (Continuous Learning)
     └─ ResponseSynthesizer -> SpeechPreprocessor -> SpeechSynthesizer -> AudioPlayback
```

## Process Isolation Architecture

Earlier prototypes embedded `PicoRuntime`, `AsrDaemon`, `cpal`, SQLite, and OpenGL (`eframe/glow`) inside the desktop window binary, causing OpenGL ICD user-mode driver DLLs (`igxe3icd64.dll`, `igc64.dll`) and runtime state to consume **156.9 MB** RSS.

The current architecture splits concerns cleanly:

1. **Assistant Runtime Daemon (`pico-cli daemon`)**: Owns `PicoRuntime`, SQLite databases, audio capture/playback streams, ASR worker processes, and tool execution. Exposes an authenticated local IPC server (`crates/pico-orchestrator/src/ipc_server.rs`).
2. **Desktop Companion (`apps/pico-desktop`)**: Links only `pico-core` and `pico-platform` alongside `winit`, `softbuffer`, `ab_glyph`, and `image`. Communicates with the daemon using blocking per-request frames in `pico_core::ipc::send_ipc_request_sync`, uses `ControlFlow::Wait` when idle, and renders pixels directly in CPU memory (`41.04 MB` steady-state Working Set RSS on Windows 11).

## Authentication & Transport Security (Local IPC)

- **Windows**: Named Pipe (`\\.\pipe\pico-assistant-v1-<user>`) configured with `first_pipe_instance(true)` on startup, `reject_remote_clients(true)`, and an Owner+SYSTEM SDDL security descriptor (`D:(A;;GA;;;OW)(A;;GA;;;SY)`).
- **Linux**: Unix Domain Socket (`$XDG_RUNTIME_DIR/pico-assistant-v1.sock`) with `0o600` file permissions and `SO_PEERCRED` effective-UID enforcement.
- **Message Framing**: Newline-delimited JSON capped at `65,536` bytes, checked for `protocol == 1` and constant-time equality with the per-daemon random token written to a user-private file on daemon startup.
