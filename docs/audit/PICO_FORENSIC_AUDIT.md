# Pico Forensic Audit

**Audit Baseline Date**: 2026-11-25
**Repository**: `https://github.com/tirthsaraiya29/Pico.git`
**Target Branch**: `main`
**Audited Base Commit**: `faf1a5a8da856a2b88eceb188e92aa1d32e401d9` (matches `origin/main` before audit fixes)
**Audit Scope**: All 11 workspace crates (`crates/*`), 2 applications (`apps/*`), 3 Python helper scripts (`scripts/*`), configuration (`configs/pico.toml`), CI workflows (`.github/workflows/*`), serverless API (`api/submit.js`), build scripts (`test.bat`, `test.sh`, `Run-Pico.bat`), documentation (`README.md`, `PROJECT_STATUS.md`, `SECURITY.md`, `CONTRIBUTING.md`, `docs/*`), website (`site/index.html`, `docs/index.html`), and runtime artifacts (`data/pico_memory.sqlite`, `logs/security_audit.jsonl`).

---

## 1. Executive Summary

Pico claims to be a "production-ready", non-generative voice assistant with 2,112 native OS tools, neural cross-encoder scoring, sub-15ms voice barge-in, lock-free audio ring buffers, and a 60 FPS hardware-accelerated desktop GUI (`README.md:66-101`, `PROJECT_STATUS.md:7-46`). Forensic inspection of the source code, runtime logs, SQLite transcripts, and automated verification scripts proves that several foundational subsystems contain severe functional, mathematical, audio-engineering, and security defects:

1. **Tool Catalog Inflation (Confirmed)**: Out of 2,112 catalog entries across `crates/pico-tools/src/catalog/windows.rs` (1,056) and `crates/pico-tools/src/catalog/linux.rs` (1,056), categories 2 through 22 (21 categories x 48 actions x 2 OSes = **2,016 tools**) collapse every verb (`get`, `set`, `start`, `stop`, `restart`, `delete`, etc.) onto a single identical read-only or invalid command (`Get-Volume`, `Get-Process`, `Get-AudioDevice -Name $name`, `powercfg.exe -Name $name`, `ps aux $name`, `systemctl $name`, `apt $name`). Even Category 1 (`filesystem`) contains unused parameters, stdin-blocking commands (`tee -a`), and misclassified permissions.
2. **Broken Decision Clarification & Miscalibration (Confirmed)**:
   - `IntentClassifier::is_direct_conversation` (`crates/pico-decision/src/intent.rs:329-353`) checks `lower.contains(phrase)` for `"first"`, `"second"`, `"neither"`, `"none"`, `"cancel"`. Any user query containing `"second"` (such as `"Set a timer for 30 seconds"`) is misclassified as `DirectConversation`, and numeric/ordinal clarification answers either misfire or lose state.
   - `PicoRuntime::pending_clarification` (`crates/pico-orchestrator/src/coordinator.rs:36,243-283`) stores only a single unkeyed `Option<(String, Vec<CandidateScore>)>` globally across all sessions, drops it on mismatch via `lock.take()`, skips `missing_required` checks, skips `ObjectiveVerifier`, ignores `tool_res.success == false`, and skips memory/learning persistence.
   - `CrossEncoderRanker::rank_candidates` (`crates/pico-inference/src/ranking.rs:136-185`) applies Platt scaling (`a = 10.0, b = -5.0`) independently per candidate, allowing multiple mutually competing actions to simultaneously display 90%+ confidence (`audio.mute` 96% vs `audio.set_volume` 95%, observed in `data/pico_memory.sqlite`).
   - Historical SQLite & audit evidence proves `"Close brave"` executed `open_application("Close brave")` and announced `"Opened Close brave."` (`data/pico_memory.sqlite`, `logs/security_audit.jsonl`).
3. **Critical Audio & Voice Engineering Defects (Confirmed)**:
   - `AudioPlayback::start` (`crates/pico-audio/src/playback.rs:57-170`) opens the default OS output stream at the hardware sample rate (typically 48,000 Hz or 44,100 Hz) and drains 16,000 Hz or 22,050 Hz buffers directly without resampling, producing 2.18x to 3.0x speed/pitch "chipmunk/alien" playback.
   - `NativeLinuxSynthesizer::synthesize_sync` (`crates/pico-voice/src/tts.rs:155-190`) runs `espeak-ng --stdout` (which emits a RIFF WAV container at 22,050 Hz with a 44-byte header) and feeds the WAV header and payload straight into `play_pcm_bytes` assuming 16-bit PCM.
   - `NativeWindowsSynthesizer::synthesize_sync` (`crates/pico-voice/src/tts.rs:100-103`) returns `Ok(Vec::new())` on PowerShell exit failure, and `PlatformSpeechSynthesizer::synthesize` hardcodes the platform native synthesizer while ignoring `VoiceConfig.tts_engine`.
   - Both `AudioPlayback` (`Mutex<VecDeque<f32>>`) and `AudioCapture` (`tokio_mpsc::Sender<Vec<i16>>`) block/allocate inside real-time OS audio callbacks.
4. **Security Vulnerabilities (Confirmed)**:
   - Command injection in `PlatformApps::open_application` (`crates/pico-platform/src/apps.rs:124-144`) via `cmd /C start` and single-quoted `Start-Process '{}'`.
   - Incomplete shell escaping and `/proc` + `/sys` path traversal in `crates/pico-tools/src/catalog/adapter.rs:143,225-236`.
   - Symlink traversal in `SearchFilesTool::walk` (`crates/pico-tools/src/filesystem_tools.rs:233-252`), empty-roots bypass and trailing `..` / TOCTOU gaps in `PathSandbox` (`crates/pico-security/src/sandbox.rs:96,175`), DNS-rebinding TOCTOU and unbounded body allocation in `FetchWebContentTool` (`crates/pico-tools/src/utility_tools.rs:303-453`), and permissive-mode / confirmation replay gaps in `SecurityPolicy` / `ChannelConfirmationManager` (`crates/pico-security/src/policy.rs:26-100`).
5. **Heavyweight Monolithic Desktop App (Confirmed)**:
   - `apps/pico-desktop/src/main.rs` instantiates the full `PicoRuntime`, SQLite stores, tool catalog index, and Python ONNX `AsrDaemon` inside the GUI process, triggers `ctx.request_repaint_after(16ms)` continuously (`apps/pico-desktop/src/main.rs:226`), polls `PlatformSysInfo::get_system_stats()` on the UI thread (`:235`), and implements a non-functional `"Mic: PAUSED"` toggle (`:815-817`) that never mutes capture.

---

## 2. Repository, Git State & Pre-Existing Local Work

### 2.1 Git Baseline Inspection
- **HEAD**: `faf1a5a8da856a2b88eceb188e92aa1d32e401d9` (synchronized with `origin/main`).
- **Pre-existing dirty working tree (preserved per engineering rules)**:
  - Modified tracked files (`17`):
    `apps/pico-desktop/src/main.rs`, `configs/pico.toml`, `crates/pico-core/src/config.rs`, `crates/pico-decision/src/argument.rs`, `crates/pico-decision/src/engine.rs`, `crates/pico-decision/src/index.rs`, `crates/pico-decision/src/intent.rs`, `crates/pico-decision/src/synthesizer.rs`, `crates/pico-inference/src/ranking.rs`, `crates/pico-orchestrator/src/coordinator.rs`, `crates/pico-orchestrator/tests/e2e_turn_test.rs`, `crates/pico-platform/src/apps.rs`, `crates/pico-tools/src/audio_tools.rs`, `crates/pico-tools/src/catalog/windows.rs`, `crates/pico-tools/src/system_tools.rs`, `crates/pico-voice/src/stt.rs`, `crates/pico-voice/src/vad.rs`.
  - Untracked files/directories:
    - `Run-Pico.bat` (root desktop launcher script).
    - `scripts/` (`asr_service.py`, `whisper_daemon.py`, `whisper_transcribe.py`).
    - `models/` (`moonshine-tiny-en-int8/` ~119 MB total; `parakeet-tdt-0.6b-v2-int8/` ~631 MB including 622.06 MB `encoder.int8.onnx`). Note: `.gitignore` currently ignores `models/downloads/` only (`line 21`), risking accidental check-in of 750+ MB ONNX weights.
    - `pico-desktop.exe` (untracked built executable at repository root; `.gitignore` does not ignore root `*.exe` binaries).

### 2.2 Baseline Toolchain, Lint & Test Verification
All checks below were executed on Windows 11 (`rustc 1.100.0`, `cargo 1.100.0`, `clippy 0.1.100`, `Python 3.12.10`):

| Check | Outcome | Exact Findings |
|---|---|---|
| `cargo fmt --all -- --check` | Failed (`4 diffs`) | Formatting mismatches in `apps/pico-desktop/src/main.rs:242,1037,1059,1085`. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Failed (`5 warnings`) | `argument.rs:471,489` (`manual_strip`); `coordinator.rs:36` (`type_complexity`); `apps/pico-desktop/src/main.rs:245` (`unnecessary_map_or`), `:823` (`collapsible_if`). CI runs with `-D warnings` (`.github/workflows/ci.yml:50`), so `main` + local changes would fail CI. |
| `cargo test --workspace` | Hung at process exit | Individual unit test binaries through `pico-voice` passed, while `pico-orchestrator` `test_e2e_app_launch_and_volume` (`crates/pico-orchestrator/tests/e2e_turn_test.rs:240-303`) spawned Notepad, Brave, and Chrome (inheriting stdout/stderr pipe handles that kept the test runner alive indefinitely) and sent 50 OS volume SendKeys events. |

---

## 3. Workspace Dependency Graph & Architecture Map

```
                   +-----------------------+
                   |    apps/pico-cli      |  (Clap CLI, voice loop, doctor)
                   +-----------+-----------+
                               |
                   +-----------v-----------+      Currently monolithic:
                   |  apps/pico-desktop    | ---> Instantiates full PicoRuntime,
                   +-----------+-----------+      ONNX AsrDaemon, SQLite stores
                               |                  inside the GUI process
                   +-----------v-----------+
                   |   pico-orchestrator   |  (PicoRuntime, ObjectiveVerifier)
                   +--+--+--+--+--+--+--+--+
                      |  |  |  |  |  |  |
       +--------------+  |  |  |  |  |  +---------------+
       v                 v  |  |  |  v                 v
+-------------+   +------+  |  |  +----------+  +-------------+
| pico-voice  |   |         |  |  pico-tools |  |pico-learning|
| (VAD, STT,  |   |         |  |  (Registry, |  |(SQLite exp, |
|  TTS, pipe) |   |         |  |   catalog,  |  | trainer,    |
+------+------+   |         |  |   built-ins)|  | evaluator)  |
       |          |         |  +------+------+  +------+------+
       v          v         v         |                |
+-------------+ +-------------+ +-----v-------+        |
| pico-audio  | |pico-decision| |pico-platform|        |
| (cpal I/O,  | |(Intent, BM25| |(apps, sys,  |        |
|  resampler) | | engine, args| | clipboard)  |        |
+------+------+ | synthesizer)| +------+------+        |
       |        +------+------+        |               |
       |               |               v               |
       |               |      +--------------+         |
       |               +----->|pico-inference|         |
       |               |      |(Embeddings,  |         |
       |               |      | ranker, platt|         |
       |               |      +------+-------+         |
       |               |             |                 |
       |               |      +------v-------+         |
       |               |      |pico-security |         |
       |               |      |(PathSandbox, |         |
       |               |      | Policy,Audit)|         |
       |               |      +------+-------+         |
       |               |             |                 |
       v               v             v                 v
    +--+---------------+-------------+-----------------+--+
    |                       pico-core                      |
    |  (Config, Errors, PicoEvent, Session, Core Traits)   |
    +------------------------------------------------------+
```

| Crate | Source LOC (approx. file span) | Core Responsibilities | Primary Audited Defects |
|---|---|---|---|
| `pico-core` | ~760 | Traits (`Tool`, `SpeechSynthesizer`, `SpeechRecognizer`), `PicoConfig`, `PicoEvent`, `Session` | `SpeechSynthesizer::synthesize -> Vec<u8>` lacks sample rate/channel/encoding metadata (`traits.rs:76`); `DesktopConfig` carries stale HTTP server fields (`config.rs:319-339`); config allows conflicting/unused knobs. |
| `pico-audio` | ~563 | `cpal` capture & playback, device enumeration, linear resampler | Playback ignores hardware sample rate (`playback.rs:92-170`); `Mutex<VecDeque<f32>>` in audio callback; linear-only resampler (`resampler.rs:55`) without anti-aliasing filter; capture omits `I32`/`U8` formats and allocates per callback (`capture.rs`). |
| `pico-voice` | ~1,115 | `VadProcessor`, STT (`AsrDaemon`, SAPI/Whisper), TTS (Win SAPI / `espeak-ng`), `VoicePipeline` | `espeak-ng --stdout` emits RIFF WAV at 22.05 kHz fed as raw PCM (`tts.rs:165`); Win SAPI returns `Ok(Vec::new())` on failure (`tts.rs:102`); `config.tts_engine` ignored; wall-clock VAD misses pre-roll (`vad.rs:57-146`); `SpeechNormalizer` strips `"like"` and all ASCII punctuation (`stt.rs:24,50`). |
| `pico-security` | ~922 | `PathSandbox`, `SecurityPolicy`, `ChannelConfirmationManager`, `AuditLogger`, `InputSanitizer`, `VoiceAuthorizer` | `allowed_roots.is_empty()` bypasses containment (`sandbox.rs:174-177`); trailing `..` uncaught (`:96`); permissive mode silently approves `Restricted`/`ConfirmationRequired` (`policy.rs`); confirmation tokens lack session/action/args binding (`policy.rs:26-100`); secret masker skips string values (`audit.rs:70-84`). |
| `pico-platform` | ~670 | App launch/list/kill, clipboard CLI wrappers, OS/memory/battery info | `cmd /C start` and `Start-Process '{}'` injection (`apps.rs:124-144`); `kill_process` spawns external taskkill/kill; `clipboard.rs` shells out cleanly? spawns PowerShell/xclip/wl-copy; `sysinfo.rs:107` hardcodes `"Windows 11 x64"`. |
| `pico-inference` | ~615 | `SemanticEmbedder` (128-d FNV-1a hash), `CrossEncoderRanker`, `Calibrator` | Misleading neural cross-encoder naming (`ranking.rs:49-50`); independent Platt `a=10, b=-5` produces 90%+ confidence on competing actions (`ranking.rs:162-169`); hashed bag-of-words n-grams have high collision noise (`embeddings.rs:63-81`). |
| `pico-decision` | ~2,150 | `ToolIndex` (BM25), `IntentClassifier`, `ArgumentExtractor`, `DecisionEngine`, `ResponseSynthesizer` | `"second"`/`"first"`/`"none"` substring checks break queries (`intent.rs:329-353`); `Index::tokenize` keeps `.`/`_` inside tokens (`index.rs:282`); `O(N^2)` avg doc length recomputation (`index.rs:86`); max candidates hardcoded to 15 (`engine.rs:156`); no stateful clarification object. |
| `pico-learning` | ~1,300+ | SQLite observations, `AntiPoisoningGuard`, `RankingTrainer`, `ModelEvaluator` | `ModelEvaluator` leaks ground-truth signals (`intent_affinities: [1.0, 0.1, 0.05]`, `historical_scores: [0.9, 0.5, 0.5]`, `evaluator.rs:129-130`); `RankingTrainer` adds constant pseudo-gradients (`0.2, 0.1, 0.05`, `trainer.rs:58-60`). |
| `pico-tools` | ~28,000 | Built-in tools, catalog registry, `windows.rs` (1,056), `linux.rs` (1,056), `adapter.rs` | 2,016/2,112 catalog actions are broken duplicates; `adapter.rs` PowerShell/procfs vulnerabilities; `SetMasterVolumeTool` uses `SendKeys`; `FetchWebContentTool` susceptible to DNS rebinding & unbounded body downloads; `SearchFilesTool` traverses symlinks. |
| `pico-memory` | ~564 | SQLite session/message/fact/pref store + `ContextBudgeter` | Plaintext local SQLite with single mutex around connection; `budget_messages` character/4 approximation. |
| `pico-orchestrator` | ~1,087 | `PicoRuntime`, `ObjectiveVerifier`, E2E & benchmark suites | Fragile global `pending_clarification` tuple (`coordinator.rs:36,243-283`); false-success announcements when `verification.verified == false` or app missing (`coordinator.rs:447-472`); E2E test mutates live user desktop/audio (`e2e_turn_test.rs:240-303`). |

---

## 4. End-to-End Interaction Execution Traces

### 4.1 Trace 1: Voice Pipeline (Microphone -> Audio Output)
1. **Capture** (`pico-audio/src/capture.rs:20-162`): `cpal` opens default input device at native hardware rate (often 48 kHz) and channels (stereo or mono). Only `SampleFormat::F32` and `SampleFormat::I16` are matched; `I32`/`U8` devices produce silence or unsupported gaps. Inside the real-time callback, `AudioConverter::convert_f32_to_16k_i16` (`resampler.rs:18-60`) averages channels and performs un-filtered linear interpolation, allocating a `Vec<f32>` + `Vec<i16>` per callback and calling `tokio::sync::mpsc::Sender::blocking_send` / `try_send`.
2. **VAD** (`pico-voice/src/vad.rs:57-146`): Computes RMS energy in chunks. Tracks `Instant::now()` wall clock instead of accumulated PCM samples, causing silence timers to drift under CPU scheduling jitter or offline replay. Contains no pre-roll ring buffer, clipping initial phonemes before 2 active frames exceed `energy_threshold`.
3. **STT** (`pico-voice/src/stt.rs:81-371`): Spawns Python `scripts/asr_service.py` or falls back to PowerShell SAPI / CLI. `PlatformSpeechRecognizer::from_config` keeps `daemon` behind a `Mutex<Option<AsrDaemon>>` with `Stdio::null()` on stderr, infinite `read_line` wait (no timeout), and skips `"whisper"`/`"native_sapi"` engine switches cleanly. `SpeechNormalizer::normalize` (`stt.rs:16-55`) strips `"like"` globally (corrupting queries like `"looks like..."`) and removes trailing/embedded punctuation (breaking decimals like `"3.14"` or paths like `C:\foo`). Note: `scripts/asr_service.py:22` also mis-normalizes 8-bit WAV as `(uint8 / 128.0) - 1.0` instead of checking `sampwidth`, though Pico writes 16-bit WAVs.
4. **Decision** (`pico-decision`): Normalization feeds `IntentClassifier`, BM25 `ToolIndex`, `SemanticEmbedder`, `CrossEncoderRanker`, and `DecisionEngine`.
5. **TTS & Playback** (`pico-voice/src/tts.rs`, `pico-audio/src/playback.rs`):
   - **Windows**: `NativeWindowsSynthesizer` spawns `powershell.exe` running `System.Speech.Synthesis.SpeechSynthesizer` configured for `16000 Hz, 16-bit, Mono`, Base64-encodes raw PCM bytes, returns `Ok(Vec::new())` silently on failure (`tts.rs:102`).
   - **Linux**: `NativeLinuxSynthesizer` runs `espeak-ng --stdout` which writes a **22,050 Hz RIFF WAV stream with a 44-byte header**.
   - **Playback Bug**: `AudioPlayback::start` (`playback.rs:57-170`) opens the OS default output device at **48,000 Hz** (typical hardware default) and pops samples from `Mutex<VecDeque<f32>>` without resampling (and with WAV header bytes treated as PCM audio on Linux). Result: **3.0x speed/pitch distortion on Windows** (48,000 / 16,000) and **2.18x distortion + click burst on Linux** (48,000 / 22,050 + WAV header).

### 4.2 Trace 2: Ambiguity & Clarification Followup (Real Failure Observed in Logs)
- **Observed Production Incident** (`data/pico_memory.sqlite`):
  - Turn 1: User says `"Mute sounds"`. Decision engine returns `audio.mute` (96% confidence) and `audio.set_volume` (95% confidence) -> triggers clarification because Platt calibration assigns >95% to both and margin is `0.01 < 0.08`.
  - Turn 1b: User says `"increase brightness"` -> `"I found two likely actions: 'windows.display.create_monitor' (67% confidence) and 'windows.display.query_monitor' (64% confidence)."` (wrong candidates and inflated percentages).
  - Turn 2 Failure Mode: If user replies `"The second one"` or `"option two"`:
    1. `coordinator.rs:251-254` only matches exact strings (`"second"`, `"2"`, `"the second"`, `"option 2"`, or `starts_with("second ")`), missing `"the second one"` and `"option two"`.
    2. `lock.take()` (`coordinator.rs:245`) permanently erases `pending_clarification` even when clarification selection isn't resolved or needs argument follow-up.
    3. Matched clarification falls straight into `execute_call` (`coordinator.rs:270`) without checking `res.missing_required`, without running `ObjectiveVerifier`, without checking `tool_res.success == false`, and without recording to `memory` or `learning` or emitting `ToolExecuted` events.
    4. Worse, if clarification wasn't active (or was lost), `IntentClassifier::is_direct_conversation` (`intent.rs:346-350`) intercepts `"first"`, `"second"`, `"none"`, `"cancel"` as `DirectConversation`, and any query containing `"second"` (e.g. `"Set a timer for 30 seconds"`) gets hijacked into chat!

### 4.3 Trace 3: Failed Execution Reported as Success
- **Observed Production Incident** (`data/pico_memory.sqlite` + `logs/security_audit.jsonl`, `2026-11-22T09:14:27`):
  - User: `"Close brave"`
  - `DecisionEngine`: mis-routes to `open_application` with `{"application": "Close brave", "target": "Close brave"}`.
  - `PlatformApps::open_application` (`apps.rs:124-144`) runs `cmd /C start "" "Close brave"` -> spawns detached `cmd.exe` successfully (`Ok(child)`), does not wait or check if `"Close brave"` failed to launch, and returns `Ok(())`.
  - `OpenApplicationTool` returns `{"opened": "Close brave", "status": "success"}`.
  - `coordinator.rs:467` speaks `"Opened Close brave."` and logs `"Success"` to `security_audit.jsonl`.
  - Even when `ObjectiveVerifier::verify_execution` (`verification.rs:24-44`) sets `verified = false`, `coordinator.rs:447-460` only emits a `warn!` log and STILL calls `ResponseSynthesizer::format_tool_result` claiming success because `result.success` was `true`.

### 4.4 Trace 4: Tool Execution via Massive Catalog Adapter
- `ToolMetadata` -> `EcosystemTool::execute` (`catalog/types.rs:170`) -> `execute_tool_adapter` (`catalog/adapter.rs:10`).
- On Windows (`adapter.rs:125-194`), arguments are interpolated directly into a PowerShell command string (`resolved_script = resolved_script.replace("$name", val)`) with only `'` -> `''` and `` ` `` -> `` `` `` escaping. Commands from `windows.rs` (for example `Get-AudioDevice -Name $name`, `Get-Volume -Name $name`, `Get-Process -Name $name`, `powercfg.exe -Name $name`) run inside `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ...`. Because 21 categories x 48 actions = 1,008 actions per OS share identical underlying commands (validated via Python forensic extraction), destructive/mutating verbs (`set_*`, `delete_*`, `stop_*`, `disable_*`, `restart_*`) silently run read-only queries (`Get-*`, `ps`, `systemctl`) or fail on unknown flags (`-Name $name`).

### 4.5 Trace 5: Desktop Companion Event Loop & Resource Drain
- `apps/pico-desktop/src/main.rs:972` initializes `PicoRuntime::new(config)` on startup, loading 2,126 tools into `ToolIndex`, opening two SQLite databases, starting `cpal` audio playback stream, and spawning `python scripts/asr_service.py` (which loads ~622 MB ONNX model weights).
- `PicoDesktopApp::update` (`main.rs:226`) calls `ctx.request_repaint_after(Duration::from_millis(16))` on every frame unconditionally, locking the UI thread into a ~60 Hz wakeup loop forever even when idle or minimized, and queries `PlatformSysInfo::get_system_stats()` periodically from the render path (`:235`).

---

## 5. Automated Audit of the 2,112-Tool Catalog (`windows.rs` & `linux.rs`)

A deterministic Python static analyzer parsed every `CategoryDef` and `ActionDef` across `crates/pico-tools/src/catalog/windows.rs` (13,234 lines) and `crates/pico-tools/src/catalog/linux.rs` (13,234 lines):

| Platform | Categories | Total Actions | Unique `(adapter_cmd, adapter_args)` Pairs in Category 1 (`filesystem`) | Unique `(adapter_cmd, adapter_args)` Pairs in Categories 2-22 | Confirmed Broken / Duplicated Actions |
|---|---|---|---|---|---|
| Windows | 22 (`48` actions each) | 1,056 | 45 (`find_files_by_size`, `find_files_by_date` drop params; `find_duplicate_files`/`find_empty_directories`/`remove_empty_directories` share `Get-ChildItem -Recurse`) | **1 per category** (21 total across 1,008 actions) | **>= 1,011** (95.7%) |
| Linux | 22 (`48` actions each) | 1,056 | 47 (`append_text` -> `tee -a` hangs stdin; `backup_acls`/`restore_acls`/`export_directory_tree` ignore target params; `create_junction` -> `mount --bind`; `encrypt_efs`/`decrypt_efs` -> interactive `gpg`) | **1 per category** (21 total across 1,008 actions) | **>= 1,014** (96.0%) |
| **Combined** | **44** | **2,112** | **92** | **42 across 2,016 actions** | **>= 2,025 / 2,112 (95.9%)** |

Notable systemic failure example: `windows.audio` maps all 48 actions (from `mute_audio` and `set_endpoint_volume` to `disable_endpoint`) to `Get-AudioDevice -Name $name`. Note that `Get-AudioDevice` is a third-party PowerShell Gallery module (`AudioDeviceCmdlets`), not present in stock Windows 11 PowerShell, so all 48 Windows audio catalog actions fail at runtime.

---

## 6. Conclusions & Audit Deliverables

All defects uncovered during this forensic audit are cataloged with exact coordinates, reproduction steps, fixes, and regression test specifications in the companion audit files:
- [`docs/audit/PICO_BUG_REGISTER.md`](./PICO_BUG_REGISTER.md)
- [`docs/audit/PICO_SECURITY_AUDIT.md`](./PICO_SECURITY_AUDIT.md)
- [`docs/audit/PICO_ARCHITECTURE_REVIEW.md`](./PICO_ARCHITECTURE_REVIEW.md)
- [`docs/audit/PICO_IMPROVEMENT_BACKLOG.md`](./PICO_IMPROVEMENT_BACKLOG.md)
