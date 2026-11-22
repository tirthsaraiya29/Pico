use pico_core::error::Result;
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

        #[repr(C)]
        #[allow(clippy::upper_case_acronyms)]
        struct MEMORYSTATUSEX {
            dw_length: u32,
            dw_memory_load: u32,
            ull_total_phys: u64,
            ull_avail_phys: u64,
            ull_total_page_file: u64,
            ull_avail_page_file: u64,
            ull_total_virtual: u64,
            ull_avail_virtual: u64,
            ull_avail_extended_virtual: u64,
        }

        #[repr(C)]
        #[allow(clippy::upper_case_acronyms)]
        struct SYSTEM_POWER_STATUS {
            ac_line_status: u8,
            battery_flag: u8,
            battery_life_percent: u8,
            system_status_flag: u8,
            battery_life_time: u32,
            battery_full_life_time: u32,
        }

        extern "system" {
            fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
            fn GetSystemPowerStatus(lpSystemPowerStatus: *mut SYSTEM_POWER_STATUS) -> i32;
        }

        let mut total_memory_mb = 32768;
        let mut available_memory_mb = 16384;
        let mut battery_percent = None;
        let mut is_charging = None;

        unsafe {
            let mut mem_status: MEMORYSTATUSEX = std::mem::zeroed();
            mem_status.dw_length = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut mem_status) != 0 {
                total_memory_mb = mem_status.ull_total_phys / (1024 * 1024);
                available_memory_mb = mem_status.ull_avail_phys / (1024 * 1024);
            }

            let mut power_status: SYSTEM_POWER_STATUS = std::mem::zeroed();
            if GetSystemPowerStatus(&mut power_status) != 0 {
                if power_status.battery_life_percent <= 100 {
                    battery_percent = Some(power_status.battery_life_percent);
                }
                if power_status.ac_line_status != 255 {
                    is_charging = Some(power_status.ac_line_status == 1);
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
