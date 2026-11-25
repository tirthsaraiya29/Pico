# Pico Remaining Defects Register

**Baseline Commit**: `f6ded10eec5fc5cd6755243137865e052641e79f` (`main`)
**Baseline Verification**: `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` all pass on Windows (`target/debug`).
**Note**: Historical entries in [PICO_BUG_REGISTER.md](PICO_BUG_REGISTER.md) are preserved with their original audit evidence. This register tracks all remaining, partially fixed, and newly discovered defects at baseline and their verified resolution status.

## Classification Legend
- **Confirmed and reproduced**: Directly observed in source and reproduced via test/runtime.
- **Confirmed from source, runtime reproduction pending**: Directly observed in source code; runtime/hardware reproduction pending.
- **Fixed and regression-tested**: Remediated in code with deterministic regression tests passing.
- **Partially fixed**: Partial remediation exists; remaining gaps documented.
- **Deferred, with a technical justification**: Intentionally deferred with documented constraint.

---

## Remaining P2 Defects

| ID | Location | Baseline Status | Current Status | Summary of Resolution |
|---|---|---|---|---|
| `BUG-055` | `crates/pico-memory/src/budget.rs` | Confirmed and reproduced | **Fixed and regression-tested** | Replaced byte-length estimator with Unicode/CJK/emoji/camelCase-aware `estimate_tokens`; added `budget_messages_with_report` preserving `Role::System` instructions and reporting `truncated_count` & `dropped_tokens`. |
| `BUG-062` | `crates/pico-core/src/events.rs`, `crates/pico-orchestrator/src/coordinator.rs`, `crates/pico-voice/src/pipeline.rs` | Partially fixed | **Fixed and regression-tested** | Added `session_id: Option<SessionId>` across all session-attributable `PicoEvent` variants and `PicoEvent::session_id()` helper; wired `Some(sid)` in `PicoRuntime`. |

## Remaining P3 Defects

| ID | Location | Baseline Status | Current Status | Summary of Resolution |
|---|---|---|---|---|
| `BUG-073` | `crates/pico-voice/src/stt.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added 2-word and 3-word adjacent stutter collapse in `SpeechNormalizer::normalize` while preserving numeric sequences. |
| `BUG-076` | `crates/pico-tools/src/calculator_parser.rs` | Partially fixed | **Fixed and regression-tested** | Bounded `Lexer::tokenize` to `MAX_CALC_INPUT_BYTES = 8192` and `MAX_CALC_TOKENS = 512`. |
| `BUG-077` | `crates/pico-core/src/session.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Verified `MAX_TOOL_HISTORY_CHARS = 16384` truncation in `Message::new_tool_result` via `ToolResult::bounded_content_string` and added multi-byte UTF-8 regression test. |
| `BUG-080` | `crates/pico-memory/src/sqlite.rs`, `crates/pico-memory/src/manager.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added `SqliteStore::search_facts_ranked` with token-overlap relevance scoring and `decay_half_life_days` exponential recency decay wired from `MemoryConfig`. |
| `BUG-081` | `crates/pico-learning/src/poisoning.rs`, `crates/pico-learning/src/manager.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Persisted `AntiPoisoningGuard` rate-limit window atomically with checksum tamper detection (`IMP-054` completed). |
| `BUG-082` | `crates/pico-orchestrator/src/coordinator.rs`, `crates/pico-voice/src/pipeline.rs` | Partially fixed | **Fixed and regression-tested** | Added `PicoEvent::VoiceBackendStatus` and `VoicePipeline::emit_backend_status()` emitted on `PicoRuntime::new`. |
| `BUG-083` | `crates/pico-tools/src/filesystem_tools.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added `.venv`, `venv`, `__pycache__`, `models`, `.mypy_cache`, `.pytest_cache` defaults plus configurable `exclude_dirs` parameter in `SearchFilesTool`. |
| `BUG-084` | `crates/pico-core/src/error.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added `PicoErrorCode` enum, `PicoError::code()`, and `PicoError::code_str()` preserving human-readable messages. |

## Remaining P4 Defects & Backlog Items

| ID | Location | Baseline Status | Current Status | Summary of Resolution |
|---|---|---|---|---|
| `BUG-085` | `crates/pico-audio/src/resampler.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added precomputed 3-phase Lanczos-3 polyphase table for `16k -> 48k` (3x upsampling) eliminating inner-loop `sin()` calls. |
| `BUG-087` | `crates/pico-voice/src/vad.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added `compute_zcr`, `calibrate_noise_floor` (`VadCalibrationReport`), and ZCR speech-band validation. |
| `BUG-089` | `crates/pico-orchestrator/tests/benchmark_test.rs` | Partially fixed | **Fixed and regression-tested** | Exported reproducible JSON report (`pico_benchmark_report.json`) with OS, arch, family, build_mode, cpu_cores, sample_count, and measured metrics. |
| `BUG-090` | `apps/pico-cli/src/main.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Expanded `pico-cli doctor` with `--json`, audio resampling self-check, IPC/daemon ping, STT config, and TTS synthesis checks. |
| `BUG-091` | `crates/pico-tools/src/registry.rs` | Partially fixed | **Fixed and regression-tested** | Added `ToolRegistry::validate_schema` checking `type == "object"`, `properties` types/combinators, and `required` field existence. |

## Newly Discovered / Unfinished Product Defects (Goals 3, 4, 5, 9)

| ID | Location | Baseline Status | Summary |
|---|---|---|---|
| `NEW-001` | `apps/pico-desktop/src/state.rs:122` | Confirmed and reproduced | `CompanionState::default()` sets `compact_mode: false`, launching into the 368x540 full window instead of the intended compact Nusa-style floating companion. |
| `NEW-002` | `apps/pico-desktop/src/main.rs:357-383` | Confirmed from source, runtime reproduction pending | `UiAction::MediaPrev`, `MediaPlayPause`, `MediaNext`, `ToggleMute`, and `SetVolumeRatio` ignore `Result` errors or swallow failures, and execute synchronous OS/WinRT/WASAPI/subprocess calls directly on the winit event/render thread. |
| `NEW-003` | `apps/pico-desktop/src/main.rs` | Confirmed from source, runtime reproduction pending | Drag-and-drop & Stash interaction model needs verification/completion for multi-file drop, text/URL drop or documented OS limitation, drag-hover feedback, previews, search, duplicate detection, and safe handling of large/inaccessible payloads. |
