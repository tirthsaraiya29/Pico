use pico_core::error::{PicoError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub os: String,
    pub hostname: String,
    pub cpu_cores: usize,
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub battery_percent: Option<u8>,
    pub is_charging: Option<bool>,
}

pub struct PlatformSysInfo;

impl PlatformSysInfo {
    /// Retrieves current system hardware and resource usage statistics.
    pub fn get_system_stats() -> Result<SystemStats> {
        #[cfg(target_os = "windows")]
        {
            Self::get_windows_stats()
        }
        #[cfg(target_os = "linux")]
        {
            Self::get_linux_stats()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            Ok(SystemStats {
                os: std::env::consts::OS.to_string(),
                hostname: "localhost".into(),
                cpu_cores: cores,
                total_memory_mb: 8192,
                available_memory_mb: 4096,
                battery_percent: None,
                is_charging: None,
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn get_windows_stats() -> Result<SystemStats> {
        let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows-PC".into());
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        // Query memory status and battery state via CIM safely
        let ps_cmd = r#"$os = Get-CimInstance Win32_OperatingSystem; $memTotal = [math]::Round($os.TotalVisibleMemorySize / 1024); $memFree = [math]::Round($os.FreePhysicalMemory / 1024); $bat = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue; $batPct = if ($bat) { $bat.EstimatedChargeRemaining } else { $null }; $charging = if ($bat) { ($bat.BatteryStatus -eq 2) } else { $null }; [PSCustomObject]@{ TotalMem = $memTotal; FreeMem = $memFree; BatteryPct = $batPct; Charging = $charging } | ConvertTo-Json -Compress"#;

        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
            .output()
            .map_err(|e| {
                PicoError::Platform(format!("Failed to execute sysinfo command: {}", e))
            })?;

        let mut total_memory_mb = 32768;
        let mut available_memory_mb = 16384;
        let mut battery_percent = None;
        let mut is_charging = None;

        if output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(out_str.trim()) {
                if let Some(t) = val.get("TotalMem").and_then(|v| v.as_u64()) {
                    total_memory_mb = t;
                }
                if let Some(f) = val.get("FreeMem").and_then(|v| v.as_u64()) {
                    available_memory_mb = f;
                }
                if let Some(b) = val.get("BatteryPct").and_then(|v| v.as_u64()) {
                    battery_percent = Some(b as u8);
                }
                if let Some(c) = val.get("Charging").and_then(|v| v.as_bool()) {
                    is_charging = Some(c);
                }
            }
        }

        Ok(SystemStats {
            os: "Windows 11 x64".to_string(),
            hostname,
            cpu_cores: cores,
            total_memory_mb,
            available_memory_mb,
            battery_percent,
            is_charging,
        })
    }

    #[cfg(target_os = "linux")]
    fn get_linux_stats() -> Result<SystemStats> {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        let hostname = std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| {
                std::env::var("HOSTNAME").unwrap_or_else(|_| "linux-host".to_string())
            });

        // Determine OS name from /etc/os-release
        let mut os_name = "Linux".to_string();
        if let Ok(release) = std::fs::read_to_string("/etc/os-release") {
            for line in release.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    os_name = line
                        .trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string();
                    break;
                }
            }
        }

        // Parse memory from /proc/meminfo
        let mut total_memory_mb = 0u64;
        let mut available_memory_mb = 0u64;

        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        total_memory_mb = kb_str.parse::<u64>().unwrap_or(0) / 1024;
                    }
                } else if line.starts_with("MemAvailable:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        available_memory_mb = kb_str.parse::<u64>().unwrap_or(0) / 1024;
                    }
                }
            }
        }

        // Battery from /sys/class/power_supply
        let mut battery_percent = None;
        let mut is_charging = None;

        for bat in &["BAT0", "BAT1"] {
            let cap_path = format!("/sys/class/power_supply/{}/capacity", bat);
            if let Ok(cap_str) = std::fs::read_to_string(&cap_path) {
                if let Ok(pct) = cap_str.trim().parse::<u8>() {
                    battery_percent = Some(pct);
                }
            }

            let status_path = format!("/sys/class/power_supply/{}/status", bat);
            if let Ok(status_str) = std::fs::read_to_string(&status_path) {
                is_charging = Some(status_str.trim() == "Charging");
            }

            if battery_percent.is_some() {
                break;
            }
        }

        Ok(SystemStats {
            os: os_name,
            hostname,
            cpu_cores: cores,
            total_memory_mb,
            available_memory_mb,
            battery_percent,
            is_charging,
        })
    }
}
