# Pico Forensic Security Audit

**Audit Date**: 2026-11-25
**Target**: `https://github.com/tirthsaraiya29/Pico.git` (`main` @ `faf1a5a8da856a2b88eceb188e92aa1d32e401d9`)
**Scope**: Command execution boundaries, filesystem sandboxing, SSRF & network egress, confirmation & authorization integrity, IPC design, local SQLite & log privacy, `unsafe` Rust blocks, supply-chain/CI & serverless API surface.

---

## 1. Threat Model Summary

Pico parses natural-language text and microphone transcripts locally and translates them into OS actions (launching programs, reading/writing files, managing audio/power/processes, fetching HTTP URLs, executing shell commands). Because STT output, web-fetched HTML/text (`FetchWebContentTool`), clipboard contents, filenames, and user-pasted text are **untrusted inputs**, every security boundary must enforce defense-in-depth independent of intent/ranking confidence.

---

## 2. Confirmed Security Vulnerabilities & Gaps

### SEC-001 | P0 Critical | Command Injection in Application Launcher
- **Location**: `crates/pico-platform/src/apps.rs:124-144`
- **Mechanism**: Unrecognized app strings pass directly into `cmd /C start "" <target>` (`line 124`) and fallback PowerShell `format!("Start-Process '{}'", target)` (`line 141`) without shell escaping or metacharacter filtering.
- **Exploitability**: Any request whose extracted `target` contains `' ; Start-Process calc.exe ; '` or `& whoami` triggers arbitrary shell/PowerShell command execution, bypassing `ExecuteShellCommandTool`'s `ConfirmationRequired` gate because `OpenApplicationTool` is marked `PermissionTier::LowRisk` (`crates/pico-tools/src/system_tools.rs:153`)!
- **Remediation**:
  1. Remove `cmd /C` and raw PowerShell string interpolation entirely from `PlatformApps::open_application`.
  2. Validate app/target names against strict character/length policies and reject control characters and shell metacharacters (`& | ; $ > < \` ' " \n \r`).
  3. Resolve executables explicitly via Windows `App Paths` registry, Start Menu `.lnk` parser, validated `PATH`, or Win32 `ShellExecuteExW` (wide UTF-16 API with `lpVerb = "open"` and explicit file/URI check, never invoking cmd shell), and direct spawn / validated `.desktop` / `xdg-open` on Linux.
- **Regression Test**: Verify `"notepad'; calc; #"`, `"notepad & whoami"`, `"notepad\ncalc"`, and `"$(calc)"` fail safely with `PicoError::Security` or validation error.

### SEC-002 | P0 Critical | Parameterized Command Injection in Massive Catalog Adapter
- **Location**: `crates/pico-tools/src/catalog/adapter.rs:139-160`
- **Mechanism**: Windows adapter substitutes parameter values directly into PowerShell `-Command` strings using `.replace('\'', "''").replace('`', "``")`. Parameters inserted unquoted into `-Command` scripts (`-Path $path`, `-Name $name`) still allow PowerShell command separators (`;`, `|`, `&`), subexpressions (`$(...)`), and redirection operators (`>`, `<`).
- **Exploitability**: A crafted string argument such as `"foo; Start-Process calc.exe"` escapes parameter context in PowerShell because placeholders like `$name` in `adapter_args` have no surrounding quotes.
- **Remediation**: Never inject argument values directly into PowerShell source text. Pass arguments cleanly through process environment variables (`cmd.env("PICO_ARG_NAME", val)`) referenced safely in script templates as `$env:PICO_ARG_NAME`, combined with per-parameter allowlist validation (`InputSanitizer::is_safe_arg`) and path validation through `PathSandbox`.
- **Regression Test**: Supply `"; calc.exe"`, `"$(whoami)"`, `"a | Out-File x"` as argument values; confirm zero command injection occurs.

### SEC-003 | P0 Critical | Arbitrary `/proc` and `/sys` Path Traversal on Linux
- **Location**: `crates/pico-tools/src/catalog/adapter.rs:225-248`
- **Mechanism**: When `meta.adapter_cmd.starts_with("/proc") || meta.adapter_cmd.starts_with("/sys")`, parameter strings replace `$param` directly inside filesystem path templates before `tokio::fs::read_to_string(&path)` (`line 236`).
- **Exploitability**: Supplying `pid = "../../etc/shadow"` or `interface = "../../../../etc/passwd"` escapes `/proc` or `/sys` into arbitrary readable files on the system, all under `PermissionTier::ReadOnly`.
- **Remediation**: Enforce strict regex validation on token values interpolated into `/proc` or `/sys` (`^\d+$` for PIDs; `^[A-Za-z0-9_.-]+$` with no `..` and no `/` for sysfs device/interface names), AND canonicalize the resulting path confirming it starts cleanly with `/proc/` or `/sys/`.
- **Regression Test**: Pass `../../etc/passwd` as `$pid` / `$iface` and assert `PicoError::Security`.

### SEC-004 | P0 Critical | Symlink / Junction Sandbox Escape in `SearchFilesTool`
- **Location**: `crates/pico-tools/src/filesystem_tools.rs:233-252`
- **Mechanism**: Recursive directory walker uses `path.is_dir()` without inspecting symlink metadata or calling `sandbox.validate_path` on traversed subdirectories.
- **Exploitability**: Any symlink or NTFS junction inside the workspace pointing to `C:\Users\<user>\.ssh`, `%APPDATA%`, or `/etc` is traversed seamlessly by `search_files` (tier `ReadOnly`), disclosing private directory/file layouts outside allowed boundaries.
- **Remediation**: Skip entries where `entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(true)` (and re-check `validate_path` for any directory entered), plus inspect Windows reparse attribute (`FILE_ATTRIBUTE_REPARSE_POINT`).
- **Regression Test**: Create a directory symlink inside a test sandbox pointing to an external sibling secret folder; assert zero external filenames appear in `SearchFilesTool` results.

### SEC-005 | P0 Critical | `PathSandbox` Fail-Open Empty Roots & Trailing `..` & Incomplete Credential Filters
- **Location**: `crates/pico-security/src/sandbox.rs:95-189`
- **Mechanism**:
  1. `if self.allowed_roots.is_empty() { return Ok(normalized); }` (`lines 175-177`) turns an empty allowlist into **full filesystem access** (minus fragments).
  2. Traversal check (`line 96`) omits trailing `subdir/..` or `subdir\..` without trailing slash.
  3. `sensitive_fragments` (`lines 153-162`) omits `.kube`, `.docker`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `authorized_keys`, `known_hosts`, `ntds.dit`, `sam`, `shadow`, `gshadow`, `/etc/sudoers`, Chrome/Edge/Firefox profile SQLite/Login Data.
  4. Multi-step `validate_path` then `create_dir_all` + `write` creates a TOCTOU window if an attacker or local race replaces a parent directory with a junction.
- **Remediation**: Reject empty `allowed_roots` cleanly (`Err(PicoError::Security(...))`), reject any path containing `Component::ParentDir` or trailing `/..` `\..`, expand sensitive fragments and exact system paths, verify parent dirs lack reparse points/symlinks before `write_file` writes, and canonicalize post-creation parent before opening output file (`O_NOFOLLOW` where available).
- **Regression Test**: Unit test verifying `PathSandbox::new(vec![], vec![])` denies access, `dir/..` denies access, and `.kube/config`, `.netrc`, `.git-credentials`, `Login Data` are denied.

### SEC-006 | P0 Critical | DNS-Rebinding TOCTOU, Unbounded Memory DoS & Incomplete IPv6 Filtering in `FetchWebContentTool`
- **Location**: `crates/pico-tools/src/utility_tools.rs:278-455`
- **Mechanism**:
  1. `validate_url_ssrf` calls `tokio::net::lookup_host` (`line 330`), discards the resolved IPs after validating, and then passes the hostname URL to `reqwest::Client` which executes a second independent DNS resolution (`TOCTOU` DNS rebinding).
  2. `is_safe_public_ipv6` misses NAT64 (`64:ff9b::/96`, `64:ff9b:1::/48`), Teredo (`2001::/32`), 6to4 (`2002::/16`), Orchid (`2001:10::/28`), benchmarking (`2001:2::/48`), and IPv4-compatible `::/96` prefixes wrapping private IPv4 addresses.
  3. `resp.text().await` (`line 452`) allocates the entire response stream in memory without Content-Length or chunk byte caps.
- **Remediation**:
  1. Inject a custom `reqwest::dns::Resolve` implementation that caches and pins only verified public `SocketAddr` entries obtained per hop during `validate_url_ssrf`.
  2. Complete IPv6 embedded/transition address extraction and private range checks.
  3. Stream response body chunk-by-chunk (`resp.chunk().await`), aborting cleanly beyond `MAX_FETCH_BYTES` (e.g. `2 * 1024 * 1024` bytes).
- **Regression Test**: Test NAT64/6to4/IPv4-compat IPv6 blocking, DNS-pinning unit test, and mock HTTP server test returning >2 MB verifying bounded read truncation/error.

### SEC-007 | P0 Critical | Unbound Confirmation Tokens, Replay Risk & Session Cross-Contamination
- **Location**: `crates/pico-security/src/policy.rs:26-100,136-215`, `apps/pico-cli/src/main.rs:66-117`, `apps/pico-desktop/src/main.rs:66-100`
- **Mechanism**: `ChannelConfirmationManager` indexes confirmations solely by `request_id: String`. It does not bind confirmations to `session_id`, `tool_name`, `arguments_hash`, or `expires_at`. Additionally, CLI/Desktop UIs store only a single global `Option<PendingConfirmation>` mutex slot, so rapid/concurrent confirmation requests drop older ones and approve/deny the wrong action if user replies after timeout or interleaving. Furthermore, in `"Permissive"` policy mode (`strict_mode = false`), `SecurityPolicy::verify_permission` can silently approve destructive/shell actions without checking interactive confirmation unless explicitly forced.
- **Remediation**:
  1. Model confirmation challenges as `PendingConfirmationRecord { request_id, session_id, tool_name, args_sha256, tier, expires_at, sender }`.
  2. Enforce single-use removal (`map.remove`) + expiry check (`Utc::now() > expires_at`) + explicit match on session/tool/args before releasing approval.
  3. Never allow `"Permissive"` mode to bypass `PermissionTier::Restricted` (must remain denied unless explicitly disabled at compile/test scope or strictly gated), and require confirmation for `ConfirmationRequired` unless explicitly auto-approved in isolated test harnesses.
- **Regression Test**: Verify expired confirmation rejection, cross-session rejection, replay failure on second `respond`, and multi-request concurrency preservation.

### SEC-008 | P1 High | Arbitrary Process Execution & Unbounded Stdout OOM in `ExecuteShellCommandTool`
- **Location**: `crates/pico-tools/src/command_tools.rs:43-120`
- **Mechanism**: Tool takes arbitrary `command` and `args` with no stdout/stderr byte limit and invokes `kill_process` fallback on timeout using stringly-typed flags; also allows invoking shell interpreters (`powershell`, `cmd`, `bash`, `sh`) with `-EncodedCommand` or `-c` scripts if approved once, without separating shell wrapper vs direct binary.
- **Remediation**: Stream stdout/stderr with bounded caps (e.g., 256 KB), retain `ConfirmationRequired`, validate non-empty command, and classify shell interpreter invocations explicitly in confirmation prompt text so the user sees exact script arguments.

### SEC-009 | P1 High | `AuditLogger` Missing Directory Creation & Secret Leakage in Argument Strings/Messages
- **Location**: `crates/pico-security/src/audit.rs:27-85`
- **Mechanism**: Parent directory of `audit_log_path` is not created, causing silent write failure when `logs/` does not exist. `mask_sensitive_data` inspects JSON key names only; secrets passed inside `"arguments"` text fields (`--token ghp_abc...`, `https://user:pass@host`, `Authorization: Bearer ...`, `AKIA...`) or inside `"message"` strings are logged verbatim in plaintext JSONL.
- **Remediation**: Auto-create parent directory (`create_dir_all`), apply compiled regex redaction over all JSON `Value::String` leaves and `Record.message` fields (masking JWTs, Bearer headers, URL embedded credentials, AWS keys, GitHub/OpenAI-style tokens), and enforce `0o600` file permissions on Unix.

### SEC-010 | P1 High | Local Companion-to-Runtime IPC Attack Surface (Architectural Design Requirement)
- **Context**: Goal 5 splits `pico-desktop` companion UI from the headless voice/assistant runtime using local IPC. Unauthenticated localhost TCP ports (`127.0.0.1:3124`, formerly stubbed in `configs/pico.toml:57-61`) allow any local process or SSRF browser request (CSRF/DNS rebinding) on the machine to trigger assistant tools!
- **Remediation**:
  - Do NOT bind unauthenticated TCP HTTP servers for desktop control.
  - On Windows: use a **Named Pipe** (`\\.\pipe\pico-assistant-<sid_or_user>`) configured with a restrictive DACL allowing access **only to the current interactive user SID / Owner** and `PIPE_REJECT_REMOTE_CLIENTS` enabled.
  - On Linux: use a **Unix Domain Socket** in `$XDG_RUNTIME_DIR/pico-assistant.sock` (or `/tmp/pico-<uid>.sock`) with directory permissions `0o700` and socket permissions `0o600` + `SO_PEERCRED` UID verification on connect.
  - Enforce versioned JSON-framed messages with max message size cap (`64 KB`) and explicit per-session confirmation binding for privileged commands.

### SEC-011 | P1 High | `VoiceAuthorizer` Multi-Clause Negation Bypass & Unification Gap
- **Location**: `crates/pico-security/src/voice_auth.rs:33-85`
- **Mechanism**: Utterance evaluation checks negation patterns first, which is good, yet lacks normalization of leading fillers/ASR noise around multi-word commands or clear prompt-injection/replay isolation when TTS speaks confirmation prompts while open mic is active (acoustic self-trigger / loopback echo!).
- **Remediation**: Suppress mic/VAD acoustic acceptance during system TTS prompt articulation (echo gating) and preserve strict negation precedence (`"yes wait no"` -> `Declined`).

### SEC-012 | P2 Medium | Plaintext Local SQLite Stores & Unescaped LIKE Search
- **Location**: `crates/pico-memory/src/sqlite.rs:240-310`, `crates/pico-learning/src/store.rs`
- **Mechanism**: Conversation histories, facts, and observations are stored in plaintext local SQLite files with default umask and `LIKE` string interpolation without escaping `%`/`_`/`\`.
- **Remediation**: Restrict file permissions (`0o600` on Linux), escape LIKE special characters cleanly, and never record raw unmasked secrets in observation `request_text` / memory facts.

### SEC-013 | P2 Medium | `api/submit.js` Rate Limit Bypass & Wildcard CORS
- **Location**: `api/submit.js:25-56,136`
- **Mechanism**: `Access-Control-Allow-Origin: *` allows third-party origins to trigger submissions against the Vercel serverless function; in-memory `rateMap` evaporates on serverless cold start; client-forged `x-forwarded-for` headers are checked directly if not overwritten at edge.
- **Remediation**: Restrict CORS origin allowlist (`https://tirthsaraiya29.github.io`, configured domain), read rightmost/Vercel-verified IP (`x-real-ip` / `req.socket.remoteAddress`), tighten payload size & validation (keep repo public issue flow intact).

### SEC-014 | P2 Medium | `unsafe` Rust Code Inventory & Review
| File & Line | Construct | Risk & Finding |
|---|---|---|
| `crates/pico-audio/src/capture.rs:16-17` | `unsafe impl Send / Sync for AudioCapture` | Audio `cpal::Stream` lives cleanly on a dedicated thread and is NOT stored in `AudioCapture`; only `Arc<AtomicBool>` and `mpsc::Sender<()>` are fields! Thus `unsafe impl Send / Sync` is completely unnecessary (`dead unsafe`) and misleading. |
| `crates/pico-audio/src/playback.rs:53-54` | `unsafe impl Send / Sync for AudioPlayback` | Identical to `AudioCapture`: `cpal::Stream` never crosses threads, struct holds `PlaybackHandle` (`Arc`+`Mutex`/`Atomic`) + `Option<mpsc::Sender<()>>`. `unsafe impl` is redundant and should be removed. |
| `crates/pico-platform/src/sysinfo.rs:87-104` | `unsafe { GlobalMemoryStatusEx; GetSystemPowerStatus }` | Structs use `#[repr(C)]` matching Win32 layout with `dw_length = size_of()` initialized properly. Safe if replaced with official `windows-sys` / `windows` bindings to prevent manual ABI drift. |
| `crates/pico-orchestrator/tests/benchmark_test.rs:32-38` | `unsafe { K32GetProcessMemoryInfo }` | Test-only Win32 FFI; valid `dwSize` check present, safe to migrate to `windows` crate bindings cleanly. |

---

## 3. Verification Matrix for Security Fixes

Every security item (`SEC-001` through `SEC-014`) must be accompanied by an explicit adversarial unit/integration test asserting rejection of exploit payloads before marking `Implemented` during Goal 4.
