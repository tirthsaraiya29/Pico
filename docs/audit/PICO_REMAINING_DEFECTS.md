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
| `BUG-062` | `crates/pico-core/src/events.rs`, `crates/pico-orchestrator/src/coordinator.rs`, `crates/pico-voice/src/pipeline.rs` | Partially fixed | **Fixed and regression-tested** | Added `session_id: Option<SessionId>` across all session-attributable `PicoEvent` variants and `PicoEvent::session_id()` helper; wired `Some(sid)` in `PicoRuntime` and added `speak_in_session`, `transcribe_in_session`, `interrupt_playback_in_session`, and `start_listening_in_session` in `VoicePipeline` with concurrent-session regression test. |

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

| ID | Location | Baseline Status | Current Status | Summary of Resolution |
|---|---|---|---|---|
| `NEW-001` | `apps/pico-desktop/src/state.rs`, `apps/pico-desktop/src/main.rs` | Confirmed and reproduced | **Fixed and regression-tested** | Set `compact_mode: true` as default launch experience (`--expanded` flag added); redesigned compact floating card (`368x102`) with prominent album art, dynamic gradient, track progress, Prev/Play-Pause/Next, mute toggle, active timer/Pomodoro badge, and smooth height animation. |
| `NEW-002` | `apps/pico-desktop/src/main.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Moved all `PlatformMedia`, `PlatformAudio`, `PlatformClipboard`, and `PlatformApps` calls off the winit event thread onto `spawn_platform_action_worker`; eliminated ignored `Result` values, reporting errors via toasts and updating UI state only from actual OS readbacks. |
| `NEW-003` | `apps/pico-desktop/src/main.rs`, `apps/pico-desktop/src/state.rs` | Confirmed from source, runtime reproduction pending | **Fixed and regression-tested** | Added `HoveredFile`/`HoveredFileCancelled` drag-hover visual feedback, multi-file drop support, stash search, size/URL/text previews, duplicate promotion, 64 KB clipboard cap, and missing-file detection. (Note: winit 0.30 `WindowEvent` exposes file drops only; plain-text/URL capture uses clipboard import.) |
| `NEW-004` | `crates/pico-decision/src/argument.rs:59-61,275-277` | Confirmed and reproduced | **Fixed and regression-tested** | `resolve_arguments` unconditionally inserted `"expression"` even when `extract_math_expression` yielded non-numeric prose (e.g. `"or"` from `"calculate or check specs"`), bypassing `MissingArguments` slot-filling after candidate selection. Guarded insertion on `expr.chars().any(|c| c.is_ascii_digit())`. |
| `NEW-005` | `apps/pico-desktop/src/main.rs` (`render` palette lerp) | Confirmed and reproduced | **Fixed and regression-tested** | `Rgb::lerp(..., 0.35)` with `u8` rounding hit a fixed-point when channel delta was `<= 2`, leaving `active_palette != target_palette` forever. Snapped to `target_palette` when a step makes no progress. |
| `NEW-006` | `apps/pico-desktop/src/main.rs` (`about_to_wait`) | Confirmed and reproduced | **Fixed and verified in release** | `about_to_wait` called `self.request_redraw()` unconditionally on every loop iteration when media was playing or clocks were running, causing a 75% core CPU busy-loop. Gated redraws on `last_tick_redraw` (1s) and `last_anim_redraw` (20ms). |
