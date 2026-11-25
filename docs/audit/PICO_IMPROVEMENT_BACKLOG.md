# Pico Engineering Improvement Backlog

Exhaustive prioritized backlog of genuine architectural, audio/voice, decision, tool, security, desktop companion, testing, packaging, CI, and documentation improvements grounded in the forensic audit of commit `faf1a5a8da856a2b88eceb188e92aa1d32e401d9`.

**Legend**: Priority `P1` High | `P2` Medium | `P3` Refinement | `P4` Future Optimization. Complexity: `S` Small | `M` Medium | `L` Large. Status: `Open` | `In Progress` | `Done` | `Deferred`.

---

## A. Decision Engine & Clarification Pipeline

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-001 | Session-scoped `ClarificationState` FSM (`AmbiguousCandidate`, `MissingArguments`, `Confirmation`) | Eliminates global `Option` overwrite, cross-session leak, and forgotten choices (`coordinator.rs:36`) | Deterministic multi-turn follow-ups (`"the second one"`, `"option two"`, missing slot answers) | P1 | M | - | Unit + E2E tests verify multi-session isolation, TTL expiry, cancel, interrupt, all selection formats | Done (`4e79282`) |
| IMP-002 | Decouple clarification ordinals from `IntentClassifier::is_direct_conversation` | Fixes `"Set a timer for 30 seconds"` hijack (`intent.rs:329-353`) | Eliminates false chat fallback on valid commands | P1 | S | IMP-001 | `"seconds"`, `"first"`, `"none"` tested in realistic queries without hijacking | Done (`4e79282`) |
| IMP-003 | Fix probability model: separate softmax selection distribution from calibrated executability score | Stops mutually exclusive candidates both displaying 90-96% (`ranking.rs:162`) | Clear ambiguity margins; fewer false clarifiers and fewer low-confidence forced actions (`26%`) | P1 | M | - | Softmax sums to 1.0; out-of-scope confidence below threshold; Platt params fitted on held-out dev data | Done (`4e79282`) |
| IMP-004 | Candidate-aware joint argument feasibility check before selecting winner | Ensures `engine.rs` doesn't commit to `scored[0]` when missing critical required params cleanly present for `scored[1]` | Higher top-1 end-to-end execution accuracy | P1 | M | IMP-003 | Multi-candidate queries rank executable candidate cleanly | Done (`4e79282`) |
| IMP-005 | Word-boundary & synonym normalization in `ToolIndex::tokenize` | Splitting `windows.filesystem.delete_file` keeps components searchable by BM25 (`index.rs:282`) | Dramatically improved recall for tool names | P1 | S | - | Query `"delete file"` retrieves `delete_file` in top 3 | Done (`4e79282`) |
| IMP-006 | `O(1)` incremental `total_doc_len` tracking in `ToolIndex` | Fixes `O(N^2)` startup rebuild cost (`index.rs:86`) | Fast cold initialization | P2 | S | - | `add_entry` benchmark scales linearly `O(N)` | Done (`4e79282`) |
| IMP-007 | Deduplicate tool entries by name in `ToolIndex` & enforce `DecisionConfig.max_candidates` | Stops silent duplicate scoring and hardcoded `15` override (`engine.rs:156`) | Deterministic top-K behavior controlled by config | P2 | S | - | Updating entry replaces old doc cleanly; changing `max_candidates` changes retrieved count | Done (`4e79282`) |
| IMP-008 | Strict intent discrimination for open vs close/quit app & volume mute vs set & brightness | Stops `"Close brave"` -> `open_application` & `"Mute sounds"` -> `create_monitor` type failures | Eliminates dangerous mis-routes observed in production logs | P1 | M | IMP-005 | `"Close brave"`, `"Mute sounds"`, `"Unmute"`, `"Set brightness to 70%"` route deterministically | Done (`4e79282`) |
| IMP-009 | Structured `ArgumentResolution` supporting minutes/hours/seconds, brightness %, app aliases, boolean flags | Currently `"1 minute 30 seconds"` drops seconds and `"Close brave"` puts verb into `application` | Clean schema-validated JSON args without hacky fallbacks | P1 | M | - | Argument extraction suite covers compounds, quotes, numbers, units, paths, booleans | Done (`4e79282`) |
| IMP-010 | Gate `ResponseSynthesizer` success replies strictly on `result.success && verification.verified` | Stops Pico speaking `"Opened Close brave."` when action fails verification (`coordinator.rs:460`) | Zero false-positive user confirmations | P1 | S | IMP-011 | Tests prove failed tool or failed `ObjectiveVerifier` always produces failure/warning reply | Done (`4e79282`) |
| IMP-011 | Real `ObjectiveVerifier` checks for apps, audio volume/mute, process kill by name, and disk state | Prevents stubbed `verified: true` generic pass (`verification.rs:302`) | Independent verification evidence on real state mutations | P1 | M | IMP-024 | Audio/app/process verifiers inspect real readback or exact result schema | Done (`4e79282`) |
| IMP-012 | Rename misleading `"neural cross-encoder"` identifiers/docs cleanly | Accurate engineering claims matching linear ensemble scorer (`ranking.rs:49`) | Technical honesty across code, config, docs, UI | P2 | S | - | Code, `configs/pico.toml`, docs accurately document linear/calibrated scorer | Done (`ad71cbb`) |
| IMP-013 | Remove label leakage from `ModelEvaluator` & constant pseudo-grads from `RankingTrainer` | Currently evaluator injects oracle scores (`evaluator.rs:129-130`) | Genuine measurable continuous-learning progress on held-out pairs | P1 | M | IMP-003 | Real pairwise feature diff gradients + zero oracle vectors in benchmark dataset | Done (`4e79282`) |
| IMP-014 | Out-of-scope / low-confidence honest abstain response instead of forced `windows.power.*` match | Incident `"I'm not sure"` shouldn't suggest `enable_plan (26%)` | Graceful handling of smalltalk, unclear prompts, unsupported actions | P1 | S | IMP-003 | Ambiguous non-action inputs respond clearly without executing unrelated OS tools | Done (`4e79282`) |
| IMP-015 | Carry session context across compound queries (`A and then B`) with per-step rollback/halt | Improves multi-step workflows (`coordinator.rs:285`) | Clear compound execution trace & deterministic failure halt | P2 | M | IMP-001 | Compound execution tests verify clean pass and mid-step halt | Done (`4e79282`) |

---

## B. Audio Hardware, Buffering & Resampling

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-016 | Define canonical `AudioFormat` & `AudioBuffer` / `SynthesizedAudio` in `pico-core` | Eliminates implicit rate/channel guesses across crates (`traits.rs:76`) | Explicit format contract end-to-end | P1 | S | - | Every capture/synth/playback boundary declares rate, channels, encoding | Done (`4e79282`) |
| IMP-017 | Decode RIFF WAV headers cleanly vs raw PCM via `hound` helper | Fixes Linux `espeak-ng --stdout` playing `"RIFF"` bytes as audio (`tts.rs:165`) | Zero header clicks or format confusion | P1 | S | IMP-016 | WAV-vs-PCM detection test passes for 16k/22.05k/44.1k/48k WAVs and raw PCM | Done (`4e79282`) |
| IMP-018 | Resample playback audio to hardware `default_output_config().sample_rate()` + channel count | Fixes 3.0x Windows & 2.18x Linux "chipmunk" speed/pitch defect (`playback.rs:92`) | Natural human-speed pitch-accurate playback on all devices | P1 | M | IMP-016 | Playback frequency & duration tests verify 16k/22.05k -> 44.1k/48k accuracy | Done (`4e79282`) |
| IMP-019 | Stateful anti-aliased windowed-sinc resampler replacing raw linear interpolation | Eliminates aliasing distortion above 8 kHz and chunk-boundary clicks (`resampler.rs:55`) | Higher STT clarity & clean TTS playback | P1 | M | IMP-016 | THD+N / Nyquist attenuation test verifies stopband rejection and continuity | Done (`4e79282`) |
| IMP-020 | Lock-free SPSC ring buffers (`rtrb`) for capture and playback OS audio callbacks | Eliminates `Mutex<VecDeque>` priority inversion & heap allocs on RT threads (`playback.rs:12`, `capture.rs`) | Glitch-free low-latency audio under load | P1 | M | IMP-018 | Zero mutex/alloc inside cpal callbacks; unit stress test passes under concurrent push/pop | Done (`4e79282`) |
| IMP-021 | Support all `cpal::SampleFormat` variants (`F32`, `I16`, `I32`, `U8`, `U16`) cleanly | Ensures compatibility across diverse Linux ALSA/PipeWire and Windows WASAPI drivers | Fewer unsupported hardware errors | P2 | S | IMP-016 | Sample conversion roundtrip tests pass for all formats | Done (`4e79282`) |
| IMP-022 | Generation-stamped atomic barge-in playback flush (`clear` ring + track end-of-playback cleanly) | Accurate `is_playing()` and deterministic instant interruption on real speech | Fast interrupt without stale sample leakage | P1 | S | IMP-020 | Interrupt clears queued audio immediately and emits `PlaybackInterrupted` | Done (`4e79282`) |
| IMP-023 | Richer `AudioDeviceInfo` (`default_rate`, `channels`, `formats`, `is_default`) for diagnostics | Helps debug OS hardware negotiation failures (`devices.rs`) | Actionable `pico-cli doctor` audio report | P3 | S | - | `pico-cli doctor` prints exact negotiated sample rates & channel configs | Done (`4e79282`) |

---

## C. Text-to-Speech (TTS), STT & VAD

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-024 | Multi-backend TTS abstraction (`WinRT OneCore`, `Windows SAPI`, `Piper`, `SpeechDispatcher`, `eSpeak-NG`, `Mock`) | Honors `VoiceConfig.tts_engine`, discovers installed voices, prefers natural neural voices (`tts.rs`) | Natural-sounding intelligible speech with failover | P1 | L | IMP-016 | Configurable engine selection + voice discovery `list_voices()` works on Win/Linux | Done (`4e79282`) |
| IMP-025 | Explicit TTS failure propagation (never return `Ok(Vec::new())` on non-zero exit) | Prevents silent failure masking (`tts.rs:102`) | Accurate diagnostics & error logging | P1 | S | IMP-024 | Test asserts non-existent voice/failing backend returns `Err` | Done (`4e79282`) |
| IMP-026 | Speech prosody preprocessor & sentence segmentation | Normalizes file paths (`C:\foo`), units (`48 kHz`, `75%`), abbreviations (`CPU`, `UI`), pauses between sentences | Much higher intelligibility on technical responses | P2 | M | IMP-024 | Unit tests verify number/path/abbreviation normalization & chunking | Done (`4e79282`) |
| IMP-027 | Nonlinear perceptually-calibrated speaking rate mapping (SAPI/Piper/eSpeak) | Fixes overly aggressive `speech_rate = 1.5..2.0` acceleration (`tts.rs:16`) | Comfortable intelligible pacing across `0.7x..1.6x` | P3 | S | IMP-024 | Verified monotonic mapping tests | Done (`4e79282`) |
| IMP-028 | Sample-counted `VadProcessor` with 150ms circular pre-roll buffer & echo suppression | Replaces wall-clock `Instant::now()` jitter and prevents initial consonant clipping (`vad.rs:57`) | Reliable activation on short commands (`"stop"`, `"mute"`, `"first"`) | P1 | M | IMP-020 | Deterministic offline sample-chunk VAD tests pass regardless of wall-clock speed | Done (`4e79282`) |
| IMP-029 | Fix `SpeechNormalizer` to preserve `"like"` in phrases, math operators, decimals, paths | Stops `"like"` deletion and `.` / `+` / `/` destruction (`stt.rs:24-55`) | `"What is 3.5 + 2"` and paths survive STT normalization intact | P1 | S | - | Comprehensive STT normalizer tests covering fillers, numbers, paths, stutters | Done (`4e79282`) |
| IMP-030 | Hardened `AsrDaemon` + Whisper CLI discovery + read timeout + RAII temp WAV cleanup | Prevents hangs when Python subprocess stalls and cleans temp WAV files (`stt.rs:81-260`) | Reliable STT lifecycle across errors/restarts | P1 | M | - | Timeout & RAII cleanup tests verify zero leaked files and bounded wait | Done (`4e79282`) |
| IMP-031 | Fix `scripts/asr_service.py`, `whisper_daemon.py`, `whisper_transcribe.py` sample-width & path checks | Handles 1/2/4-byte WAVs, stereo downmix, resampling, structured error output | Eliminates Python ASR crashes on varied WAV inputs | P2 | S | - | Python unit/smoke check verifies 8/16/32-bit mono/stereo WAV handling | Done (`4e79282`) |

---

## D. Tool Catalog & Native OS Execution

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-032 | Purge 2,016 duplicated placeholder tools from `windows.rs` & `linux.rs` | Eliminates broken clones (`Get-Volume` x48, `Get-Process` x96, `Get-AudioDevice` x48) | Clean catalog where every registered tool is genuine and functional | P1 | L | - | Zero duplicated `(adapter_cmd, adapter_args)` across non-equivalent tools | Done (`4e79282`) |
| IMP-033 | Build verified curated OS tool catalog across Windows & Linux domains | Provides real file, process, system, audio, display, power, network, service, package, clipboard, timer tools | Honest, predictable tool execution on both OSes | P1 | L | IMP-032 | Every retained catalog entry verified for schema, params, permissions, and safe adapter | Done (`4e79282`) |
| IMP-034 | Replace `WScript.Shell SendKeys` audio hack with real WASAPI COM & `wpctl`/`pactl` tools | Eliminates 50x focus-stealing keystrokes (`audio_tools.rs:55`) and implements true `mute: bool` | Exact instant silent volume set/get & true mute/unmute | P1 | M | - | `SetMasterVolumeTool` & `MuteAudioTool` (with `mute` bool) verified via unit/backend checks | Done (`4e79282`, `99634e2`) |
| IMP-035 | Real display brightness adjustment tool (WMI / monitor API on Windows; `brightnessctl`/`/sys/class/backlight` on Linux) | Fixes `"increase brightness"` failing with `create_monitor` | Common laptop desktop request works natively | P1 | M | - | Parameterized brightness tool indexed, extracted, and tested | Done (`4e79282`) |
| IMP-036 | App close/focus tools + Start Menu / App Paths / `.desktop` discovery | Fixes `"Close brave"` & missing UWP/Store app launches (`apps.rs`) | Clean open AND close app workflows | P1 | M | SEC-001 | Launch/close tests pass with alias lookup & confirmation safety | Done (`4e79282`, `99634e2`) |
| IMP-037 | OS-filtered `ToolRegistry` registration at runtime | Prevents Linux tools polluting Windows BM25 retrieval (and vice versa) (`registry.rs`) | Sharper decision accuracy & lower memory | P1 | S | IMP-032 | Windows runtime loads Windows + Cross-OS tools only; catalog tests verify both sets | Done (`4e79282`) |
| IMP-038 | Native Win32 process enumeration (`CreateToolhelp32Snapshot`) & self-PID termination guard | Replaces ~300ms PowerShell startup overhead (`apps.rs:172`) and protects Pico from killing itself | Sub-millisecond process listing and safe termination | P2 | M | - | `list_processes` & `kill_process` tests include self-PID / PID 0-4 protection | Done (`6637207`) |
| IMP-039 | Native Win32 clipboard API + explicit error on missing Linux clipboard provider | Avoids PowerShell overhead & silent `Ok("")` on headless Linux (`clipboard.rs:55`) | Fast reliable clipboard operations | P2 | S | - | Clipboard error/roundtrip tests cleanly structured | Done (`6637207`) |
| IMP-040 | Persistent `ManageTimerTool` backed by wall-clock deadlines & event notifications | Fixes fire-and-forget log-only sleep task (`utility_tools.rs:126`) | Usable voice & companion timers persisting restarts | P1 | M | - | Timers persist, list, pause, cancel, and fire cleanly | Done (`99634e2`) |
| IMP-041 | Dynamic Windows OS version/arch detection (`RtlGetVersion`) & multi-battery Linux support | Replaces hardcoded `"Windows 11 x64"` & `BAT0`-only assumptions (`sysinfo.rs:107,167`) | Accurate cross-device hardware diagnostics | P2 | S | - | Sysinfo tests verify fields match OS queries | Done (`99634e2`) |

---

## E. Security Hardening

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-042 | Eliminate `cmd /C` & PowerShell quote injection in `apps.rs` (`SEC-001`) | Closes P0 command injection via `LowRisk` `open_application` | Zero shell interpretation on app launch | P1 | M | - | Injection test suite verifies metacharacters rejected | Done (`6637207`) |
| IMP-043 | Parameterize `catalog/adapter.rs` PowerShell args via environment variables (`SEC-002`) | Closes PowerShell AST breakout across all catalog tools | Parameter values cannot modify script control flow | P1 | M | - | Adversarial `$(...)`, `;`, `|` tests pass | Done (`6637207`) |
| IMP-044 | Strictly validate `/proc` & `/sys` dynamic tokens on Linux (`SEC-003`) | Blocks `../../etc/passwd` traversal in procfs/sysfs adapter | Strict containment inside `/proc` & `/sys` | P1 | S | - | Traversal payloads rejected with `PicoError::Security` | Done (`6637207`) |
| IMP-045 | Fix `SearchFilesTool` symlink/reparse escape & add scan limits (`SEC-004`) | Prevents leaking arbitrary directory listings outside sandbox | Symlink isolation verified | P1 | S | - | External symlink test returns zero external paths | Done (`6637207`) |
| IMP-046 | Fail-closed empty `allowed_roots`, block trailing `..`, expand credential blocklist, guard parent TOCTOU (`SEC-005`) | Prevents total sandbox bypass when `allowed_roots` is empty | Airtight path canonicalization | P1 | M | - | Adversarial path suite verifies all vectors blocked | Done (`6637207`) |
| IMP-047 | DNS-pinning `reqwest::dns::Resolve`, IPv6 transition blocking, bounded body read in `FetchWebContentTool` (`SEC-006`) | Closes DNS rebinding SSRF & OOM response DoS | Safe bounded web fetching | P1 | M | - | NAT64/6to4/rebind/oversize body unit tests pass | Done (`6637207`) |
| IMP-048 | Session/tool/arg-bound single-use `ChannelConfirmationManager` & concurrent queue (`SEC-007`) | Prevents confirmation replay, expiry bypass, and UI slot overwrites | Cryptographically sound approval binding | P1 | M | - | Replay, timeout, cross-session & concurrency tests pass | Done (`6637207`) |
| IMP-049 | Regex secret redaction in string leaves + auto-created logs dir in `AuditLogger` (`SEC-009`) | Prevents tokens inside CLI/tool arguments from leaking to plaintext logs | Clean redacted audit logs | P2 | S | - | Audit logger tests mask tokens inside strings/messages | Done (`6637207`) |
| IMP-050 | Bound read/write file sizes in `ReadFileTool` & `WriteFileTool` (`BUG-014`) | Prevents 10 GB file read OOM crashes | Safe predictable memory footprint | P1 | S | - | Large-file test confirms streaming cap enforcement | Done (`6637207`) |
| IMP-051 | Bound stdout/stderr capture & surface shell wrapper warning in `ExecuteShellCommandTool` (`SEC-008`) | Prevents memory DoS & invisible shell wrapper bypass | Predictable bounded shell execution | P2 | S | - | Oversized-output command test caps output cleanly | Done (`6637207`) |
| IMP-052 | Escape SQLite `LIKE` wildcards & offload blocking DB calls via `spawn_blocking` (`BUG-056`) | Prevents wildcard DoS and async executor stalls | Non-blocking responsive memory retrieval | P2 | S | - | Search with `%`/`_` returns literal matches only | Done (`6637207`) |
| IMP-053 | Remove dead `unsafe impl Send / Sync` from `AudioCapture` & `AudioPlayback` | Eliminates unnecessary `unsafe` blocks cleanly | Cleaner safe Rust surface | P2 | S | - | Zero `unsafe` in `pico-audio` | Done (`4e79282`) |
| IMP-054 | Persist `AntiPoisoningGuard` rate window cleanly & restrict `api/submit.js` CORS/IP parsing | Prevents restart rate-limit bypass & browser-origin spam | Stronger operational resilience | P3 | S | - | Rate limit verified; CORS configurable | Done (`6637207`, persisted guard in `b7f1595`) |

---

## F. Lightweight Desktop Companion & Runtime IPC (`apps/pico-desktop`)

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-055 | Separate headless assistant runtime daemon with versioned Named Pipe (Win) / Unix Socket (Linux) IPC (`SEC-010`) | Allows `pico-desktop` to shed 600+ MB ONNX/SQLite/decision bloat while communicating over authenticated local IPC | Enables `< 50 MB` companion RSS & shared CLI/tray assistant control | P1 | L | - | IPC integration test verifies auth handshake, prompt execution, confirmation, offline handling | Done (`99634e2`, `bdf6e3e`, `415b507`) |
| IMP-056 | Event-driven compact floating companion UI (`< 50 MB` RSS, `< 1%` idle CPU) | Eliminates 60 FPS continuous idle spinning & giant chat dashboard (`apps/pico-desktop`) | Minimal battery/CPU/memory impact suitable for always-on desktop companion | P1 | L | IMP-055 | Measured release RSS `< 50 MB`, measured idle CPU `< 1%` | Done (`99634e2`) |
| IMP-057 | Native OS Media Controller (WinRT GSMTC on Windows + MPRIS D-Bus on Linux) | Real track title, artist, album, artwork, playback state, play/pause/next/prev, session switcher | Responsive floating music HUD matching system media players | P1 | L | - | Live tests / graceful empty state when no media active | Done (`99634e2`) |
| IMP-058 | Artwork-driven dominant palette extraction & contrast-safe animated gradient theme | Matches Nusa-inspired clean companion aesthetic with >= 4.5:1 WCAG contrast on all extracted colors | Refined cohesive look across diverse album art | P2 | M | IMP-057 | `contrast-check` >= 4.5:1 verified across bright/dark/saturated palettes; fallback without art | Done (`99634e2`) |
| IMP-059 | Companion native System Volume Control (WASAPI `IAudioEndpointVolume` / Linux `pactl`/`wpctl` + external change sync) | Accurate volume slider, mute toggle, output device display synced with OS hardware | Instant hardware-accurate audio control without stealing focus | P1 | M | - | Changing OS volume updates companion UI; slider mutates system volume accurately | Done (`99634e2`) |
| IMP-060 | Companion Persistent Multi-Timers (labels, pause/resume/reset, wall-clock UTC, notifications) | Survives app restarts & sleep/wake accurately | Dependable everyday timer utility | P1 | M | - | Timers persist to JSON store, handle UTC wake drift, trigger OS alerts | Done (`99634e2`) |
| IMP-061 | Companion Pomodoro Engine (focus/short/long break, cycles, auto-advance, persisted daily stats, alerts) | Productivity loop built natively into companion | Structured deep-work workflow with history | P2 | M | IMP-060 | Cycle transition, persistence, and stats unit tests pass | Done (`99634e2`) |
| IMP-062 | Companion Quick Notes (instant capture, edit, delete, search, pin, tag, keyboard shortcuts, persistence) | Zero-friction desktop scratchpad | Instant note capture from global hotkey / tray | P2 | M | - | Full CRUD + search/pin/tag tests on local note store | Done (`99634e2`) |
| IMP-063 | Companion Persistent Drag-and-Drop Stash (files, URLs, text, clipboard paste, search, copy/open, safe missing-file handling) | Handy floating shelf matching Nusa companion workflow without risking user source files | Drag/drop & clipboard persistence across sessions | P1 | M | - | Drops persist metadata cleanly; never deletes/modifies originals; handles missing targets cleanly | Done (`99634e2`) |
| IMP-064 | Cross-platform System Tray Icon, Minimize-to-Tray, Global Hotkey toggle, Clean Quit | Makes companion effortless to summon and keep out of taskbar clutter | True desktop utility ergonomics | P1 | M | IMP-056 | Single-click / hotkey toggle hides/shows cleanly; Quit exits all threads cleanly | Done (`99634e2`) |
| IMP-065 | Lightweight justified companion extras (quick math scratch calc via `calculator_parser` + battery/network pill + assistant prompt bar over IPC) | High daily utility at negligible RSS/CPU cost | Cohesive assistant+utility companion | P3 | M | IMP-055 | Math evaluator works offline without daemon; assistant bar connects/disconnects gracefully | Done (`99634e2`) |

---

## G. Testing, Benchmarks, CI & Documentation

| ID | Improvement | Why It Matters | Expected Benefit | Pri | Cmp | Deps | Acceptance Criteria | Status |
|---|---|---|---|---|---|---|---|---|
| IMP-066 | Replace browser-launching / volume-mashing `test_e2e_app_launch_and_volume` with safe hermetic & injectable tests | Prevents `cargo test` hanging on inherited browser handles and disrupting user hardware (`e2e_turn_test.rs:240`) | Reliable fast `cargo test --workspace` on local + CI | P1 | M | - | `cargo test --workspace` finishes cleanly without launching Chrome/Brave/Notepad | Done (`4e79282`) |
| IMP-067 | Comprehensive Decision & Clarification test matrix (ordinals, rephrase, missing-args, expiry, multi-session, calibration) | Locks down every Goal 2 invariant | Zero regression in conversational state | P1 | M | IMP-001..015 | All scenarios covered by deterministic tests | Done (`4e79282`) |
| IMP-068 | Comprehensive Audio DSP & Voice test suite (resampling ratios, WAV headers, ring buffers, barge-in, VAD pre-roll, normalizer) | Prevents return of alien TTS audio or deadlocked callbacks | Measurable DSP invariants in CI | P1 | M | IMP-016..031 | Dedicated tests in `pico-audio` and `pico-voice` pass headless on CI | Done (`4e79282`, `6873408`) |
| IMP-069 | Companion & IPC Resource Benchmark harness (`pico-desktop` RSS < 50 MB, idle CPU < 1%) | Quantitatively validates Goal 5 strict resource budgets in release builds | Measured truth for docs and CI | P1 | M | IMP-056 | Automated harness logs exact release RSS & idle CPU | Done (`99634e2`) |
| IMP-070 | Rewrite `README.md`, `PROJECT_STATUS.md`, `SECURITY.md`, `CONTRIBUTING.md`, `docs/*.md` from verified facts | Eliminates every fabricated claim (`2,112 tools`, `sub-15ms`, `60 FPS`, `neural cross-encoder`) | Trustworthy technical documentation | P1 | M | IMP-069 | Every metric in docs traced directly to measured outputs | Done (`ad71cbb`) |
| IMP-071 | Overhaul `site/index.html` & `docs/index.html` (antislop UI, contrast-verified, real architecture & features, mobile layout) | Fixes misleading marketing copy and matches redesigned Pico product & companion cleanly | Clean accessible site keeping `/api/submit` intact | P2 | L | IMP-070 | Contrast >= 4.5:1 verified; mobile 375px + desktop 1440px inspected; zero em dashes/slop | Done (`ad71cbb`) |
| IMP-072 | Update `.gitignore` for `models/*/` (`.onnx`, `.gguf`), `*.exe`, `__pycache__/` + fix all `fmt`/`clippy` warnings | Keeps 750 MB model binaries out of Git history and turns CI green | Clean git status & green multi-OS CI | P1 | S | - | `cargo fmt --check`, `cargo clippy -- -D warnings`, and GitHub Actions pass on Win + Linux | Done (`6873408`) |

---

## H. Additional Tracked Refinements (IMP-073 to IMP-100)

- **IMP-073** (`P3`, `S`) [`Done` (`b7f1595`)]: Machine-readable `PicoError::code()` enum values for clean IPC error display (`pico-core/src/error.rs`).
- **IMP-074** (`P3`, `S`) [`Done` (`b7f1595`)]: Truncate oversized `ToolResult` JSON before inserting into `pico_memory.sqlite` (`session.rs`).
- **IMP-075** (`P3`, `S`) [`Done` (`b7f1595`)]: Support UTF-8 aware token estimation & relevance decay weighting in `ContextBudgeter` / `MemoryManager` (`budget.rs`, `sqlite.rs`).
- **IMP-076** (`P3`, `S`) [`Done` (`b7f1595`)]: Bound `calculator_parser` lexer token length to guard against extreme multi-megabyte arithmetic strings (`calculator_parser.rs`).
- **IMP-077** (`P3`, `S`) [`Done` (`4e79282`)]: Normalize spoken arithmetic phrases (`"twenty five times four"`, `"10 percent of 250"`) before `calculator` execution (`argument.rs`).
- **IMP-078** (`P3`, `S`) [`Done` (`b7f1595`)]: Emit `session_id` on session-bound `PicoEvent` variants (`events.rs`).
- **IMP-079** (`P3`, `S`) [`Done` (`b7f1595`)]: Enforce JSON Schema self-consistency check (`type == "object"`, `properties` map, valid `required`) in `ToolRegistry::register` (`registry.rs`).
- **IMP-080** (`P3`, `S`) [`Done` (`b7f1595`)]: Add `--json` output flag and exit-code diagnostics to `pico-cli doctor` (`apps/pico-cli/src/main.rs`).
- **IMP-081** (`P3`, `S`) [`Done` (`b7f1595`)]: Add multi-file batch/search depth and entry count configurability cleanly in `SearchFilesTool` (`filesystem_tools.rs`).
- **IMP-082** (`P3`, `S`) [`Done` (`6637207`)]: Add explicit HTTP `User-Agent`, safe TLS defaults, and configurable request timeout on `FetchWebContentTool` (`utility_tools.rs`).
- **IMP-083** (`P3`, `S`) [`Done` (`4e79282`)]: Guard against acoustic echo/self-transcription in `VoicePipeline` while TTS playback window closes (`pipeline.rs`).
- **IMP-084** (`P3`, `S`) [`Done`]: Add `PicoRuntime` clean graceful shutdown method cancelling active tasks & closing voice/IPC handles (`coordinator.rs`).
- **IMP-085** (`P3`, `S`) [`Done` (`4e79282`)]: Detect Windows `App Paths` registry and Start Menu `.lnk` shortcuts cleanly in native Rust (`pico-platform/src/apps.rs`).
- **IMP-086** (`P3`, `S`) [`Done` (`4e79282`)]: Add Linux XDG `.desktop` entry parser (`Name=` / `Exec=` field stripping `%u`/`%f`) for reliable Linux app launching (`pico-platform/src/apps.rs`).
- **IMP-087** (`P3`, `S`) [`Done` (`6637207`)]: Inspect `PingHostTool` structured metrics (`transmitted`, `received`, `avg_ms`) and fail properly on 100% loss (`network_tools.rs`).
- **IMP-088** (`P3`, `S`) [`Done` (`99634e2`)]: Support `BAT*` / `power_supply` wildcard discovery on Linux instead of hardcoded `BAT0` (`sysinfo.rs:167`).
- **IMP-089** (`P3`, `S`) [`Done` (`ad71cbb`)]: Add `Run-Pico.bat` + `test.bat` + `test.sh` updates pointing directly to Cargo release/target binaries rather than stale root `pico-desktop.exe`.
- **IMP-090** (`P3`, `S`) [`Done` (`ad71cbb`)]: Update `.github/workflows/ci.yml` with Linux packages (`libasound2-dev`, `pkg-config`, `libdbus-1-dev`, `libx11-dev`, `libgtk-3-dev` or tray deps as required) + site sync test.
- **IMP-091** (`P4`, `M`) [`Done` (`4e79282`)]: Add Piper TTS auto-model discovery in `%LOCALAPPDATA%\Pico\voices` & `~/.local/share/pico/voices` (`tts.rs`).
- **IMP-092** (`P4`, `M`) [`Done` (`4e79282`)]: Add Windows WinRT `Windows.Media.SpeechSynthesis.SpeechSynthesizer` (OneCore natural neural/modern voices like David/Zira/Mark/Heera) as primary Windows TTS before Win32 SAPI (`tts.rs`).
- **IMP-093** (`P4`, `M`) [`Done` (`4e79282`)]: Add optional Speech Dispatcher (`spd-say` / socket) discovery alongside `espeak-ng` & `piper` on Linux (`tts.rs`).
- **IMP-094** (`P4`, `M`) [`Done` (`b7f1595`)]: Add multi-word stutter deduplication (`"set the set the"`) carefully bounded in `SpeechNormalizer` (`stt.rs`).
- **IMP-095** (`P4`, `S`) [`Done` (`ad71cbb`)]: Deduplicate synced `site/index.html` and `docs/index.html` via check script so GitHub Pages never drifts from `site/`.
- **IMP-096** (`P4`, `M`) [`Deferred`]: Add IPC push event streaming for live voice status (`Listening`, `Thinking`, `Clarifying`, `ConfirmationNeeded`) to the desktop companion HUD (`pico-desktop`).
- **IMP-097** (`P4`, `S`) [`Done` (`99634e2`)]: Add companion keyboard navigation map (`Tab`, `Ctrl+1..5` tabs, `Space` media play/pause, `Esc` hide to tray).
- **IMP-098** (`P4`, `S`) [`Done` (`99634e2`)]: Add companion atomic JSON save (write `.tmp` + `rename`) preventing corrupted notes/timers/stash on crash.
- **IMP-099** (`P4`, `S`) [`Done` (`99634e2`)]: Add configurable companion anchor/position persistence (top-right / bottom-right / dragged coordinate clamping inside active monitor work area).
- **IMP-100** (`P4`, `S`) [`Done` (`6873408`)]: Verify CodeQL Rust workflow compatibility (`codeql.yml`) with all updated Windows/Linux conditional compilation gates.
