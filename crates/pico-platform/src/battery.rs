use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryStatus {
    pub percent: u8,
    pub on_ac_power: bool,
    pub charging: bool,
}

/// Reads real battery level and AC status on Windows and Linux laptops; returns `None` on desktops.
pub fn get_battery_status() -> Option<BatteryStatus> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
            // 128 indicates "no system battery"; 255 indicates unknown percent.
            if status.BatteryFlag == 128 || status.BatteryLifePercent > 100 {
                return None;
            }
            return Some(BatteryStatus {
                percent: status.BatteryLifePercent,
                on_ac_power: status.ACLineStatus == 1,
                charging: (status.BatteryFlag & 8) != 0,
            });
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
            return None;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
            if kind.trim() != "Battery" {
                continue;
            }
            let cap_raw = std::fs::read_to_string(p.join("capacity")).ok()?;
            let percent = cap_raw.trim().parse::<u8>().ok()?.min(100);
            let status_raw = std::fs::read_to_string(p.join("status"))
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            let charging = status_raw == "charging";
            let on_ac = charging || status_raw == "full" || status_raw == "not charging";
            return Some(BatteryStatus {
                percent,
                on_ac_power: on_ac,
                charging,
            });
        }
        None
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}
