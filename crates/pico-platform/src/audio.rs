//! Native OS master audio endpoint control.
//! Uses Windows Core Audio WASAPI COM (`IAudioEndpointVolume`) directly on Windows
//! and PipeWire (`wpctl`) / PulseAudio (`pactl`) / ALSA (`amixer`) on Linux.

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use pico_core::error::PicoError;
use pico_core::error::Result;
use serde::{Deserialize, Serialize};

/// Snapshot of the default output audio endpoint volume and mute status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioEndpointState {
    pub volume_percent: u32,
    pub muted: bool,
    pub backend: String,
}

pub struct PlatformAudio;

impl PlatformAudio {
    pub fn get_state() -> Result<AudioEndpointState> {
        #[cfg(target_os = "windows")]
        {
            win_wasapi::get_state()
        }
        #[cfg(target_os = "linux")]
        {
            linux_audio::get_state()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(PicoError::Platform(
                "Audio endpoint control is not supported on this OS".into(),
            ))
        }
    }

    pub fn set_volume(percent: u32) -> Result<AudioEndpointState> {
        let clamped = percent.min(100);
        #[cfg(target_os = "windows")]
        {
            win_wasapi::set_volume(clamped)
        }
        #[cfg(target_os = "linux")]
        {
            linux_audio::set_volume(clamped)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = clamped;
            Err(PicoError::Platform(
                "Audio endpoint control is not supported on this OS".into(),
            ))
        }
    }

    pub fn set_mute(muted: bool) -> Result<AudioEndpointState> {
        #[cfg(target_os = "windows")]
        {
            win_wasapi::set_mute(muted)
        }
        #[cfg(target_os = "linux")]
        {
            linux_audio::set_mute(muted)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = muted;
            Err(PicoError::Platform(
                "Audio endpoint control is not supported on this OS".into(),
            ))
        }
    }

    pub fn toggle_mute() -> Result<AudioEndpointState> {
        let current = Self::get_state()?;
        Self::set_mute(!current.muted)
    }
}

#[cfg(target_os = "windows")]
mod win_wasapi {
    use super::AudioEndpointState;
    use pico_core::error::{PicoError, Result};
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    struct ComGuard {
        should_uninit: bool,
    }

    impl ComGuard {
        fn init() -> Result<Self> {
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                Ok(Self {
                    should_uninit: true,
                })
            } else if hr == RPC_E_CHANGED_MODE {
                Ok(Self {
                    should_uninit: false,
                })
            } else {
                Err(PicoError::Platform(format!(
                    "COM initialization failed: {}",
                    hr.message()
                )))
            }
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.should_uninit {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn with_endpoint_volume<F, T>(f: F) -> Result<T>
    where
        F: FnOnce(&IAudioEndpointVolume) -> Result<T>,
    {
        let _com = ComGuard::init()?;
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                    PicoError::Platform(format!("MMDeviceEnumerator unavailable: {}", e))
                })?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| {
                    PicoError::Platform(format!("No active default output audio endpoint: {}", e))
                })?;
            let volume: IAudioEndpointVolume = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| PicoError::Platform(format!("IAudioEndpointVolume error: {}", e)))?;
            f(&volume)
        }
    }

    pub fn get_state() -> Result<AudioEndpointState> {
        with_endpoint_volume(|vol| unsafe {
            let scalar = vol
                .GetMasterVolumeLevelScalar()
                .map_err(|e| PicoError::Platform(format!("Failed reading master volume: {}", e)))?;
            let muted = vol
                .GetMute()
                .map_err(|e| PicoError::Platform(format!("Failed reading mute state: {}", e)))?
                .as_bool();
            Ok(AudioEndpointState {
                volume_percent: (scalar * 100.0).round().clamp(0.0, 100.0) as u32,
                muted,
                backend: "wasapi".into(),
            })
        })
    }

    pub fn set_volume(percent: u32) -> Result<AudioEndpointState> {
        let scalar = (percent as f32 / 100.0).clamp(0.0, 1.0);
        with_endpoint_volume(|vol| unsafe {
            vol.SetMasterVolumeLevelScalar(scalar, std::ptr::null())
                .map_err(|e| PicoError::Platform(format!("Failed setting master volume: {}", e)))?;
            let readback = vol
                .GetMasterVolumeLevelScalar()
                .map_err(|e| PicoError::Platform(format!("Failed verifying volume: {}", e)))?;
            let muted = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            Ok(AudioEndpointState {
                volume_percent: (readback * 100.0).round().clamp(0.0, 100.0) as u32,
                muted,
                backend: "wasapi".into(),
            })
        })
    }

    pub fn set_mute(muted: bool) -> Result<AudioEndpointState> {
        with_endpoint_volume(|vol| unsafe {
            vol.SetMute(muted, std::ptr::null())
                .map_err(|e| PicoError::Platform(format!("Failed setting mute state: {}", e)))?;
            let readback_mute = vol
                .GetMute()
                .map_err(|e| PicoError::Platform(format!("Failed verifying mute state: {}", e)))?
                .as_bool();
            let scalar = vol.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            Ok(AudioEndpointState {
                volume_percent: (scalar * 100.0).round().clamp(0.0, 100.0) as u32,
                muted: readback_mute,
                backend: "wasapi".into(),
            })
        })
    }
}

#[cfg(target_os = "linux")]
mod linux_audio {
    use super::AudioEndpointState;
    use pico_core::error::{PicoError, Result};
    use std::process::Command;

    pub fn get_state() -> Result<AudioEndpointState> {
        if let Ok(out) = Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                // Example: "Volume: 0.65 [MUTED]" or "Volume: 0.65"
                if let Some(num) = text
                    .split_whitespace()
                    .find_map(|w| w.parse::<f32>().ok())
                {
                    let muted = text.contains("[MUTED]");
                    return Ok(AudioEndpointState {
                        volume_percent: (num * 100.0).round().clamp(0.0, 100.0) as u32,
                        muted,
                        backend: "wpctl".into(),
                    });
                }
            }
        }

        if let (Ok(vol_out), Ok(mute_out)) = (
            Command::new("pactl")
                .args(["get-sink-volume", "@DEFAULT_SINK@"])
                .output(),
            Command::new("pactl")
                .args(["get-sink-mute", "@DEFAULT_SINK@"])
                .output(),
        ) {
            if vol_out.status.success() && mute_out.status.success() {
                let vtext = String::from_utf8_lossy(&vol_out.stdout);
                let pct = vtext
                    .split_whitespace()
                    .find_map(|tok| tok.strip_suffix('%').and_then(|n| n.parse::<u32>().ok()))
                    .unwrap_or(50);
                let muted = String::from_utf8_lossy(&mute_out.stdout)
                    .to_lowercase()
                    .contains("yes");
                return Ok(AudioEndpointState {
                    volume_percent: pct.min(100),
                    muted,
                    backend: "pactl".into(),
                });
            }
        }

        Err(PicoError::Platform(
            "No compatible Linux audio control tool (wpctl or pactl) is available".into(),
        ))
    }

    pub fn set_volume(percent: u32) -> Result<AudioEndpointState> {
        let wp_arg = format!("{:.2}", (percent as f32 / 100.0).clamp(0.0, 1.0));
        if let Ok(st) = Command::new("wpctl")
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &wp_arg])
            .status()
        {
            if st.success() {
                return get_state().or(Ok(AudioEndpointState {
                    volume_percent: percent,
                    muted: false,
                    backend: "wpctl".into(),
                }));
            }
        }

        if let Ok(st) = Command::new("pactl")
            .args([
                "set-sink-volume",
                "@DEFAULT_SINK@",
                &format!("{}%", percent),
            ])
            .status()
        {
            if st.success() {
                return get_state().or(Ok(AudioEndpointState {
                    volume_percent: percent,
                    muted: false,
                    backend: "pactl".into(),
                }));
            }
        }

        Err(PicoError::Platform(
            "Failed to set volume via wpctl or pactl".into(),
        ))
    }

    pub fn set_mute(muted: bool) -> Result<AudioEndpointState> {
        let wp_val = if muted { "1" } else { "0" };
        if let Ok(st) = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", wp_val])
            .status()
        {
            if st.success() {
                return get_state().or(Ok(AudioEndpointState {
                    volume_percent: 50,
                    muted,
                    backend: "wpctl".into(),
                }));
            }
        }

        let pactl_val = if muted { "1" } else { "0" };
        if let Ok(st) = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", pactl_val])
            .status()
        {
            if st.success() {
                return get_state().or(Ok(AudioEndpointState {
                    volume_percent: 50,
                    muted,
                    backend: "pactl".into(),
                }));
            }
        }

        Err(PicoError::Platform(
            "Failed to set mute state via wpctl or pactl".into(),
        ))
    }
}
