# Tools & Verification Guide

Pico exposes a host-filtered tool ecosystem where every registered action maps to a concrete implementation, JSON parameter schema, permission tier, and verification test.

## Core Built-In Tools (`pico-tools`)

| Tool Name | Permission Tier | Implementation |
|---|---|---|
| `read_file` | `ReadOnly` | Streaming `BufReader` (`256 KB` / `2000` lines max), symlink check, `PathSandbox` enforcement |
| `write_file` | `ConfirmationRequired` | `2 MB` size cap, parent creation + post-create `PathSandbox` re-validation, symlink protection |
| `search_files` | `ReadOnly` | `DirEntry::file_type().is_symlink()` skip guard, depth `<= 5`, `5000` entries cap |
| `calculator` | `ReadOnly` | Recursive-descent AST math evaluator (arithmetic, powers, `sqrt`, `sin/cos/tan`, constants) |
| `clipboard` | `LowRisk` | Native Win32 `OpenClipboard` / Linux Wayland+X11 clipboard (`wl-copy`/`xclip`/`xsel`) |
| `fetch_web_content` | `ReadOnly` | SSRF validator + DNS `SocketAddr` pinning (`resolve_to_addrs`) + `256 KB` bounded body |
| `execute_shell_command` | `ConfirmationRequired` | Direct process spawn without shell interpreter wrappers, arg sanitizer, 30s tree kill, `256 KB` cap |
| `get_system_info` | `ReadOnly` | OS version (`RtlGetVersion` / `/etc/os-release`), memory, CPU cores, host name |
| `list_running_processes` | `ReadOnly` | Native `CreateToolhelp32Snapshot` on Windows / `/proc` scan on Linux |
| `kill_process` | `Restricted` | Self/critical-PID guard + `TerminateProcess` / `kill -TERM` |
| `manage_timer` | `LowRisk` | Persistent wall-clock UTC timers (`start`, `pause`, `resume`, `list`, `cancel`) |
| `open_application` | `LowRisk` | Direct `App Paths` / `PATH` resolution (`Stdio::null()`); `ShellExecuteW` strictly for `http/https` URIs |
| `close_application` | `ConfirmationRequired` | Process matching & safe termination excluding Pico and critical system PIDs |
| `set_master_volume` / `get_master_volume` / `mute_audio` | `LowRisk` | Native WASAPI COM (`IAudioEndpointVolume`) on Windows; `wpctl`/`pactl`/`amixer` on Linux |
| `set_brightness` / `get_brightness` | `LowRisk` | Native DXVA2 (`GetMonitorBrightness`/`SetMonitorBrightness`) + WMI fallback on Windows; `/sys/class/backlight` or `brightnessctl` on Linux |
| `network.ping` | `ReadOnly` | Input-validated `ping` invocation reporting latency and packet loss |
| `lock_screen` / `sleep_system` | `ConfirmationRequired` / `Restricted` | `LockWorkStation` / `SetSuspendState` on Windows; `loginctl` / `systemctl` on Linux |

## Curated Platform Catalog (`pico-tools/src/catalog/`)

Previously duplicated synthetic templates were removed. Instead, Pico registers verified, uniquely keyed platform catalog tools (`windows.*` on Windows, `linux.*` on Linux) filtered at runtime to the active host OS (`crates/pico-tools/src/registry.rs`):

- Every Windows catalog PowerShell command passes user arguments through `$env:PICO_ARG_*` environment variables (never interpolated into `-Command` strings).
- Every Linux catalog action validates numeric PIDs, service names, sysfs interface tokens, and sandboxed file paths cleanly before execution.
- `crates/pico-tools/tests/catalog_verification_test.rs` validates schema uniqueness, parameter adapter contracts, sandbox/injection rejection across every Windows & Linux catalog entry, live host execution, and cross-OS fail-closed behavior.
