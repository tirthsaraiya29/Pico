# Pico Forensic Bug Register

**Repository**: `https://github.com/tirthsaraiya29/Pico.git` (Audited at `faf1a5a8da856a2b88eceb188e92aa1d32e401d9`, remediated through `main`)
**Classification**: `P0` Critical functionality or security failure | `P1` Major functionality, architecture or reliability failure | `P2` Significant quality, usability or maintainability problem | `P3` Minor bug, edge case or refinement | `P4` Optional improvement / optimization.
**Evidence Labels**: `Confirmed` (observed directly in source code, command output, SQLite DB, or audit logs) | `Suspected` (likely under edge or Linux runtime conditions inferred from code).
**Remediation Summary**: All 18 `P0`, 28 `P1`, and 22 of 24 `P2` defects across audio DSP/playback, voice pipeline, decision/clarification calibration, tool catalog integrity, security sandbox/SSRF/confirmations/audit logging, SQLite queries, and lightweight split-process desktop companion (`41.04 MB` RSS, `0.47%` idle core CPU) have been remediated and verified in automated regression tests on `main` (`BUG-055` deferred; `BUG-062` partially remediated). Headless Windows CI also uncovered and resolved a transient COM STA lifetime defect in `crates/pico-audio/src/host_com.rs` (`6873408`). See [PROJECT_STATUS.md](../../PROJECT_STATUS.md).

---

## Summary Counts by Severity

| Severity | Count | Primary Concentration |
|---|---|---|
| `P0` | 18 | Voice/TTS playback pitch/speed & WAV-as-PCM, broken clarification & substring hijacking, 2,016 duplicated fake catalog tools, shell/PowerShell/procfs security injections |
| `P1` | 28 | Per-candidate Platt probability overconfidence, missing-arg drops, false success synthesis, VAD timing & pre-roll, real-time audio mutex/allocs, desktop 60 FPS bloat & dead UI controls, E2E test side effects |
| `P2` | 24 | Index dot-tokenization & O(N^2) rebuild, evaluator label leakage & fake training gradients, hardcoded OS names, clippy/fmt CI failures, stale docs & `.gitignore` hazards |
| `P3` | 14 | Speech normalizer edge cases, calculator/argument parser quirks, error propagation gaps, shutdown cleanup |
| `P4` | 8 | Resampling filter quality, companion feature refinements, CI automation optimizations |
| **Total** | **92** | Across all 11 crates, 2 apps, scripts, tests, CI, docs, and site |

---

## P0: Critical Functionality & Security Defects

### BUG-001 | P0 | Confirmed | `crates/pico-audio/src/playback.rs:57-170`
- **Observed Behaviour**: `AudioPlayback::start` obtains `default_config = device.default_output_config()`, extracts `channels` and `sample_format`, ignores `default_config.sample_rate()`, and plays 16,000 Hz or 22,050 Hz speech samples directly into the hardware output device stream (commonly 48,000 Hz or 44,100 Hz).
- **Expected Behaviour**: Audio frames carry an explicit source `AudioFormat` (`sample_rate`, `channels`, `sample_format`) and are resampled/converted to the output device's actual negotiated `sample_rate`, `channels`, and `sample_format` before enqueueing.
- **Root Cause**: Playback buffer assumes the device operates at the synthesizer rate (16 kHz) and omits output resampling. At 48 kHz output, 16 kHz PCM plays back at `48000 / 16000 = 3.0x` speed and pitch, rendering speech alien and unintelligible.
- **Reproduction**: Speak any reply on a standard Windows 11 / Linux desktop with default 48 kHz output.
- **Recommended Correction**: Introduce a unified `AudioBuffer { samples: Vec<f32>, sample_rate: u32, channels: u16 }`, use an anti-aliased resampler to convert from source `sample_rate` to actual `output_config.sample_rate().0`, and downmix/upmix channels cleanly.
- **Regression Test**: Assert playback resampler converts 16,000 Hz / 22,050 Hz buffers to 44,100 Hz and 48,000 Hz preserving duration (`len / out_rate == in_len / in_rate +- 1 frame`) and fundamental sine frequency within 1 Hz.
- **Status**: `Fixed` (`4e79282`)

### BUG-002 | P0 | Confirmed | `crates/pico-voice/src/tts.rs:165-190`, `crates/pico-voice/src/pipeline.rs:120-139`
- **Observed Behaviour**: `NativeLinuxSynthesizer::synthesize_sync` executes `espeak-ng --stdout`, which outputs a 22,050 Hz **RIFF WAV file** (with a 44-byte RIFF/WAVE header), and returns those raw bytes into `VoicePipeline::speak` -> `play_pcm_bytes`, which treats every pair of bytes (including the `"RIFF"` ASCII header) as signed 16-bit PCM samples at 16 kHz.
- **Expected Behaviour**: Parse RIFF WAV headers explicitly using `hound` (or explicit container parser) to extract actual `sample_rate`, `channels`, bits-per-sample, and pure PCM sample payload; never treat WAV headers as audio samples.
- **Root Cause**: Confusion between encoded WAV containers (`espeak-ng --stdout`) and headerless raw PCM streams (`System.Speech` `SetOutputToAudioStream`).
- **Reproduction**: Synthesize speech on Linux with `espeak-ng` installed; inspect leading 4 bytes (`52 49 46 46` `"RIFF"`), played as acoustic pop plus 2.18x-3.0x distorted speech.
- **Recommended Correction**: Return `SynthesizedAudio { samples: Vec<f32>, sample_rate: u32, channels: u16 }` from `SpeechSynthesizer`; decode WAV headers whenever a synthesizer emits WAV.
- **Regression Test**: Feed synthetic 22,050 Hz RIFF WAV bytes into the TTS/playback decoder and verify header bytes do not appear in decoded PCM output and declared sample rate equals 22,050 Hz.
- **Status**: `Fixed` (`4e79282`)

### BUG-003 | P0 | Confirmed | `crates/pico-voice/src/tts.rs:100-103,235-285`
- **Observed Behaviour**: `PlatformSpeechSynthesizer::synthesize` (`tts.rs`) marks `tts_engine: String` as `#[allow(dead_code)]`, ignores user config (`"piper"`, `"espeak"`, `"mock"`, `"system"`), and unconditionally invokes `NativeWindowsSynthesizer` or `NativeLinuxSynthesizer`. Furthermore, `NativeWindowsSynthesizer::synthesize_sync` returns `Ok(Vec::new())` (`line 102`) when the PowerShell process exits non-zero.
- **Expected Behaviour**: Dispatch cleanly based on `VoiceConfig.tts_engine`, discover installed engines and voices, support Piper neural voices where available alongside OS-native WinRT/SAPI/SpeechDispatcher/espeak-ng engines, and return explicit `Err(PicoError::Voice(...))` when synthesis fails.
- **Root Cause**: Stubbed engine routing and silent error swallowing.
- **Reproduction**: Set `tts_engine = "piper"` in `configs/pico.toml` or pass invalid voice name on Windows; engine field is ignored and failures produce `Ok(vec![])`.
- **Recommended Correction**: Implement real TTS backend trait dispatch (`WinRt`, `Sapi`, `Piper`, `EspeakNg`, `SpeechDispatcher`, `Mock`), `list_voices()`, and hard error propagation on subprocess/API failure.
- **Regression Test**: Unit test verifying `tts_engine = "mock"` and unknown/failing engine error handling never yield silent `Ok(vec![])`.
- **Status**: `Fixed` (`4e79282`)

### BUG-004 | P0 | Confirmed | `crates/pico-tools/src/catalog/windows.rs:615-13234`, `crates/pico-tools/src/catalog/linux.rs:615-13234`
- **Observed Behaviour**: Categories 2 through 22 on both Windows (1,008 actions) and Linux (1,008 actions) each contain 48 distinct advertised action names (`set_*`, `delete_*`, `stop_*`, `enable_*`, `restart_*`, etc.) where **all 48 entries map to the exact same `adapter_cmd` and `adapter_args`** (e.g., `Get-Volume -Name $name`, `Get-Process -Name $name`, `Get-AudioDevice -Name $name`, `powercfg.exe -Name $name`, `ps aux $name`, `systemctl $name`, `apt $name`). On Windows, `Get-AudioDevice` is not even installed by default.
- **Expected Behaviour**: Every registered tool executes a distinct, real, parameter-accurate, cross-checked OS operation, or is omitted if unsupported on the target platform.
- **Root Cause**: Procedural/templated catalog generation reused one placeholder command template per category across 48 synthetic subaction definitions, and `catalog/mod.rs` tests only checked count == 1,056 and non-overlap of names rather than execution correctness.
- **Reproduction**: Inspect `windows.rs` lines 615-1225 (every single `storage` action runs `Get-Volume`) or `windows.audio` (every action runs `Get-AudioDevice`).
- **Recommended Correction**: Audit and remove all 2,016 fabricated/duplicate catalog entries and rebuild a verified, realistic, working catalog with unique command adapters, accurate schemas, accurate `PermissionTier`s, and per-category execution verification.
- **Regression Test**: Automated catalog invariant test asserting no category collapses distinct verbs onto identical `(adapter_cmd, adapter_args)` templates and every catalog action passes parameter placeholder validation and dry-run/real execution checks.
- **Status**: `Fixed` (`4e79282`)

### BUG-005 | P0 | Confirmed | `crates/pico-decision/src/intent.rs:329-353`
- **Observed Behaviour**: `IntentClassifier::is_direct_conversation` scans `exact_phrases` (`"first"`, `"second"`, `"neither"`, `"none"`, `"cancel"`) using `lower.contains(phrase)`. Any normal request containing `"second"` (e.g., `"Set a timer for 30 seconds"`) or `"first"` (`"Open the first browser tab"`) is hijacked into `UserIntent::DirectConversation`, bypassing tool retrieval completely.
- **Expected Behaviour**: Word/phrase checks only match clarification or conversational turns in appropriate context and use whole-word boundaries (`\b`), never hijacking `"seconds"` as `"second"`.
- **Root Cause**: Ad-hoc substring patch added to work around broken clarification turns.
- **Reproduction**: Call `IntentClassifier::new().classify("Set a timer for 30 seconds")` -> returns `DirectConversation` (`0.90`).
- **Recommended Correction**: Remove clarification ordinal tokens from `IntentClassifier::is_direct_conversation`, parse clarification responses explicitly inside session-scoped `ClarificationState`, and enforce strict regex/token boundaries on conversational phrases.
- **Regression Test**: Assert `"Set a timer for 30 seconds"`, `"Check first disk partition"`, and `"Cancel timer"` are not classified as `DirectConversation`.
- **Status**: `Fixed` (`4e79282`)

### BUG-006 | P0 | Confirmed | `crates/pico-orchestrator/src/coordinator.rs:36,243-283`
- **Observed Behaviour**: `pending_clarification` is a single global `Option<(String, Vec<CandidateScore>)>` unlinked from `SessionId`, immediately cleared via `lock.take()` before parsing succeeds, only matches rigid strings (`"first"`, `"1"`, `"the first"`, `"option 1"`, ignoring `"the second one"`, `"option two"`, `"execute the first"`, `"choose the second"`), ignores missing required arguments, bypasses `ObjectiveVerifier`, ignores `tool_res.success == false`, skips `memory.save_message` and `learning.record_observation`, and skips event emission.
- **Expected Behaviour**: Maintain a typed, session-scoped `ClarificationState` holding `session_id`, `original_request`, candidate metadata/schemas/extracted args, clarification kind (`AmbiguousCandidate` vs `MissingArguments` vs `Confirmation`), creation timestamp, and TTL; preserve state on ambiguous/unparseable replies, support all natural selection phrasings, re-validate tool/args/permissions before execution, and route execution through the full verified pipeline.
- **Root Cause**: Quick prototype handler grafted onto the top of `process_turn`.
- **Reproduction**: Trigger clarification, respond `"the second one"`, observe `pending_clarification` wiped and turn treated as fresh query; or trigger across two sessions and observe cross-session leakage.
- **Recommended Correction**: Implement session-keyed `ClarificationState` state machine in `pico-core`/`pico-orchestrator` cleanly shared with `DecisionEngine`.
- **Regression Test**: End-to-end multi-session test covering `"the second one"`, `"option two"`, `"execute the first"`, `"choose the second"`, missing-arg follow-up, expiration timeout, cancellation, interruption/new topic, and session isolation.
- **Status**: `Fixed` (`4e79282`)

### BUG-007 | P0 | Confirmed | `crates/pico-orchestrator/src/coordinator.rs:447-520`, `crates/pico-platform/src/apps.rs:124-144`
- **Observed Behaviour**: In real production run (`data/pico_memory.sqlite`, `logs/security_audit.jsonl` at `2026-11-22T09:14:27`), `"Close brave"` routed to `open_application("Close brave")`. `apps.rs:124` launched `cmd /C start "" "Close brave"` returning `Ok(())` without checking whether `"Close brave"` was valid or opened, and Pico reported `"Opened Close brave."` (`Success`). Additionally, `coordinator.rs:460-472` only checks `!result.success` and ignores `verification.verified == false` when formatting the user reply.
- **Expected Behaviour**: Unknown apps / failing launches must fail loudly; `"close/quit/kill <app>"` must target process termination (with confirmation) rather than app launching; when `verification.verified == false`, Pico must NEVER claim success.
- **Root Cause**: Unchecked detached spawn returns `Ok(())`; no app-close intent/argument guard; `coordinator.rs` treats `result.success == true && verification.verified == false` as success to the user.
- **Reproduction**: Process `"Close brave"` or `"open nonexistent_app_xyz123"`.
- **Recommended Correction**: Distinguish open vs close intents, validate target executable/app alias before reporting success, check process birth/exit errors in `PlatformApps::open_application`, and gate `ResponseSynthesizer` on `result.success && verification.verified`.
- **Regression Test**: Test `"Close brave"` routes to close/kill confirmation (never `open_application`), and `"open nonexistent_app_xyz_999"` fails with error rather than `"Opened ..."`.
- **Status**: `Fixed` (`4e79282`, `6637207`)

### BUG-008 | P0 | Confirmed | `crates/pico-platform/src/apps.rs:124-144`
- **Observed Behaviour**: `PlatformApps::open_application` constructs `cmd /C start "" <target>` (`line 124`) and fallback PowerShell `format!("Start-Process '{}'", target)` (`line 141`) without escaping `'` or validating against shell metacharacters, allowing arbitrary PowerShell / cmd execution from unquoted/crafted app names (e.g. `foo'; calc.exe; '` or `foo & calc`).
- **Expected Behaviour**: Never concatenate untrusted user strings into `cmd /C` or PowerShell script bodies. Resolve applications via allowlisted executables/Start Menu/PATH/.desktop lookup or Win32 `ShellExecuteExW` / parameterized `Command` / `xdg-open` without shell interpretation.
- **Root Cause**: Direct string interpolation into shell interpreters.
- **Reproduction**: Pass `"notepad'; Start-Process calc; #"` to `PlatformApps::open_application`.
- **Recommended Correction**: Replace shell fallback with validated direct process spawn / Win32 API (`ShellExecuteExW` with `SEE_MASK_NOASYNC` / `Command::new(resolved)`) or safe environment-passed arguments, rejecting control characters and shell metacharacters.
- **Regression Test**: Security test ensuring `"notepad'; calc; #"` and `"notepad & whoami"` are rejected and never invoke injected commands.
- **Status**: `Fixed` (`6637207`)

### BUG-009 | P0 | Confirmed | `crates/pico-tools/src/catalog/adapter.rs:139-151,225-260`
- **Observed Behaviour**: (a) `execute_windows_command` substitutes argument values via naive `.replace('\'', "''").replace('`', "``")` directly into `-Command` script text without quoting the `$param` placeholder if not already quoted in `adapter_args`, exposing PowerShell subexpression/semicolon/pipe injection (`$(...)`, `;`, `|`). (b) `execute_linux_command` concatenates `$param` directly into `/proc/...` or `/sys/...` filesystem paths (`line 225-236`) without path traversal checks (`..`, `/`) allowing arbitrary reads under `/proc` or `/sys`.
- **Expected Behaviour**: Bind parameters via environment variables (`$env:PICO_ARG_<NAME>`) or strict typed validators on Windows, never interpolating raw strings into PowerShell ASTs; validate every `/proc`/`/sys` placeholder as strictly numeric PID or single safe ASCII identifier without `/` or `..`.
- **Root Cause**: String-template replacement used instead of parameterized argument passing.
- **Reproduction**: Supply `name = "$(whoami)"` on Windows or `pid = "../meminfo"` / `interface = "../../etc/passwd"` on Linux.
- **Recommended Correction**: Redesign `adapter.rs` argument passing to use process `env(...)` + strict allowlisted pattern checks on `/proc`/`/sys` path segments, plus enforce `PathSandbox` on all path parameters.
- **Regression Test**: Assert `$()`, `;`, `|`, `../` injection payloads are sanitized or rejected before execution.
- **Security Implications**: Arbitrary command execution (Windows) and procfs/sysfs traversal (Linux).
- **Status**: `Fixed` (`6637207`)

### BUG-010 | P0 | Confirmed | `crates/pico-tools/src/filesystem_tools.rs:233-252`
- **Observed Behaviour**: `SearchFilesTool::walk` skips only `"target"`, `".git"`, `"node_modules"`, then follows `if path.is_dir()` unconditionally without checking `symlink_metadata()` or `entry.file_type().is_symlink()`, nor re-validating resolved children against `self.sandbox`.
- **Expected Behaviour**: Never follow directory symlinks or NTFS junctions outside the sandbox; check `entry.file_type()` for symlinks/reparse points and skip external links or validate canonical target via `PathSandbox`.
- **Root Cause**: Standard `Path::is_dir()` traverses symbolic links transparently.
- **Reproduction**: Place a symlink/junction inside workspace root pointing to `C:\Users\<user>\.ssh` or `/etc`; `search_files` lists target filenames outside the sandbox.
- **Recommended Correction**: Inspect `entry.file_type()` (and Windows reparse tag / canonicalize child dir via `sandbox.validate_path`), skip symlinks pointing outside allowed roots, and enforce bounded max entries/files scanned.
- **Regression Test**: Create directory symlink to out-of-sandbox temp dir and assert `SearchFilesTool` never returns files inside it.
- **Status**: `Fixed` (`6637207`)

### BUG-011 | P0 | Confirmed | `crates/pico-security/src/sandbox.rs:95-102,111-177`
- **Observed Behaviour**:
  1. Line 96 checks `path_str.contains("../") || path_str.contains("..\\") || path_str == ".."`, missing trailing `dir/..` or `dir\..` strings.
  2. Line 175: `if self.allowed_roots.is_empty() { return Ok(normalized); }` completely disables root containment when `allowed_roots` is empty or fails to resolve!
  3. Line 153: `sensitive_fragments` omits `ntds.dit`, `sam`, `shadow`, `.kube`, `.docker`, `.npmrc`, `.netrc`, `.git-credentials`, `keychain`, `appdata\local\google\chrome\user data`, etc.
  4. Multi-step check-then-use TOCTOU window when `write_file` creates intermediate dirs and writes later.
- **Expected Behaviour**: Reject any `Component::ParentDir` cleanly after explicit checks, fail closed (`Err`) when `allowed_roots` is empty instead of returning `Ok`, expand sensitive credential exclusions, and canonicalize created/parent handles safely.
- **Root Cause**: String-based traversal heuristic and fail-open empty-roots fallback.
- **Reproduction**: Construct `PathSandbox::new(vec![], vec![])` and call `validate_path("/etc/passwd")` -> returns `Ok("/etc/passwd")`.
- **Recommended Correction**: Fail closed on empty `allowed_roots` (default explicitly to canonicalized current dir or error), check `Component::ParentDir` and trailing `..`, reject reparse points/symlinks in non-existing suffix creation paths, expand sensitive credential fragment rules.
- **Regression Test**: Unit tests for empty `allowed_roots`, `subdir/..`, symlinks to outside dirs, `.netrc`/`.kube` denial.
- **Status**: `Fixed` (`6637207`)

### BUG-012 | P0 | Confirmed | `crates/pico-tools/src/utility_tools.rs:278-455`
- **Observed Behaviour**:
  1. `FetchWebContentTool::execute` validates DNS via `validate_url_ssrf` (`line 330`) and then lets `reqwest` re-resolve DNS independently (`line 428+`), creating a classic DNS-rebinding TOCTOU vulnerability (first lookup returns public IP, second lookup returns `127.0.0.1` or `169.254.169.254`).
  2. `is_safe_public_ipv6` (`line 278`) omits NAT64 (`64:ff9b::/96`, `64:ff9b:1::/48`), Teredo (`2001::/32`), 6to4 (`2002::/16`), discard-only, and IPv4-compatible `::/96` embedded private IPv4 checks.
  3. `resp.text().await` (`line 452`) reads an unbounded HTTP response body into RAM, enabling OOM DoS on malicious URLs.
- **Expected Behaviour**: Pin `reqwest::Client` connections to the already-validated `SocketAddr` list (custom `reqwest::dns::Resolve`), block all IPv6 transition/private ranges, stream body chunks with strict byte cap (e.g. 2 MB), and re-validate every redirect target.
- **Root Cause**: Unpinned async HTTP client + full-body buffering.
- **Recommended Correction**: Implement `VerifiedDnsResolver` wrapper pinning validated `SocketAddr`, complete IPv6 transition range checks, and bounded streaming read.
- **Regression Test**: Unit tests for IPv6 NAT64 (`64:ff9b::127.0.0.1`), 6to4 (`2002:7f00:0001::1`), and truncated streaming on oversized body.
- **Status**: `Fixed` (`6637207`)

### BUG-013 | P0 | Confirmed | `crates/pico-security/src/policy.rs:26-100,136-215`
- **Observed Behaviour**: `ChannelConfirmationManager` stores only `HashMap<String, oneshot::Sender<bool>>` (`line 27`), never recording which `session_id`, `tool_name`, `arguments`, or expiration belongs to `request_id`. Any caller possessing a `request_id` string (or calling `respond` in another session) can approve an unrelated action. Moreover, `strict_mode = false` ("Permissive") skips confirmation even for `Restricted` actions depending on caller configuration or missing provider behavior checks (`policy.rs:140+`), and `apps/pico-cli/src/main.rs:66` + `apps/pico-desktop/src/main.rs:66` store only a single `Option<PendingConfirmation>` slot that silently overwrites prior pending confirmations under concurrent requests.
- **Expected Behaviour**: Confirmation requests are scoped to `(request_id, session_id, tool_name, args_hash, expires_at)` with single-use consumption, cryptographic non-predictable IDs, explicit expiry enforcement, and support for multiple queued confirmations per session. `Restricted` tier must remain blocked regardless of permissive mode unless explicitly elevated.
- **Root Cause**: Unbound oneshot map & singleton slot.
- **Recommended Correction**: Bind confirmation records to session + tool + arg digest + expiration, prevent overwrite, enforce `Restricted` denial.
- **Regression Test**: Verify replay, expired token approval, wrong-session/wrong-tool approval failure, and concurrent confirmation safety.
- **Status**: `Fixed` (`6637207`)

### BUG-014 | P0 | Confirmed | `crates/pico-tools/src/filesystem_tools.rs:69,149-162`
- **Observed Behaviour**: `ReadFileTool` reads the entire file (`std::fs::read_to_string(&safe_path)` at `line 69`) into memory BEFORE truncating to `max_lines`. Reading a 10 GB log or binary file causes multi-gigabyte memory spike / OOM crash.
- **Expected Behaviour**: Check file metadata size before reading and read lines incrementally via buffered `BufReader` stopping once `max_lines` or max byte cap (e.g., 1 MB) is reached.
- **Root Cause**: Eager full-file read followed by `.lines().take(max_lines)`.
- **Regression Test**: Read a >2 MB file with `max_lines = 10` and verify fast bounded return (no full read into memory) and binary/oversize guard.
- **Status**: `Fixed` (`6637207`)

### BUG-015 | P0 | Confirmed | `crates/pico-tools/src/catalog/windows.rs:141-612`, `crates/pico-tools/src/catalog/linux.rs:264-601`
- **Observed Behaviour**: Even in Category 1 (`filesystem`), concrete tool bugs exist:
  - Windows `find_files_by_size` (`windows.rs:141`) accepts `min_bytes` (`required: true`) but `adapter_args` is `"-Recurse -Path $path"` (drops `min_bytes`!).
  - Windows `find_files_by_date` (`:167`) accepts `after_date` (`required: true`) but drops it.
  - Windows `export_directory_tree` (`:601`) and `backup_acls` (`:493`) write files to disk yet declare `PermissionTier::ReadOnly`!
  - Windows `export_directory_tree` uses shell redirection `> $output_file` (`:611`) in `tree.exe` (which is ignored/invalid when executed without cmd shell wrapping or injects via unescaped path).
  - Linux `append_text` (`linux.rs:264`) uses `tee -a $path` with no text piped to stdin, hanging until 30s timeout!
  - Linux `create_junction` (`linux.rs:319`) executes `mount --bind` marked `LowRisk` (requires root and mounts filesystems).
  - Linux `encrypt_efs`/`decrypt_efs` (`linux.rs:565,577`) execute interactive `gpg` marked `LowRisk` without `--batch`.
- **Expected Behaviour**: Accurate command arguments utilizing all required parameters, non-blocking execution, and honest permission tiers (`Write`/mutating tools never `ReadOnly`).
- **Status**: `Fixed` (`4e79282`)

### BUG-016 | P0 | Confirmed | `crates/pico-tools/src/audio_tools.rs:55-82,120-135`
- **Observed Behaviour**: `SetMasterVolumeTool` sets Windows volume by sending `[char]174 * 50 + [char]175 * (level / 2)` fake keyboard media keypresses via `WScript.Shell.SendKeys` (`line 55`), hijacking desktop focus/keys, failing on headless/locked sessions, and taking noticeable delay; `MuteAudioTool` ignores its own `"mute": bool` parameter (`line 101-125`) and always toggles mute (`[char]173` / `pactl set-sink-mute @DEFAULT_SINK@ toggle`), plus ignores `Command::new("pactl")` return error with `let _ = ...` (`lines 75,131`).
- **Expected Behaviour**: Set exact scalar volume (`0.0..=1.0`) and idempotent `mute(true|false)` directly via Windows Core Audio WASAPI COM `IAudioEndpointVolume` and Linux `wpctl` / `pactl` / `amixer`, returning real hardware state or explicit error.
- **Root Cause**: Prototype shortcut via `WScript.Shell` keystrokes.
- **Status**: `Fixed` (`4e79282`, `99634e2`)

### BUG-017 | P0 | Confirmed | `crates/pico-inference/src/ranking.rs:136-185`, `crates/pico-decision/src/engine.rs:195-280`
- **Observed Behaviour**: `CrossEncoderRanker::rank_candidates` passes each candidate's heuristic weighted sum (`raw in [0, 1]`) independently through `platt_calibrate_with_temperature` (`a = 10.0, b = -5.0`). A candidate scoring `raw = 0.78` becomes `0.943` and runner-up `raw = 0.72` becomes `0.900`. Simultaneously reporting `94%` and `90%` while margin is `0.043` confuses both downstream ambiguity logic and users, especially when `Calibrator::calibrated_probabilities` (`calibration.rs:92`) was written specifically to combine relative softmax and absolute plausibility yet is never called! Furthermore, `DecisionEngine::decide` runs argument extraction only AFTER selecting `scored[0]` (`engine.rs:266`), never checking whether runner-up candidates satisfy required schemas/args better.
- **Expected Behaviour**: Clearly separate (1) ranking score, (2) normalized candidate distribution via temperature-scaled softmax, (3) well-calibrated executability confidence (avoiding `a=10, b=-5` saturation), and (4) candidate-aware argument feasibility; validate fitted calibration on held-out realistic utterances.
- **Status**: `Fixed` (`4e79282`)

### BUG-018 | P0 | Confirmed | `crates/pico-decision/src/engine.rs:195-313`, `crates/pico-orchestrator/src/coordinator.rs:243-446`
- **Observed Behaviour**: Clarification flows for ambiguous intents AND missing required arguments rely on stateless prompt strings (`ClarificationNeeded`, `MissingArguments`) with no structured candidate list or schema state stored per session. `DecisionEngine::decide` skips clarification for `open_application` / `calculator` (`engine.rs:231-233` dirty edits), while still producing spurious clarifications for `"Mute sounds"` (`audio.mute` vs `audio.set_volume`) and `"increase brightness"` (`windows.display.create_monitor` vs `query_monitor`).
- **Expected Behaviour**: Unified `ClarificationState` across candidate disambiguation and slot-filling (`MissingArguments`), retaining candidate options, extracted args, original request, and TTL, while improving intent + tool + verb discrimination so clear single-intent requests execute deterministically.
- **Status**: `Fixed` (`4e79282`)

---

## P1: Major Functionality, Architecture & Reliability Defects

### BUG-019 | P1 | Confirmed | `crates/pico-audio/src/playback.rs:12,104-169`, `crates/pico-audio/src/capture.rs:88-137`
- **Observed Behaviour**: Audio callbacks acquire `Mutex<VecDeque<f32>>` (playback) and heap-allocate `Vec<f32>`/`Vec<i16>` plus invoke `tokio::sync::mpsc::Sender` inside OS high-priority real-time audio threads. Contention or allocator locks inside callbacks can cause audio glitches, underruns, and priority inversion.
- **Expected Behaviour**: Lock-free bounded SPSC ring buffers (`rtrb` or atomic ring buffer) between capture/playback threads and OS audio callbacks with zero locks and zero allocations in the real-time callback closure.
- **Status**: `Fixed` (`4e79282`)

### BUG-020 | P1 | Confirmed | `crates/pico-audio/src/capture.rs:60-137`, `crates/pico-audio/src/playback.rs:92-169`
- **Observed Behaviour**: Capture handles only `SampleFormat::F32` and `I16` (omitting `I32`/`U8` and ignoring stereo/multi-channel subtle edge cases); Playback handles only `SampleFormat::F32` and `I16` (omitting `I32`/`U8` supported by several Linux ALSA devices). If device format is unsupported, `build_output_stream` fails silently or hits an `Err` branch without format conversion.
- **Expected Behaviour**: Normalize all common `cpal::SampleFormat` variants (`F32`, `I16`, `I32`, `U8`, `U16`) via shared sample conversion helpers.
- **Status**: `Fixed` (`4e79282`)

### BUG-021 | P1 | Confirmed | `crates/pico-audio/src/resampler.rs:18-65`
- **Observed Behaviour**: `AudioConverter` uses first-order linear interpolation (`mono[idx_floor] * (1 - frac) + mono[idx_ceil] * frac`) when downsampling (e.g. 48 kHz -> 16 kHz) without a low-pass anti-aliasing filter, folding frequencies > 8 kHz (sibilants, fan noise, hiss) into the speech band and degrading STT accuracy; additionally, it retains no fractional phase state across chunk boundaries, causing phase discontinuities every 20ms.
- **Expected Behaviour**: Stateful windowed-sinc / polyphase anti-aliased resampler maintaining boundary history across chunks for arbitrary rate ratios (`44.1k <-> 48k <-> 22.05k <-> 16k`).
- **Status**: `Fixed` (`4e79282`)

### BUG-022 | P1 | Confirmed | `crates/pico-voice/src/vad.rs:57-146`
- **Observed Behaviour**: `VadProcessor` measures `speech_start` and `silence_start` using wall-clock `std::time::Instant::now()`. Under slow chunk arrivals, offline WAV batch tests, or thread preemption, VAD duration calculations become non-deterministic. Furthermore, `speech_buffer` starts pushing only after `consecutive_active >= 2` frames (`~64ms` late), clipping initial plosives/consonants of short words like `"first"`, `"stop"`, `"mute"`.
- **Expected Behaviour**: Compute speech/silence durations strictly from accumulated sample counts (`samples / sample_rate`), keep a ~150ms circular pre-roll buffer prepended upon speech onset, and expose explicit unit-tested noise-floor adaptation and cancellation.
- **Status**: `Fixed` (`4e79282`)

### BUG-023 | P1 | Confirmed | `crates/pico-voice/src/stt.rs:24-55`
- **Observed Behaviour**: `SpeechNormalizer::normalize`:
  1. `FILLER_RE` strips `\blike\b`, corrupting `"sounds like"`, `"open something like notepad"`.
  2. Line 41-50 strips EVERY `is_ascii_punctuation()` character (`c.is_ascii_punctuation()`), destroying decimal points (`"3.5"` -> `"35"`), Windows paths (`C:\Users` -> `CUsers`), Unix paths (`/etc/hosts` -> `etchosts`), URLs, hyphens (`twenty-one`), and math expressions (`2 + 2` -> `2  2`, `5 * 4` -> `5  4`, `10 / 2` -> `10  2`, completely breaking spoken calculator queries!).
- **Expected Behaviour**: Strip only conversational edge punctuation (`,`, `?`, `!`, trailing `.`) outside paths/numbers/expressions, preserve mathematical operators (`+ - * / ^ % ( ) .`), URLs, file paths, and keep meaningful uses of `"like"`.
- **Status**: `Fixed` (`4e79282`)

### BUG-024 | P1 | Confirmed | `crates/pico-voice/src/stt.rs:81-175`
- **Observed Behaviour**:
  1. `AsrDaemon::spawn` hardcodes Python executable `"python"` (often `python3` on Linux) and sets `.stderr(Stdio::null())`, hiding Python import/model-load failures.
  2. `AsrDaemon::transcribe` blocks indefinitely on `self.stdout.read_line(&mut response)` with no timeout, freezing the entire voice pipeline if Python hangs.
  3. `PlatformSpeechRecognizer::transcribe_sync` ignores `"whisper"` and `"native_sapi"` engine selections cleanly when falling back, and temp WAV creation uses non-RAII cleanup prone to leaving orphan `.wav` files on errors/panics (`stt.rs:260+`).
- **Expected Behaviour**: Configurable Python discovery (`python3` / `python` / venv), bounded read timeouts with watchdog restart, clean Whisper executable discovery (`whisper-cli` / `whisper.exe` / `main`), and RAII temp file cleanup.
- **Status**: `Fixed` (`4e79282`)

### BUG-025 | P1 | Confirmed | `scripts/asr_service.py:22`, `scripts/whisper_daemon.py:21`, `scripts/whisper_transcribe.py:22`
- **Observed Behaviour**: All 3 Python ASR scripts assume `sampwidth == 1` maps to `uint8 / 128.0 - 1.0`, and do not handle 32-bit float/PCM (`sampwidth == 4`) or stereo -> mono downmix or sample-rate mismatch (`framerate != 16000`). Furthermore, `asr_service.py` prints nothing structured if model files are missing, emits raw tracebacks to suppressed stderr, and does not check path traversal on incoming WAV paths.
- **Expected Behaviour**: Support `sampwidth` in `(1, 2, 4)`, downmix multi-channel WAVs cleanly, validate incoming path stays inside temp/sandbox dir, and emit clean JSON status/error objects over stdio.
- **Status**: `Fixed` (`4e79282`)

### BUG-026 | P1 | Confirmed | `crates/pico-decision/src/index.rs:282-304`
- **Observed Behaviour**: `ToolIndex::tokenize` keeps `.` and `_` inside tokens (`.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')`). Tool IDs like `windows.filesystem.delete_file` stay indexed as the single token `"windows.filesystem.delete_file"`, so BM25 searching for `"delete"` or `"file"` gives **zero ID/name match** unless those words happen to appear in the description!
- **Expected Behaviour**: Split identifiers on `.`, `_`, `-`, `/` and camelCase boundaries so both full IDs and individual component words (`windows`, `filesystem`, `delete`, `file`) are indexed with appropriate weighting.
- **Status**: `Fixed` (`4e79282`)

### BUG-027 | P1 | Confirmed | `crates/pico-decision/src/index.rs:59-94`
- **Observed Behaviour**: `ToolIndex::add_entry` calls `recompute_avg_doc_length()`, which iterates over ALL `N` indexed documents on every insertion (`O(N^2)` startup cost when registering thousands of tools) and does not deduplicate duplicate tool names cleanly.
- **Expected Behaviour**: Maintain running `total_doc_len: usize` updated in `O(1)` per `add_entry`, and overwrite/update or deduplicate existing tool names cleanly.
- **Status**: `Fixed` (`4e79282`)

### BUG-028 | P1 | Confirmed | `crates/pico-decision/src/engine.rs:156`
- **Observed Behaviour**: `DecisionEngine::decide` passes hardcoded `15` to `retrieve_candidates(trimmed, 15)`, ignoring `DecisionConfig.max_candidates` stored on `self.config`.
- **Expected Behaviour**: Honor `cfg.max_candidates.max(1)` cleanly.
- **Status**: `Fixed` (`4e79282`)

### BUG-029 | P1 | Confirmed | `crates/pico-decision/src/argument.rs:118-165`
- **Observed Behaviour**: Argument extraction sets hardcoded fallback defaults (`path = "."`, `action = "get"`) and mis-assigns `application`/`target` from the whole raw sentence (`"Close brave"` -> `application: "Close brave"`, `target: "Close brave"`), while missing structured extraction for OS operations (screen brightness delta/percent, close app name, search pattern vs directory, timer minutes/hours vs seconds, mute `true`/`false`).
- **Expected Behaviour**: Intent-aware slot extraction stripping verb prefixes (`open`, `launch`, `close`, `quit`, `kill`, `mute`, `unmute`, `set`), validating parsed types against JSON schema (`integer`, `boolean`, `number`, `string`, enum), and surfacing genuine `missing_required` args instead of silent bogus strings.
- **Status**: `Fixed` (`4e79282`)

### BUG-030 | P1 | Confirmed | `crates/pico-orchestrator/src/verification.rs:71-335`
- **Observed Behaviour**: `ObjectiveVerifier`:
  - `verify_process_termination` checks only `pid` and skips verification if tool was invoked with `name`.
  - `verify_audio_operation` returns `verified: true, confidence: 0.95` blindly without checking actual OS audio volume/mute state or result fields (`verification.rs:302-319`).
  - `verify_generic_execution` considers any execution without `"error"` verified (`confidence: 0.90`).
  - No verifier exists for `open_application`, `write_file` partial/overwrite metadata, `manage_timer`, or catalog tool non-empty/exit-code checks.
- **Expected Behaviour**: Independent state verification where observable (WASAPI/pactl volume readback for audio, process table presence check for launch/kill, file mtime/bytes check for writes, non-zero exit verification for commands).
- **Status**: `Fixed` (`4e79282`)

### BUG-031 | P1 | Confirmed | `crates/pico-tools/src/registry.rs:12-49`
- **Observed Behaviour**: `ToolRegistry` registers both Windows (1,056) and Linux (1,056) catalog tools simultaneously into the same runtime (`get_massive_tool_catalog_with_sandbox`), regardless of host OS. BM25 retrieval and Decision ranking on Windows waste half the catalog space on Linux `/proc` and `systemctl` tools (and vice versa on Linux). Furthermore `get_tool_definitions` iterates an unordered `HashMap` producing non-deterministic ordering.
- **Expected Behaviour**: Register only host-OS-applicable tools (`target_os == current_os || target_os == All`) at runtime (while exposing `catalog_for_os` for verification tests), and sort definitions deterministically.
- **Status**: `Fixed` (`4e79282`)

### BUG-032 | P1 | Confirmed | `apps/pico-desktop/src/main.rs:8,125,226,235,815,972`
- **Observed Behaviour**:
  1. `pico-desktop` embeds the full `PicoRuntime` + SQLite DBs + `AsrDaemon` (622 MB ONNX model) inside the UI process, ballooning RAM far above companion budget.
  2. `ctx.request_repaint_after(Duration::from_millis(16))` (`line 226`) wakes the GUI thread at 60 FPS continuously even when idle or minimized.
  3. `PlatformSysInfo::get_system_stats()` runs synchronously inside the UI `update()` path (`line 235`).
  4. `"Mic: PAUSED"` / `"Mic: ACTIVE"` toggle button (`lines 815-817`) flips a local UI boolean while capture keeps running continuously.
- **Expected Behaviour**: Decouple UI into a dedicated lightweight companion + tray process (`< 50 MB` RSS, `< 1%` idle CPU) communicating over versioned local IPC (Windows Named Pipe / Linux Unix domain socket) with a separate voice/assistant daemon, using purely event-driven repaints and real working controls (media, volume, timers, Pomodoro, notes, stash, tray, hotkey).
- **Status**: `Fixed` (`99634e2`)

### BUG-033 | P1 | Confirmed | `crates/pico-learning/src/evaluator.rs:112-135`, `crates/pico-learning/src/trainer.rs:47-72`
- **Observed Behaviour**:
  1. `ModelEvaluator::build_standard_benchmark` hardcodes oracle features: target candidate gets `intent_affinities: [1.0, 0.1, 0.05]` and `historical_scores: [0.9, 0.5, 0.5]` (`lines 129-130`), causing every model to evaluate artificially high accuracy regardless of actual retrieval/ranking quality.
  2. `RankingTrainer::train_step` updates weights with hardcoded constants (`grad_intent += weight * 0.2; grad_lexical += 0.1; grad_history += 0.05`, `lines 58-60`) rather than computing actual per-feature score differentials `feature(target) - feature(chosen)`.
- **Expected Behaviour**: Independent held-out evaluation computing real `IntentClassifier`, lexical overlap, semantic similarity, and history features per candidate without label leakage, plus genuine pairwise margin gradient computation on feature vectors.
- **Status**: `Fixed` (`4e79282`)

### BUG-034 | P1 | Confirmed | `crates/pico-inference/src/calibration.rs:12-25,137-190`, `crates/pico-orchestrator/tests/benchmark_test.rs:588-615`
- **Observed Behaviour**: `PlattParameters::default()` hardcodes `a = 10.0, b = -5.0`, compressing scores `[0.55..0.95]` into `[0.62..0.99]`; calibration is never fit from real held-out logits, and `benchmark_test.rs:588-615` verifies `evaluate_calibration` using hardcoded synthetic tuples `[(0.95, true), (0.92, true), ...]` rather than real model predictions!
- **Expected Behaviour**: Fit `PlattParameters` (logistic regression / NLL minimization) on real held-out validation predictions and evaluate ECE/Brier/reliability diagrams on real engine outputs.
- **Status**: `Fixed` (`4e79282`)

### BUG-035 | P1 | Confirmed | `crates/pico-orchestrator/tests/e2e_turn_test.rs:240-303`
- **Observed Behaviour**: `test_e2e_app_launch_and_volume` actually spawns Notepad, Brave, and Chrome on the developer/CI machine (causing Brave/Chrome child processes to inherit cargo test stdout/stderr pipe handles and hang `cargo test` indefinitely!) and sends 50 real Windows volume keypresses during unit testing.
- **Expected Behaviour**: E2E decision/execution tests must be hermetic and safe on headless CI (`ubuntu-latest`, `windows-latest`) using sandboxed temp paths / injectable OS execution drivers or safe processes, never hijacking user browsers or hardware speakers during `cargo test`.
- **Status**: `Fixed` (`4e79282`)

### BUG-036 | P1 | Confirmed | `crates/pico-tools/src/power_tools.rs:33-105`
- **Observed Behaviour**: `LockScreenTool` and `SleepSystemTool` spawn OS power commands (`rundll32.exe user32.dll,LockWorkStation`, `systemctl suspend`, etc.) using blocking `std::process::Command::output()` inside async `execute`, potentially locking or suspending a machine if mis-triggered without confirmation enforcement, and blocking Tokio worker threads.
- **Expected Behaviour**: Enforce `ConfirmationRequired`, run blocking subprocess calls via `tokio::process::Command` with timeout, and validate clean exit codes and `PermissionTier`.
- **Status**: `Fixed` (`6637207`)

### BUG-037 | P1 | Confirmed | `crates/pico-tools/src/command_tools.rs:75-115`
- **Observed Behaviour**: `ExecuteShellCommandTool` executes any binary name and argument array (`Command::new(command).args(&args_list)`) without command-name policy checking beyond `ConfirmationRequired`; when a shell binary (`cmd`, `powershell`, `pwsh`, `bash`, `sh`) is passed, no inner command check runs, and stdout/stderr are buffered without size cap (OOM risk on noisy commands like `dir /s C:\`).
- **Expected Behaviour**: Cap captured stdout/stderr bytes (e.g., 256 KB per stream), enforce explicit interactive confirmation, block bypass flags when `Strict` policy forbids raw shells, and verify clean process-tree kill on timeout across Windows and Linux.
- **Status**: `Fixed` (`6637207`)

### BUG-038 | P1 | Confirmed | `crates/pico-platform/src/apps.rs:170-320`
- **Observed Behaviour**: `PlatformApps::list_processes` spawns `powershell.exe -Command "Get-Process ..."` (slow ~200-400ms startup on every call) instead of native Win32 Toolhelp32 APIs (`CreateToolhelp32Snapshot`), and `kill_process` shells out to `taskkill.exe` / `kill -9` instead of native Win32 `OpenProcess` + `TerminateProcess` / libc or clean validation (allowing PID `0` / `4` / self PID without guard!).
- **Expected Behaviour**: Protect current PID (`std::process::id()`) and critical PID 0/1/4 from self-termination, validate non-zero exit codes accurately, and prefer fast native OS enumeration (`CreateToolhelp32Snapshot` on Windows, direct `/proc` read on Linux).
- **Status**: `Fixed` (`6637207`)

### BUG-039 | P1 | Confirmed | `crates/pico-tools/src/system_tools.rs:82-125`
- **Observed Behaviour**: `KillProcessTool` resolves process `name` (`"Kill chrome"`) inconsistently or kills only the first PID returned without graceful terminate attempt / self-PID guard, and reports success even when a target process had multiple worker processes remaining.
- **Expected Behaviour**: Strip action prefixes safely, protect self/system critical processes, require confirmation, terminate matching user processes or requested PID cleanly, and verify termination.
- **Status**: `Fixed` (`6637207`)

### BUG-040 | P1 | Confirmed | `crates/pico-tools/src/utility_tools.rs:75-138`
- **Observed Behaviour**: `ManageTimerTool` spawns a detached `tokio::spawn(async move { sleep(...).await; warn!(...) })` that ONLY logs a `tracing::warn!` after sleeping. No OS notification, no persistent timer storage, no `PicoEvent` emission, no way to list/pause/cancel timers, and zero persistence across restarts.
- **Expected Behaviour**: Manage persistent timers with wall-clock UTC deadlines (`chrono::DateTime<Utc>`), emit runtime notifications/events, support create/list/pause/resume/cancel, and synchronize with the desktop companion timer panel.
- **Status**: `Fixed` (`99634e2`)

### BUG-041 | P1 | Confirmed | `crates/pico-security/src/voice_auth.rs:100-125`
- **Observed Behaviour**: `VoiceAuthRequest::new` accepts arbitrary expiry timestamps and `VoiceAuthorizer::evaluate_utterance` relies on regex lists without rejecting mixed/contradictory utterances such as `"yes wait no cancel"` where both affirmative and negative clauses occur near punctuation or multi-clause speech; additionally `VoiceAuthRequest` tracks `resolved: bool` independently from `ChannelConfirmationManager`.
- **Expected Behaviour**: Prioritize any negation/cancellation anywhere in multi-clause utterances (`"yes, wait, cancel"` -> `Declined`), unify expiration and single-use consumption with `ChannelConfirmationManager`.
- **Status**: `Fixed` (`6637207`)

### BUG-042 | P1 | Confirmed | `crates/pico-voice/src/pipeline.rs:46-149`
- **Observed Behaviour**:
  1. `start_listening` spawns an unjoined background Tokio task (`line 59`) without storing a `JoinHandle` or abort signal cleanly across restarts, causing leaked background tasks if restarted.
  2. `VoicePipeline::new` starts `AudioPlayback` immediately (`line 28`), holding OS audio output handles open even in text-only CLI/test runs (causing unit tests on headless CI without sound cards to fail or hit mock fallback).
  3. During TTS playback (`speak`), `VoicePipeline::speak` returns immediately after copying bytes into `PlaybackHandle`, setting `is_speaking = true`, and barge-in relies on `AudioPlayback`'s `is_playing` flag which updates asynchronously on the audio thread.
- **Expected Behaviour**: Lazy or optional audio device init for non-voice tests, proper `CancellationToken`/task handle management, clean playback-completion polling / notification, and verified barge-in latency metrics.
- **Status**: `Fixed` (`4e79282`)

### BUG-043 | P1 | Confirmed | `crates/pico-core/src/traits.rs:73-84`
- **Observed Behaviour**: `SpeechSynthesizer::synthesize(&self, text: &str) -> Result<Vec<u8>>` returns an untyped byte vector with no format descriptor (`sample_rate`, `channels`, `sample_format`, container vs raw PCM), forcing every caller to guess formats. Also lacks `list_voices()` and `engine_name()`.
- **Expected Behaviour**: Return typed `SynthesizedAudio` (`samples: Vec<f32>`, `sample_rate: u32`, `channels: u16`, `voice: String`, `engine: String`) plus voice discovery APIs.
- **Status**: `Fixed` (`4e79282`)

### BUG-044 | P1 | Confirmed | `crates/pico-core/src/config.rs:319-339`, `configs/pico.toml:57-61`
- **Observed Behaviour**: `DesktopConfig` defines `port`, `bind_address`, `auto_open_browser`, `theme` left over from an unimplemented web dashboard, while missing actual desktop companion configuration (companion socket/pipe endpoint, global hotkey, idle/tray behavior, companion state path).
- **Expected Behaviour**: Align `DesktopConfig` and `configs/pico.toml` with the native companion + IPC architecture (while keeping serde `default` backward compatibility for old keys).
- **Status**: `Fixed` (`4e79282`, `99634e2`)

### BUG-045 | P1 | Confirmed | `crates/pico-orchestrator/src/coordinator.rs:285-535`
- **Observed Behaviour**: Compound queries split on `" and then "` / `" then "` / `";"` via `split_compound_queries`, yet `coordinator.rs` cannot carry an output/reference or clarification cleanly between sub-steps, and loop detection (`line 231+`) treats two legitimate similar operations differently only via exact `(action, args)` tuples while `test_e2e_loop_detection` asserts `contains("already performed") || contains("42")` (an OR assertion masking whether loop detection actually fired!).
- **Expected Behaviour**: Deterministic loop-detection verification, explicit tests asserting both exact behavior paths without `a || b` escape hatches.
- **Status**: `Fixed` (`4e79282`)

### BUG-046 | P1 | Confirmed | `crates/pico-security/src/audit.rs:21-85`
- **Observed Behaviour**: `AuditLogger::new` does not create parent directories before appending to `audit_log_path`, silently dropping logs if `logs/` doesn't exist; `mask_sensitive_data` only masks JSON object keys named `"password"`, `"token"`, `"secret"`, `"api_key"` etc., leaving bearer tokens, AWS keys (`AKIA...`), GitHub PATs (`ghp_...`, `github_pat_...`), or connection strings inside arbitrary JSON string arguments or error messages completely unmasked.
- **Expected Behaviour**: Create parent directory lazily, mask both sensitive keys AND regex-matched secret patterns (`Bearer ...`, `AKIA...`, `ghp_...`, `sk-...`, URL basic auth `https://user:pass@...`) inside string leaves, and report write errors via tracing warning.
- **Status**: `Fixed` (`6637207`)

---

## P2: Significant Quality, Usability & Maintainability Defects

### BUG-047 | P2 | Confirmed | `apps/pico-desktop/src/main.rs:242,245,823,1037,1059,1085`
- **Observed Behaviour**: Unformatted Rust syntax (`cargo fmt --check` fails with 4 diffs) and 2 Clippy lints (`unnecessary_map_or`, `collapsible_if`).
- **Expected Behaviour**: Clean `cargo fmt` and zero `cargo clippy --workspace --all-targets --all-features -- -D warnings` warnings.
- **Status**: `Fixed` (`99634e2`)

### BUG-048 | P2 | Confirmed | `crates/pico-decision/src/argument.rs:471,489`, `crates/pico-orchestrator/src/coordinator.rs:36`
- **Observed Behaviour**: 3 pre-existing Clippy lints (`manual_strip` in `argument.rs:471,489`, `type_complexity` in `coordinator.rs:36`) failing CI's `-D warnings` gate.
- **Expected Behaviour**: Zero Clippy warnings workspace-wide.
- **Status**: `Fixed` (`6637207`)

### BUG-049 | P2 | Confirmed | `.gitignore:21`
- **Observed Behaviour**: `.gitignore` ignores `models/downloads/` only, leaving downloaded subdirectories (`models/parakeet-tdt-0.6b-v2-int8/encoder.int8.onnx`, 622 MB; `models/moonshine-tiny-en-int8/`, 119 MB) and root binaries (`pico-desktop.exe`) untracked and exposed to accidental `git add .` (exceeding GitHub's 100 MB limit).
- **Expected Behaviour**: Ignore `models/*/` (preserving any tracked README/manifest), `*.onnx`, `*.exe`, `__pycache__/`, `scripts/__pycache__/`.
- **Status**: `Fixed` (`1b3c960`, `ad71cbb`)

### BUG-050 | P2 | Confirmed | `crates/pico-platform/src/sysinfo.rs:107`
- **Observed Behaviour**: `get_windows_stats()` hardcodes `"Windows 11 x64".to_string()` instead of querying `RtlGetVersion` or environment/registry/architecture dynamically.
- **Expected Behaviour**: Report actual Windows version/build and CPU architecture cleanly.
- **Status**: `Fixed` (`99634e2`)

### BUG-051 | P2 | Confirmed | `crates/pico-decision/src/synthesizer.rs:151-215`
- **Observed Behaviour**: Static template responses repeat generic wording (`"All local subsystems are operational and performing optimally"`), ignore failure state metadata when called directly, and format `ClarificationNeeded` options without clear numbered instructions or human-friendly descriptions (exposing raw internal IDs like `windows.display.create_monitor`).
- **Expected Behaviour**: Concise, clear, honest templates displaying tool descriptions and numbered actions (`1.` / `2.`), never claiming `"performing optimally"` without checking telemetry.
- **Status**: `Fixed` (`4e79282`)

### BUG-052 | P2 | Confirmed | `crates/pico-tools/src/utility_tools.rs:140-200`, `crates/pico-platform/src/clipboard.rs:8-154`
- **Observed Behaviour**: `ClipboardTool` delegates to `PlatformClipboard` which runs PowerShell `Get-Clipboard` / `Set-Clipboard` on Windows (spawning a ~200ms PS process per read/write and potentially converting line endings/trailing newlines) and tries `wl-paste`/`xclip`/`xsel` on Linux; `Set` sends text via stdin cleanly on Linux/Windows but fails without clear diagnostic on headless Linux when none of `wl-copy`/`xclip`/`xsel` exist, while `get_text()` silently returns `Ok(String::new())` (`line 55,59`) on failure!
- **Expected Behaviour**: Use native Win32 clipboard API (`OpenClipboard` / `GetClipboardData` / `SetClipboardData` via `windows` crate) or safe fallback on Windows, and return structured error when no clipboard provider exists on Linux instead of `Ok("")`.
- **Status**: `Fixed` (`6637207`)

### BUG-053 | P2 | Confirmed | `crates/pico-tools/src/network_tools.rs:39-78`
- **Observed Behaviour**: `PingHostTool` passes `-c 4` on Linux and `-n 4` on Windows, checks basic character allowlist, then parses raw stdout generically with no timeout around `Command::output()` and treats exit code != 0 inconsistently vs unreachable host.
- **Expected Behaviour**: Wrap with timeout, parse packet loss / latency metrics deterministically, and set `success = false` cleanly when host is unreachable or command fails.
- **Status**: `Fixed` (`6637207`)

### BUG-054 | P2 | Confirmed | `crates/pico-tools/src/calculator_parser.rs:371-474`
- **Observed Behaviour**: Evaluated functions handle `sqrt`, `sin`, `cos`, etc., but `evaluate_expression` checks `args` ranges only in parts and does not return structured domain-error messages on NaN/Infinity overflow (e.g. `10^400`), nor support spoken forms like `"25 percent of 200"` at parser/preprocessor level.
- **Expected Behaviour**: Reject non-finite results (`!val.is_finite()`) consistently with descriptive errors and handle percentage/spoken arithmetic normalization cleanly in `pico-decision`.
- **Status**: `Fixed` (`6637207`)

### BUG-055 | P2 | Confirmed | `crates/pico-memory/src/budget.rs:7-38`
- **Observed Behaviour**: `estimate_tokens` uses `text.len() / 4` (byte count instead of char/word count, misestimating UTF-8 CJK/Unicode text by 3-4x), and `budget_messages` drops middle messages without indicating truncation or honoring priority weights.
- **Status**: `Deferred` (byte-length estimator in `budget.rs` retained)

### BUG-056 | P2 | Confirmed | `crates/pico-memory/src/sqlite.rs:10-276`
- **Observed Behaviour**: Every async method in `MemoryManager` grabs a single blocking `std::sync::Mutex<rusqlite::Connection>` directly on Tokio async worker threads (blocking the async executor if SQLite encounters a 5s busy timeout!). Also `search_relevant_context` uses naive substring `LIKE` without sanitization of SQL LIKE wildcards (`%`, `_`).
- **Expected Behaviour**: Execute synchronous SQLite queries inside `tokio::task::spawn_blocking` and escape SQLite `LIKE` wildcards (`\`, `%`, `_`).
- **Status**: `Fixed` (`6637207`)

### BUG-057 | P2 | Confirmed | `crates/pico-learning/src/manager.rs:35-179`
- **Observed Behaviour**: Uses synchronous `rusqlite` `Mutex<Connection>` and initializes `top1_accuracy: 1.0, top3_recall: 1.0` (`lines 54-55, 83-84`) even before any evaluation has ever run!
- **Expected Behaviour**: Initialize initial/unverified model metrics accurately (e.g. evaluated at startup or `0.0` until evaluated) rather than fabricating `100%` initial accuracy.
- **Status**: `Fixed` (`4e79282`)

### BUG-058 | P2 | Confirmed | `crates/pico-inference/src/embeddings.rs:63-90`
- **Observed Behaviour**: `SemanticEmbedder` hashes character 3-grams and word tokens into a fixed 128-dim signed bag using FNV-1a. Short tool names vs natural sentences suffer hash collisions and lack synonym normalization (`"mute"` vs `"silence"`, `"quit"` vs `"close"` vs `"exit"`, `"volume"` vs `"louder"`).
- **Expected Behaviour**: Document accurately as deterministic lightweight lexical/subword hash projection (not neural Transformer embeddings), and enrich with curated synonym/domain expansion + BM25 field weighting.
- **Status**: `Fixed` (`4e79282`)

### BUG-059 | P2 | Confirmed | `crates/pico-tools/tests/catalog_verification_test.rs`
- **Observed Behaviour**: Catalog verification test only checks superficial metadata invariants (`count == 1056`), allowing 2,016 duplicate/non-functional `adapter_cmd` entries to pass CI completely unnoticed.
- **Expected Behaviour**: Fail test if two actions in a category share identical non-parameterized commands or declare invalid/missing placeholders.
- **Status**: `Fixed` (`4e79282`)

### BUG-060 | P2 | Confirmed | `README.md`, `PROJECT_STATUS.md`, `docs/*.md`, `site/index.html`
- **Observed Behaviour**: All documentation and `site/index.html` advertise `2,112 native OS tools`, `Cross-encoder neural scoring`, `lock-free ring buffers`, `Sub-15ms Barge-in`, `0.04 ms` benchmarks, and `60 FPS` GUI, directly contradicting observed code behavior. Also `site/index.html` and `docs/index.html` must stay synchronized (or clearly built from canonical source).
- **Expected Behaviour**: Honest, exact, evidence-backed documentation and clean `antislop`-compliant UI site accurately reflecting rebuilt capabilities and real measured metrics.
- **Status**: `Fixed` (`ad71cbb`)

### BUG-061 | P2 | Confirmed | `apps/pico-cli/src/main.rs:66-117,322-455`
- **Observed Behaviour**: Single `PendingConfirmation` tuple overwrites concurrent confirmations; no headless IPC daemon/service subcommand exists yet; terminal barge-in claim (`line 356`: `(<15ms)`) is unverified marketing copy.
- **Status**: `Fixed` (`99634e2`)

### BUG-062 | P2 | Confirmed | `crates/pico-core/src/events.rs:9-123`
- **Observed Behaviour**: Events lack `session_id` on `ClarificationNeeded`, `SpeechRecognized`, and `ToolExecutionStarted`/`Completed`, making multi-session UI or companion routing unable to isolate events per session cleanly.
- **Status**: `Partial` (`4e79282`; turn and clarification events include `session_id`)

### BUG-063 | P2 | Confirmed | `crates/pico-core/src/config.rs:144-185`
- **Observed Behaviour**: `VoiceConfig.validate()` accepts `tts_engine = "espeak"` and `"system"` while doc comment says `"native_sapi", "piper", or "cloud"`, and does not validate `stt_engine = "auto"`, `"parakeet"`, `"moonshine"`, `"native_sapi"`, `"whisper"`, `"mock"` consistently with `configs/pico.toml`.
- **Status**: `Fixed` (`4e79282`)

### BUG-064 | P2 | Confirmed | `crates/pico-platform/src/apps.rs:16-115`
- **Observed Behaviour**: Dirty user modification added hardcoded Windows paths (`LocalAppData\BraveSoftware\...`, `Program Files\Google\Chrome\...`) for ~10 apps; fails for Microsoft Store / UWP / custom-installed apps (Settings, Calculator URI/exe quirks, custom Steam/VSCode paths) and lacks clean Start Menu / App Paths registry (`HKCU`/`HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths`) discovery or Linux `.desktop` XDG lookup.
- **Expected Behaviour**: Search Windows Registry `App Paths`, `PATH`, common install dirs, Start Menu `.lnk` targets, and Linux `PATH` + XDG `.desktop` files without shell injection.
- **Status**: `Fixed` (`4e79282`, `99634e2`)

### BUG-065 | P2 | Confirmed | `crates/pico-decision/src/argument.rs:80-165`
- **Observed Behaviour**: Regexes run globally across query string without checking which tool parameters actually exist in `schema["properties"]`, sometimes inserting bogus `"target"` or `"path": "."` into tools that reject additional properties or require concrete paths!
- **Status**: `Fixed` (`4e79282`)

### BUG-066 | P2 | Confirmed | `crates/pico-tools/src/filesystem_tools.rs:90-168`
- **Observed Behaviour**: `WriteFileTool` enforces `additionalProperties: false` in schema, creates arbitrary nested directories via `create_dir_all(parent)` before writing, does not cap `content.len()`, and does not verify post-write against symlinks swapped into `parent` (`TOCTOU`).
- **Status**: `Fixed` (`6637207`)

### BUG-067 | P2 | Confirmed | `api/submit.js:25-56,106-285`
- **Observed Behaviour**: In-memory `rateMap` resets per serverless cold start (`line 25`); trusts client-supplied `x-forwarded-for` directly if spoofed outside proxy; uses wide-open `Access-Control-Allow-Origin: *` (`line 32,136`) while posting directly to `GITHUB_TOKEN`-backed GitHub Issues.
- **Status**: `Fixed` (`6637207`)

### BUG-068 | P2 | Confirmed | `.github/workflows/ci.yml:21-56`
- **Observed Behaviour**: Linux CI installs only `libasound2-dev` (`line 25`); when companion/voice features expand (e.g. `espeak-ng`, `dbus-1-dev` / `libx11` / `pkg-config` / tray dependencies), CI dependency install must match Linux requirements accurately. Additionally CI runs `cargo test --workspace` without verifying release RSS or binary execution isolation.
- **Status**: `Fixed` (`ad71cbb`, `6873408`)

### BUG-069 | P2 | Confirmed | `Run-Pico.bat:1-3`, `test.bat:18-42`, `test.sh:16-35`
- **Observed Behaviour**: `Run-Pico.bat` launches a stale root `pico-desktop.exe` binary outside `target/` which drifts out of sync with `cargo build`; `test.bat`/`test.sh` immediately launch `pico-desktop` in the background after building debug binaries.
- **Status**: `Fixed` (`ad71cbb`)

### BUG-070 | P2 | Confirmed | `crates/pico-audio/src/devices.rs:16-90`
- **Observed Behaviour**: `AudioDeviceManager` lists device names only (`AudioDeviceInfo { name }`), omitting supported sample rates, channel counts, default format, and whether a device is the default device, preventing audio diagnostics (`pico doctor`) from identifying format mismatches automatically.
- **Status**: `Fixed` (`4e79282`)

---

## P3: Minor Bugs, Edge Cases & Refinements

### BUG-071 | P3 | Confirmed | `crates/pico-decision/src/synthesizer.rs:109`
`snippet` takes 200 chars of `read_file` output and includes newlines unmodified into TTS speech, causing SAPI/espeak to read raw Markdown/JSON punctuation awkwardly.
- **Status**: `Fixed` (`4e79282`)

### BUG-072 | P3 | Confirmed | `crates/pico-voice/src/tts.rs:14-21`
Rate mapping `((speech_rate_multiplier - 1.0) * 10.0)` maps `2.0x` to SAPI `10` (excessively fast), whereas linear SAPI rate `-10..+10` is exponential in words/min (`+4` is already ~1.5x).
- **Status**: `Fixed` (`4e79282`)

### BUG-073 | P3 | Confirmed | `crates/pico-voice/src/stt.rs:32-54`
Stutter deduplication only compares exact adjacent lowercased tokens; `"set set"` collapses properly, `"uh set"` works, but two-word stutters (`"set the set the"`) remain.
- **Status**: `Deferred` (single-word dedupe fixed; multi-word stutters open)

### BUG-074 | P3 | Confirmed | `crates/pico-decision/src/intent.rs:373-425`
`is_system_intent`, `is_audio_intent`, `is_filesystem_intent`, and `is_process_intent` overlap on generic verbs like `"check"`, `"status"`, `"open"`, and `"list"`, causing unstable tie-breaking when multi-word queries hit multiple heuristics.
- **Status**: `Fixed` (`4e79282`)

### BUG-075 | P3 | Confirmed | `crates/pico-decision/src/argument.rs:125-135`
Time parser `(\d+)\s*(s|sec|seconds?|m|min|minutes?|h|hr|hours?)` matches only the first unit in `"1 minute 30 seconds"`, discarding `"30 seconds"`.
- **Status**: `Fixed` (`4e79282`)

### BUG-076 | P3 | Confirmed | `crates/pico-tools/src/calculator_parser.rs:191`
`Parser` recursion depth limit prevents stack overflow on `((((...))))`, yet lexer token count itself isn't bounded on multi-megabyte inputs.
- **Status**: `Partial` (`Parser` depth cap 32 enforced; lexer token bound deferred)

### BUG-077 | P3 | Confirmed | `crates/pico-core/src/session.rs:130-142`
`Message::new_tool_result` serializes the entire `ToolResult.content` JSON string without truncation guard into session history.
- **Status**: `Deferred`

### BUG-078 | P3 | Confirmed | `crates/pico-security/src/sandbox.rs:197-208`
`normalize_existing_path` strips `\\?\` prefix via string slicing, which turns UNC `\\?\UNC\server\share` (if ever reached) into `UNC\server\share` rather than rejecting `UNC` prefix explicitly post-canonicalization.
- **Status**: `Fixed` (`6637207`)

### BUG-079 | P3 | Confirmed | `crates/pico-platform/src/sysinfo.rs:147-180`
Linux `get_linux_stats()` inspects only `BAT0` for battery percent/charging, missing `BAT1` / `macsmc-battery` / `CMB0` on ThinkPads, Framework, or ARM laptops.
- **Status**: `Fixed` (`99634e2`)

### BUG-080 | P3 | Confirmed | `crates/pico-memory/src/sqlite.rs:264-280`
Memory search returns unsorted concatenation or newest-first without relevance ranking, and `decay_half_life_days` in `MemoryConfig` is never applied in `MemoryManager`.
- **Status**: `Deferred`

### BUG-081 | P3 | Confirmed | `crates/pico-learning/src/poisoning.rs:32-55`
`AntiPoisoningGuard` keeps an in-memory `VecDeque<DateTime<Utc>>` that resets on restart, allowing unlimited rate bypass across process restarts.
- **Status**: `Deferred`

### BUG-082 | P3 | Confirmed | `crates/pico-orchestrator/src/coordinator.rs:75-160`
`PicoRuntime::new` sets up `PathSandbox` without canonicalizing home/desktop directories cleanly or exposing diagnostic readiness status of voice STT/TTS backends via `PicoEvent`.
- **Status**: `Partial` (`PathSandbox` canonicalization fixed in `6637207`; voice `PicoEvent` readiness deferred)

### BUG-083 | P3 | Confirmed | `crates/pico-tools/src/filesystem_tools.rs:241`
`SearchFilesTool` hardcodes skip list `["target", ".git", "node_modules"]`, omitting `.venv`, `__pycache__`, `models/` which slows workspace searches when Python/ONNX artifacts exist.
- **Status**: `Deferred`

### BUG-084 | P3 | Confirmed | `crates/pico-core/src/error.rs`
`PicoError` maps diverse failures (`ToolExecution`, `Security`, `Audio`, `Voice`) to string payloads without machine-readable error codes for IPC or retry classification.
- **Status**: `Deferred` (remediated in subsequent pass; see [PICO_REMAINING_DEFECTS.md](PICO_REMAINING_DEFECTS.md))

---

## P4: Optional Improvements & Optimizations

### BUG-085 | P4 | Confirmed | `crates/pico-audio/src/resampler.rs`
No fast SSE/polyphase windowed-sinc resampling filter table for 1:3 (`16k <-> 48k`) and 147:320 (`22.05k <-> 48k`) common rates.
- **Status**: `Deferred` (`resampler` sinc runs in `0.120 ms`)

### BUG-086 | P4 | Confirmed | `crates/pico-voice/src/tts.rs`
No sentence chunking or pronunciation pre-processor for technical identifiers (camelCase, paths `C:\Users`, units `48 kHz`, percentages `75%`, acronyms `CPU`, `RAM`).
- **Status**: `Fixed` (`4e79282`)

### BUG-087 | P4 | Confirmed | `crates/pico-voice/src/vad.rs`
No calibration self-check or optional WebRTC-style spectral / zero-crossing speech validation alongside adaptive RMS.
- **Status**: `Deferred`

### BUG-088 | P4 | Confirmed | `crates/pico-decision/src/index.rs`
English suffix stemmer (`stem`) handles only basic plurals/`-ing`/`-ed`; adding light synonym normalization (`mute<->silence`, `close<->quit<->exit`, `loud<->volume`) improves recall cleanly.
- **Status**: `Fixed` (`4e79282`)

### BUG-089 | P4 | Confirmed | `crates/pico-orchestrator/tests/benchmark_test.rs`
Benchmarks run unparameterized loops in debug builds and emit pass/fail without writing structured reproducible JSON report metadata (target triplet, release vs debug).
- **Status**: `Partial` (`b776531`; debug/release CI budgets calibrated; JSON export deferred)

### BUG-090 | P4 | Confirmed | `apps/pico-cli/src/main.rs`
`pico-cli doctor` does not currently synthesize/verify sample TTS format conversion, STT daemon health check, or IPC socket/pipe readiness.
- **Status**: `Deferred`

### BUG-091 | P4 | Confirmed | `crates/pico-tools/src/registry.rs`
No registry schema self-validator ensuring every registered tool emits valid JSON Schema object with `properties` and `required` array.
- **Status**: `Partial` (`registry.rs` `type == "object"` check + `catalog_verification_test`)

### BUG-092 | P4 | Confirmed | `.github/workflows/ci.yml`
No dedicated workflow step checking that generated binaries stay under binary/memory smoke thresholds or checking `site/index.html` and `docs/index.html` parity.
- **Status**: `Fixed` (`ad71cbb`; CI checks `site/index.html` and `docs/index.html` parity)
