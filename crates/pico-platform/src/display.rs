//! Native display brightness control across Windows and Linux.

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use pico_core::error::PicoError;
use pico_core::error::Result;

pub struct PlatformDisplay;

impl PlatformDisplay {
    pub fn get_brightness() -> Result<u32> {
        #[cfg(target_os = "windows")]
        {
            win_display::get_brightness()
        }
        #[cfg(target_os = "linux")]
        {
            linux_display::get_brightness()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(PicoError::Platform(
                "Display brightness control is unsupported on this OS".into(),
            ))
        }
    }

    pub fn set_brightness(level: u32) -> Result<u32> {
        let clamped = level.min(100);
        #[cfg(target_os = "windows")]
        {
            win_display::set_brightness(clamped)
        }
        #[cfg(target_os = "linux")]
        {
            linux_display::set_brightness(clamped)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = clamped;
            Err(PicoError::Platform(
                "Display brightness control is unsupported on this OS".into(),
            ))
        }
    }

    pub fn adjust_brightness(delta: i32) -> Result<u32> {
        let curr = Self::get_brightness().unwrap_or(50) as i32;
        let next = (curr + delta).clamp(0, 100) as u32;
        Self::set_brightness(next)
    }
}

#[cfg(target_os = "windows")]
mod win_display {
    use pico_core::error::{PicoError, Result};
    use std::process::Command;
    use windows::Win32::Devices::Display::{
        DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
        GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
    };
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    fn try_dxva2_get() -> Option<u32> {
        unsafe {
            let hwnd = GetDesktopWindow();
            let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count).is_err() || count == 0 {
                return None;
            }
            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            if GetPhysicalMonitorsFromHMONITOR(hmon, &mut monitors).is_err() {
                return None;
            }
            let mut min_b = 0u32;
            let mut cur_b = 0u32;
            let mut max_b = 0u32;
            let ok = GetMonitorBrightness(
                monitors[0].hPhysicalMonitor,
                &mut min_b,
                &mut cur_b,
                &mut max_b,
            ) != 0;
            let _ = DestroyPhysicalMonitors(&monitors);
            if ok && max_b > min_b {
                let pct = ((cur_b - min_b) as f32 / (max_b - min_b) as f32) * 100.0;
                Some(pct.round().clamp(0.0, 100.0) as u32)
            } else {
                None
            }
        }
    }

    fn try_dxva2_set(percent: u32) -> Option<u32> {
        unsafe {
            let hwnd = GetDesktopWindow();
            let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count).is_err() || count == 0 {
                return None;
            }
            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            if GetPhysicalMonitorsFromHMONITOR(hmon, &mut monitors).is_err() {
                return None;
            }
            let mut min_b = 0u32;
            let mut cur_b = 0u32;
            let mut max_b = 0u32;
            let mut updated = None;
            if GetMonitorBrightness(
                monitors[0].hPhysicalMonitor,
                &mut min_b,
                &mut cur_b,
                &mut max_b,
            ) != 0
                && max_b > min_b
            {
                let target = min_b + (((max_b - min_b) * percent) / 100);
                if SetMonitorBrightness(monitors[0].hPhysicalMonitor, target) != 0 {
                    updated = Some(percent);
                }
            }
            let _ = DestroyPhysicalMonitors(&monitors);
            updated
        }
    }

    pub fn get_brightness() -> Result<u32> {
        if let Some(v) = try_dxva2_get() {
            return Ok(v);
        }
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Select-Object -First 1 -ExpandProperty CurrentBrightness)",
            ])
            .output()
            .map_err(|e| PicoError::Platform(format!("Failed querying brightness: {}", e)))?;
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(val) = s.parse::<u32>() {
                return Ok(val.min(100));
            }
        }
        Err(PicoError::Platform(
            "Display hardware does not expose controllable brightness on this device".into(),
        ))
    }

    pub fn set_brightness(percent: u32) -> Result<u32> {
        if let Some(v) = try_dxva2_set(percent) {
            return Ok(v);
        }
        let output = Command::new("powershell")
            .env("PICO_BRIGHTNESS", percent.to_string())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$lvl = [byte]$env:PICO_BRIGHTNESS; (Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Select-Object -First 1) | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{ Timeout = 1; Brightness = $lvl } | Out-Null",
            ])
            .output()
            .map_err(|e| PicoError::Platform(format!("Failed setting display brightness: {}", e)))?;
        if !output.status.success() {
            return Err(PicoError::Platform(format!(
                "Display brightness control failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(percent)
    }
}

#[cfg(target_os = "linux")]
mod linux_display {
    use pico_core::error::{PicoError, Result};
    use std::path::PathBuf;
    use std::process::Command;

    fn find_backlight_dir() -> Option<PathBuf> {
        let dir = PathBuf::from("/sys/class/backlight");
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.join("brightness").exists() && path.join("max_brightness").exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    pub fn get_brightness() -> Result<u32> {
        if let Some(dir) = find_backlight_dir() {
            if let (Ok(cur), Ok(max)) = (
                std::fs::read_to_string(dir.join("brightness")),
                std::fs::read_to_string(dir.join("max_brightness")),
            ) {
                if let (Ok(c), Ok(m)) = (cur.trim().parse::<u32>(), max.trim().parse::<u32>()) {
                    if m > 0 {
                        return Ok(((c as f32 / m as f32) * 100.0).round().clamp(0.0, 100.0) as u32);
                    }
                }
            }
        }
        Err(PicoError::Platform(
            "No controllable /sys/class/backlight device found".into(),
        ))
    }

    pub fn set_brightness(percent: u32) -> Result<u32> {
        if let Ok(st) = Command::new("brightnessctl")
            .args(["set", &format!("{}%", percent)])
            .status()
        {
            if st.success() {
                return Ok(percent);
            }
        }
        if let Some(dir) = find_backlight_dir() {
            if let Ok(max_str) = std::fs::read_to_string(dir.join("max_brightness")) {
                if let Ok(max) = max_str.trim().parse::<u32>() {
                    let raw = (max * percent) / 100;
                    std::fs::write(dir.join("brightness"), raw.to_string()).map_err(|e| {
                        PicoError::Platform(format!("Failed writing backlight brightness: {}", e))
                    })?;
                    return Ok(percent);
                }
            }
        }
        Err(PicoError::Platform(
            "Failed to adjust display brightness cleanly".into(),
        ))
    }
}
