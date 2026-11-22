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
        #[cfg(not(target_os = "windows"))]
        {
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
