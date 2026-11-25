use cpal::traits::{DeviceTrait, HostTrait};
use pico_core::error::{PicoError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_input: bool,
    pub is_default: bool,
    pub default_sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

pub struct AudioDeviceManager;

impl AudioDeviceManager {
    /// Lists all available audio input (capture) devices with negotiated default formats.
    pub fn list_input_devices() -> Result<Vec<AudioDeviceInfo>> {
        crate::host_com::with_host(|host| {
            let default_name = host.default_input_device().and_then(|d| d.name().ok());
            let devices = host.input_devices().map_err(|e| {
                PicoError::Audio(format!("Failed to enumerate input devices: {}", e))
            })?;

            let mut list = Vec::new();
            for dev in devices {
                if let Ok(name) = dev.name() {
                    let (default_sample_rate, channels, sample_format) = dev
                        .default_input_config()
                        .map(|c| {
                            (
                                c.sample_rate().0,
                                c.channels(),
                                format!("{:?}", c.sample_format()),
                            )
                        })
                        .unwrap_or((16_000, 1, "Unknown".into()));
                    let is_default = default_name.as_deref() == Some(name.as_str());
                    list.push(AudioDeviceInfo {
                        name,
                        is_input: true,
                        is_default,
                        default_sample_rate,
                        channels,
                        sample_format,
                    });
                }
            }
            Ok(list)
        })
    }

    /// Lists all available audio output (playback) devices with negotiated default formats.
    pub fn list_output_devices() -> Result<Vec<AudioDeviceInfo>> {
        crate::host_com::with_host(|host| {
            let default_name = host.default_output_device().and_then(|d| d.name().ok());
            let devices = host.output_devices().map_err(|e| {
                PicoError::Audio(format!("Failed to enumerate output devices: {}", e))
            })?;

            let mut list = Vec::new();
            for dev in devices {
                if let Ok(name) = dev.name() {
                    let (default_sample_rate, channels, sample_format) = dev
                        .default_output_config()
                        .map(|c| {
                            (
                                c.sample_rate().0,
                                c.channels(),
                                format!("{:?}", c.sample_format()),
                            )
                        })
                        .unwrap_or((48_000, 2, "Unknown".into()));
                    let is_default = default_name.as_deref() == Some(name.as_str());
                    list.push(AudioDeviceInfo {
                        name,
                        is_input: false,
                        is_default,
                        default_sample_rate,
                        channels,
                        sample_format,
                    });
                }
            }
            Ok(list)
        })
    }
}
