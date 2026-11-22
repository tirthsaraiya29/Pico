use pico_core::error::{PicoError, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub memory_mb: u64,
}

pub struct PlatformApps;

impl PlatformApps {
    /// Launches an application or document safely without shell interpreters.
    pub fn open_application(app_or_path: &str) -> Result<()> {
        let trimmed = app_or_path.trim();
        if trimmed.is_empty() {
            return Err(PicoError::Platform(
                "Target application or URL cannot be empty".into(),
            ));
        }

        // Strictly reject shell chaining metacharacters
        if trimmed.contains('&')
            || trimmed.contains('|')
            || trimmed.contains(';')
            || trimmed.contains('\n')
            || trimmed.contains('\r')
            || trimmed.contains('`')
            || trimmed.contains('$')
        {
            return Err(PicoError::Security(format!(
                "Target '{}' contains prohibited shell metacharacters",
                trimmed
            )));
        }

        #[cfg(target_os = "windows")]
        {
            // Launch directly via Windows shell handler (explorer) without invoking cmd.exe /c start
            let status = Command::new("explorer")
                .arg(trimmed)
                .status()
                .map_err(|e| {
                    PicoError::Platform(format!("Failed to start target '{}': {}", trimmed, e))
                })?;

            if !status.success() {
                return Err(PicoError::Platform(format!(
                    "Target launch for '{}' exited with status: {}",
                    trimmed, status
                )));
            }
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let status = Command::new("xdg-open")
                .arg(trimmed)
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to open '{}': {}", trimmed, e)))?;
            if !status.success() {
                return Err(PicoError::Platform(format!("Failed to open '{}'", trimmed)));
            }
            Ok(())
        }
    }

    /// Lists active system processes.
    pub fn list_processes(limit: usize) -> Result<Vec<ProcessInfo>> {
        #[cfg(target_os = "windows")]
        {
            let output = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!(
                        "Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First {} Id, ProcessName, @{{Name='MemMB';Expression={{[math]::Round($_.WorkingSet64 / 1MB)}}}} | ConvertTo-Json -Compress",
                        limit
                    ),
                ])
                .output()
                .map_err(|e| PicoError::Platform(format!("Failed to list processes: {}", e)))?;

            let mut processes = Vec::new();
            if output.status.success() {
                let out_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(out_str.trim()) {
                    if let Some(arr) = val.as_array() {
                        for item in arr {
                            if let (Some(pid), Some(name)) = (
                                item.get("Id").and_then(|v| v.as_u64()),
                                item.get("ProcessName").and_then(|v| v.as_str()),
                            ) {
                                let mem = item.get("MemMB").and_then(|v| v.as_u64()).unwrap_or(0);
                                processes.push(ProcessInfo {
                                    pid: pid as u32,
                                    name: name.to_string(),
                                    memory_mb: mem,
                                });
                            }
                        }
                    } else if let (Some(pid), Some(name)) = (
                        val.get("Id").and_then(|v| v.as_u64()),
                        val.get("ProcessName").and_then(|v| v.as_str()),
                    ) {
                        let mem = val.get("MemMB").and_then(|v| v.as_u64()).unwrap_or(0);
                        processes.push(ProcessInfo {
                            pid: pid as u32,
                            name: name.to_string(),
                            memory_mb: mem,
                        });
                    }
                }
            }
            Ok(processes)
        }
        #[cfg(target_os = "linux")]
        {
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
                processes.sort_by(|a, b| b.memory_mb.cmp(&a.memory_mb));
                processes.truncate(limit);
                return Ok(processes);
            }

            // Fallback to ps command if /proc is not directly accessible
            if let Ok(output) = Command::new("ps")
                .args(["-eo", "pid,rss,comm", "--sort=-rss"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    for line in text.lines().skip(1).take(limit) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 {
                            if let (Ok(pid), Ok(rss_kb)) =
                                (parts[0].parse::<u32>(), parts[1].parse::<u64>())
                            {
                                let name = parts[2..].join(" ");
                                processes.push(ProcessInfo {
                                    pid,
                                    name,
                                    memory_mb: rss_kb / 1024,
                                });
                            }
                        }
                    }
                }
            }

            Ok(processes)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = limit;
            Ok(vec![])
        }
    }

    /// Terminates a process by its PID.
    pub fn kill_process(pid: u32) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let status = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .status()
                .map_err(|e| {
                    PicoError::Platform(format!("Failed to terminate PID {}: {}", pid, e))
                })?;

            if !status.success() {
                return Err(PicoError::Platform(format!(
                    "Failed to terminate process PID {}: taskkill returned {}",
                    pid, status
                )));
            }
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let status = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to kill PID {}", pid)))?;
            if !status.success() {
                return Err(PicoError::Platform(format!("Failed to kill PID {}", pid)));
            }
            Ok(())
        }
    }
}
