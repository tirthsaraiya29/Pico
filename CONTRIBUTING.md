# Contributing to Pico

Contributions focused on reliability, cross-platform Windows/Linux parity, audio/DSP quality, security hardening, and clean minimalist UI are welcome.

## Development Workflow

1. Fork or branch cleanly from `main`.
2. Ensure all code adheres to strict workspace quality gates before opening a PR:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

3. Validate companion scripts:

```bash
python3 -m py_compile scripts/asr_service.py scripts/whisper_daemon.py scripts/whisper_transcribe.py
node --check api/submit.js
cmp -s site/index.html docs/index.html
```

## Guidelines

- **Real Implementations Only**: Never commit placeholder tools, fake success statuses, or synthetic catalog duplicates. Every registered tool must map to a genuine execution path and handle failure cleanly.
- **Keep `pico-desktop` Lightweight**: Never add `pico-orchestrator`, `cpal`, `rusqlite`, OpenGL/GPU ICD drivers, or heavy async runtimes directly into `apps/pico-desktop`. Route assistant turns and status updates over `pico-core::ipc`.
- **Cross-Platform Parity**: Any OS-specific behavior must isolate cleanly behind `#[cfg(target_os = "windows")]` and `#[cfg(target_os = "linux")]` in `pico-platform`, with fail-closed behavior elsewhere.
- **Website Sync**: `site/index.html` and `docs/index.html` must remain byte-identical.
