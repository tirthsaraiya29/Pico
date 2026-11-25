use pico_core::error::{PicoError, Result};
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "windows"))]
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub memory_mb: u64,
}

pub struct PlatformApps;

impl PlatformApps {
    /// Checks whether a candidate application name/path contains dangerous shell characters.
    pub fn validate_app_target(app_or_path: &str) -> Result<&str> {
        let trimmed = app_or_path.trim();
        if trimmed.is_empty() {
            return Err(PicoError::Platform(
                "Target application or URL cannot be empty".into(),
            ));
        }

        if trimmed.chars().any(|c| c.is_control())
            || trimmed.contains('&')
            || trimmed.contains('|')
            || trimmed.contains(';')
            || trimmed.contains('`')
            || trimmed.contains('$')
            || trimmed.contains('>')
            || trimmed.contains('<')
            || trimmed.contains('\'')
            || trimmed.contains('"')
        {
            return Err(PicoError::Security(format!(
                "Target '{}' contains prohibited shell metacharacters",
                trimmed
            )));
        }

        Ok(trimmed)
    }

    /// Launches a verified application, executable, or safe URI without cmd/PowerShell interpreters.
    pub fn open_application(app_or_path: &str) -> Result<()> {
        let trimmed = Self::validate_app_target(app_or_path)?;

        #[cfg(target_os = "windows")]
        {
            win_apps::open_application(trimmed)
        }
        #[cfg(target_os = "linux")]
        {
            linux_apps::open_application(trimmed)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(PicoError::Platform(format!(
                "Application launching unsupported on this OS for '{}'",
                trimmed
            )))
        }
    }

    /// Closes running user processes matching an application name safely (blocks system & self PIDs).
    pub fn close_application(app_name: &str) -> Result<usize> {
        let trimmed = Self::validate_app_target(app_name)?;
        let normalized = trimmed.trim_end_matches(".exe").to_lowercase();

        let protected = [
            "system", "idle", "csrss", "lsass", "wininit", "services", "smss", "explorer",
            "svchost", "systemd", "init", "pico",
        ];
        if protected
            .iter()
            .any(|p| *p == normalized || normalized.contains("pico"))
        {
            return Err(PicoError::Security(format!(
                "Refusing to terminate protected system/assistant process '{}'",
                trimmed
            )));
        }

        let procs = Self::list_processes(300)?;
        let self_pid = std::process::id();
        let matching_pids: Vec<u32> = procs
            .into_iter()
            .filter(|p| p.pid > 4 && p.pid != self_pid)
            .filter(|p| {
                let pname = p.name.trim_end_matches(".exe").to_lowercase();
                pname == normalized || pname.contains(&normalized)
            })
            .map(|p| p.pid)
            .collect();

        if matching_pids.is_empty() {
            return Err(PicoError::NotFound(format!(
                "No running process found matching '{}'",
                trimmed
            )));
        }

        let mut terminated = 0usize;
        for pid in matching_pids {
            if Self::kill_process(pid).is_ok() {
                terminated += 1;
            }
        }

        if terminated == 0 {
            return Err(PicoError::Platform(format!(
                "Found '{}' but could not terminate it (access denied or already exited)",
                trimmed
            )));
        }

        Ok(terminated)
    }

    /// Lists active system processes sorted by resident working memory descending.
    pub fn list_processes(limit: usize) -> Result<Vec<ProcessInfo>> {
        let clamped_limit = limit.clamp(1, 500);
        #[cfg(target_os = "windows")]
        {
            win_apps::list_processes(clamped_limit)
        }
        #[cfg(target_os = "linux")]
        {
            linux_apps::list_processes(clamped_limit)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = clamped_limit;
            Ok(vec![])
        }
    }

    /// Terminates a non-critical user process by PID cleanly.
    pub fn kill_process(pid: u32) -> Result<()> {
        if pid <= 4 || pid == std::process::id() {
            return Err(PicoError::Security(format!(
                "Refusing to terminate protected PID {}",
                pid
            )));
        }

        #[cfg(target_os = "windows")]
        {
            win_apps::kill_process(pid)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let status = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to kill PID {}: {}", pid, e)))?;
            if !status.success() {
                return Err(PicoError::Platform(format!("Failed to kill PID {}", pid)));
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "windows")]
mod win_apps {
    use super::ProcessInfo;
    use pico_core::error::{PicoError, Result};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn find_in_path(exe_name: &str) -> Option<PathBuf> {
        let candidate_path = Path::new(exe_name);
        if candidate_path.is_file() {
            return Some(candidate_path.to_path_buf());
        }
        let with_exe = if exe_name.to_lowercase().ends_with(".exe") {
            exe_name.to_string()
        } else {
            format!("{}.exe", exe_name)
        };
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let p = dir.join(&with_exe);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    fn lookup_app_paths_registry(exe_name: &str) -> Option<PathBuf> {
        let with_exe = if exe_name.to_lowercase().ends_with(".exe") {
            exe_name.to_string()
        } else {
            format!("{}.exe", exe_name)
        };
        let subkey = format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
            with_exe
        );
        let wide_subkey = to_wide(&subkey);

        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            unsafe {
                let mut hkey = HKEY::default();
                if RegOpenKeyExW(root, PCWSTR(wide_subkey.as_ptr()), 0, KEY_READ, &mut hkey).is_ok()
                {
                    let mut buf = [0u16; MAX_PATH as usize * 2];
                    let mut byte_len = (buf.len() * 2) as u32;
                    let ok = RegQueryValueExW(
                        hkey,
                        PCWSTR::null(),
                        None,
                        None,
                        Some(buf.as_mut_ptr() as *mut u8),
                        Some(&mut byte_len),
                    )
                    .is_ok();
                    let _ = RegCloseKey(hkey);
                    if ok && byte_len >= 2 {
                        let chars = (byte_len as usize / 2).saturating_sub(1);
                        let raw = String::from_utf16_lossy(&buf[..chars]);
                        let clean = raw.trim_matches('"').trim();
                        let pb = PathBuf::from(clean);
                        if pb.is_file() {
                            return Some(pb);
                        }
                    }
                }
            }
        }
        None
    }

    fn resolve_known_alias(trimmed: &str) -> (&'static str, Vec<PathBuf>) {
        let lower = trimmed.to_lowercase();
        let mut extras = Vec::new();
        let target: &str = match lower.as_str() {
            "calculator" | "calc" | "calc.exe" => "calc.exe",
            "notepad" | "notepad.exe" => "notepad.exe",
            "edge" | "microsoft edge" | "msedge" => {
                extras.push(PathBuf::from(
                    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                ));
                extras.push(PathBuf::from(
                    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                ));
                "msedge.exe"
            }
            "brave" | "brave browser" | "brave-browser" => {
                extras.push(PathBuf::from(
                    r"C:\Program Files\BraveSoftware\Brave-Browser-Nightly\Application\brave.exe",
                ));
                extras.push(PathBuf::from(
                    r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                ));
                extras.push(PathBuf::from(
                    r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
                ));
                "brave.exe"
            }
            "chrome" | "google chrome" | "browser" => {
                extras.push(PathBuf::from(
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                ));
                extras.push(PathBuf::from(
                    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                ));
                extras.push(PathBuf::from(
                    r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                ));
                "chrome.exe"
            }
            "firefox" => {
                extras.push(PathBuf::from(
                    r"C:\Program Files\Mozilla Firefox\firefox.exe",
                ));
                "firefox.exe"
            }
            "editor" | "code" | "vscode" | "visual studio code" => "code.exe",
            "files" | "folder" | "explorer" | "file explorer" => "explorer.exe",
            "terminal" | "console" | "wt" => "wt.exe",
            "task manager" | "taskmgr" => "taskmgr.exe",
            "paint" => "mspaint.exe",
            "settings" | "ms-settings:" => "ms-settings:",
            _ => return ("", extras),
        };
        (target, extras)
    }

    pub fn open_application(trimmed: &str) -> Result<()> {
        let lower = trimmed.to_lowercase();

        // Allow strictly validated web URLs or Windows Settings URI via ShellExecuteW (never cmd/powershell)
        if lower == "settings"
            || lower == "ms-settings:"
            || lower.starts_with("http://")
            || lower.starts_with("https://")
        {
            let uri = if lower == "settings" {
                "ms-settings:"
            } else {
                trimmed
            };
            let verb = to_wide("open");
            let file = to_wide(uri);
            let hinst = unsafe {
                ShellExecuteW(
                    None,
                    PCWSTR(verb.as_ptr()),
                    PCWSTR(file.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                )
            };
            if (hinst.0 as isize) <= 32 {
                return Err(PicoError::Platform(format!(
                    "Failed to open URI target '{}' (code {})",
                    uri, hinst.0 as isize
                )));
            }
            return Ok(());
        }

        let (alias_target, extra_paths) = resolve_known_alias(trimmed);
        let lookup_name = if !alias_target.is_empty() {
            alias_target
        } else {
            trimmed
        };

        let resolved = extra_paths
            .into_iter()
            .find(|p| p.is_file())
            .or_else(|| find_in_path(lookup_name))
            .or_else(|| lookup_app_paths_registry(lookup_name));

        let Some(exe_path) = resolved else {
            return Err(PicoError::NotFound(format!(
                "Application '{}' could not be located on this system",
                trimmed
            )));
        };

        Command::new(&exe_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| {
                PicoError::Platform(format!(
                    "Failed to launch '{}' ({}): {}",
                    trimmed,
                    exe_path.display(),
                    e
                ))
            })?;

        Ok(())
    }

    pub fn list_processes(limit: usize) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| {
                PicoError::Platform(format!("CreateToolhelp32Snapshot failed: {}", e))
            })?;
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };

            if Process32FirstW(snap, &mut entry).is_ok() {
                loop {
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
                    let pid = entry.th32ProcessID;
                    let mut mem_mb = 0u64;

                    if pid > 0 {
                        if let Ok(proc_handle) =
                            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                        {
                            let mut pmc = PROCESS_MEMORY_COUNTERS {
                                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                                ..Default::default()
                            };
                            if K32GetProcessMemoryInfo(
                                proc_handle,
                                &mut pmc,
                                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                            )
                            .as_bool()
                            {
                                mem_mb = (pmc.WorkingSetSize as u64) / (1024 * 1024);
                            }
                            let _ = CloseHandle(proc_handle);
                        }
                    }

                    processes.push(ProcessInfo {
                        pid,
                        name,
                        memory_mb: mem_mb,
                    });

                    if Process32NextW(snap, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }

        processes.sort_by_key(|a| std::cmp::Reverse(a.memory_mb));
        processes.truncate(limit);
        Ok(processes)
    }

    pub fn kill_process(pid: u32) -> Result<()> {
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|e| {
                PicoError::Platform(format!("Unable to open process PID {}: {}", pid, e))
            })?;
            let term_res = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
            term_res
                .map_err(|e| PicoError::Platform(format!("Failed to terminate PID {}: {}", pid, e)))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_apps {
    use super::ProcessInfo;
    use pico_core::error::{PicoError, Result};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    fn find_binary_in_path(name: &str) -> Option<PathBuf> {
        let p = Path::new(name);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn map_alias(trimmed: &str) -> &str {
        match trimmed.to_lowercase().as_str() {
            "browser" | "chrome" | "google chrome" => "google-chrome",
            "brave" | "brave browser" => "brave-browser",
            "firefox" => "firefox",
            "editor" | "vscode" | "visual studio code" => "code",
            "terminal" => "x-terminal-emulator",
            "files" => "xdg-open",
            _ => trimmed,
        }
    }

    pub fn open_application(trimmed: &str) -> Result<()> {
        let lower = trimmed.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let status = Command::new("xdg-open")
                .arg(trimmed)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| PicoError::Platform(format!("Failed to open '{}': {}", trimmed, e)))?;
            let _ = status;
            return Ok(());
        }

        let target = map_alias(trimmed);
        if let Some(bin) = find_binary_in_path(target).or_else(|| find_binary_in_path(trimmed)) {
            Command::new(bin)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| {
                    PicoError::Platform(format!("Failed to launch '{}': {}", trimmed, e))
                })?;
            return Ok(());
        }

        Err(PicoError::NotFound(format!(
            "Application '{}' could not be located in PATH",
            trimmed
        )))
    }

    pub fn list_processes(limit: usize) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if let Ok(pid) = name_str.parse::<u32>() {
                    let proc_path = entry.path();
                    let comm_path = proc_path.join("comm");
                    let proc_name = std::fs::read_to_string(&comm_path)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|_| name_str.to_string());

                    let statm_path = proc_path.join("statm");
                    let memory_mb = if let Ok(statm) = std::fs::read_to_string(&statm_path) {
                        let parts: Vec<&str> = statm.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let pages = parts[1].parse::<u64>().unwrap_or(0);
                            (pages * 4096) / (1024 * 1024)
                        } else {
                            0
                        }
                    } else {
                        0
                    };

                    processes.push(ProcessInfo {
                        pid,
                        name: proc_name,
                        memory_mb,
                    });
                }
            }
            processes.sort_by_key(|a| std::cmp::Reverse(a.memory_mb));
            processes.truncate(limit);
            return Ok(processes);
        }

        Ok(processes)
    }
}
