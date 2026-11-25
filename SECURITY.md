# Security Policy

## Security Architecture

Pico executes local operating-system actions without cloud delegation, enforcing zero-trust defense in depth across every boundary:

1. **Filesystem Isolation (`PathSandbox`)**:
   - Fails closed when `allowed_directories` is empty.
   - Blocks UNC shares (`\\server\share`), raw device namespaces (`\\?\`), drive-relative Windows paths (`C:file`), NTFS Alternate Data Streams (`file:stream`), and DOS reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`..`COM9`, `LPT1`..`LPT9`).
   - Rejects parent traversal components (`..`, trailing `/..`, `\..`) before path resolution.
   - Walks existing prefixes and leaves via `symlink_metadata` to block symlink/junction and dangling-symlink escapes.
   - Denies access to credential and secret paths (`.ssh`, `.gnupg`, `.aws`, `.kube`, `.docker`, `.netrc`, `.npmrc`, `.git-credentials`, `.env`, `Login Data`, `key4.db`, `ntds.dit`, `SAM`, `/etc/shadow`, `/etc/sudoers`).
   - File tools stream reads with bounded byte/line limits (`256 KB` / `2000` lines), enforce `2 MB` write caps, re-validate parent directories after creation, and skip symlinks during directory search.

2. **SSRF & Network Hardening (`FetchWebContentTool`)**:
   - Permits only `http` and `https` without embedded URL credentials (`user:pass@`).
   - Blocks `.local`, `.internal`, `.localhost`, `localhost`, RFC 1918 private ranges, CGNAT (`100.64.0.0/10`), cloud metadata (`169.254.169.254`), loopback, multicast, IPv4-mapped/IPv4-compatible IPv6, NAT64 (`64:ff9b::/96`), 6to4 (`2002::/16`), Teredo (`2001::/32`), documentation (`2001:db8::/32`), unique local (`fc00::/7`), and link-local (`fe80::/10`).
   - Defeats DNS-rebinding TOCTOU by pinning resolved & validated `SocketAddr`s directly into per-request HTTP clients via `.resolve_to_addrs(...)`, manually re-validating every redirect hop (max 5), and bounding body downloads to `256 KB`.

3. **Execution Authorization (`SecurityPolicy` & `ChannelConfirmationManager`)**:
   - `PermissionTier::Restricted` can never bypass interactive confirmation even in permissive mode.
   - Confirmation tokens bind strictly to `(request_id, session_id, tool_name, args_sha256, expires_at)` and are atomically consumed on first use.
   - `VoiceAuthorizer` gives strict priority to negation terms over affirmatives using word-boundary regexes so traps like `"don't confirm"` or `"no wait yes confirm"` fail closed.
   - `ExecuteShellCommandTool` uses `tokio::process::Command` directly without spawning shells, rejects shell interpreter wrappers (`powershell`, `cmd`, `pwsh`, `bash`, `sh`, `zsh`), checks metacharacters/NUL bytes via `InputSanitizer`, enforces 30s timeouts with process-tree cleanup, and caps stdout/stderr at `256 KB`.

4. **Local IPC Authentication (`pico-core::ipc` & `pico-orchestrator::ipc_server`)**:
   - **Windows Named Pipes**: Owner+SYSTEM SDDL DACL (`D:(A;;GA;;;OW)(A;;GA;;;SY)`), `first_pipe_instance(true)` on startup, and `PIPE_REJECT_REMOTE_CLIENTS`.
   - **Linux Unix Domain Sockets**: Mode `0o600` under `$XDG_RUNTIME_DIR` plus kernel `SO_PEERCRED` UID matching.
   - Both platforms enforce `protocol = 1`, `64 KB` max frame size, and constant-time comparison against an ephemeral per-daemon random token written with user-private permissions.

5. **Audit Logging & Storage (`AuditLogger` & `SqliteStore`)**:
   - Automatically masks sensitive keys (`key`, `secret`, `password`, `token`, `auth`, `credential`, `cookie`, `bearer`) AND redacts credential patterns inside arbitrary string values (`Bearer ...`, `ghp_`, `github_pat_`, `sk-`, `AIza`, PEM private keys, `https://user:pass@`).
   - SQLite `search_facts` escapes `%`, `_`, and `\` wildcards with `ESCAPE '\'`.

6. **Web Submission Endpoint (`api/submit.js`)**:
   - Uses explicit origin allowlists instead of wildcard `*` CORS, enforces `application/json`, validates proxy IP headers + rate limits, and routes security reports privately rather than creating public issues.

## Reporting a Vulnerability

Do **not** open a public GitHub issue for security vulnerabilities.
Use the confidential security form on the project website or contact the maintainer privately with reproduction steps, affected files, and impact assessment.
