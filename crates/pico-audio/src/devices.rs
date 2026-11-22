use cpal::traits::{DeviceTrait, HostTrait};
use pico_core::error::{PicoError, Result};
use tracing::info;

#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub supported_sample_rates: Vec<u32>,
}

pub struct AudioDeviceManager;

impl AudioDeviceManager {
    /// Lists all available input (microphone) devices.
    pub fn list_input_devices() -> Result<Vec<AudioDeviceInfo>> {
        let host = cpal::default_host();
        let default_device_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let devices = host
            .input_devices()
            .map_err(|e| PicoError::Audio(format!("Failed to enumerate input devices: {}", e)))?;

        let mut list = Vec::new();
        for dev in devices {
            if let Ok(name) = dev.name() {
                let is_default = name == default_device_name;
                let sample_rates = dev
                    .supported_input_configs()
                    .map(|configs| configs.map(|c| c.max_sample_rate().0).collect::<Vec<u32>>())
                    .unwrap_or_default();

                list.push(AudioDeviceInfo {
                    name,
                    is_default,
                    supported_sample_rates: sample_rates,
                });
            }
        }
        info!(count = list.len(), "Enumerated audio input devices");
        Ok(list)
    }

    /// Lists all available output (speaker/headphone) devices.
    pub fn list_output_devices() -> Result<Vec<AudioDeviceInfo>> {
        let host = cpal::default_host();
        let default_device_name = host
            .default_output_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let devices = host
            .output_devices()
            .map_err(|e| PicoError::Audio(format!("Failed to enumerate output devices: {}", e)))?;

        let mut list = Vec::new();
        for dev in devices {
            if let Ok(name) = dev.name() {
                let is_default = name == default_device_name;
                let sample_rates = dev
                    .supported_output_configs()
                    .map(|configs| configs.map(|c| c.max_sample_rate().0).collect::<Vec<u32>>())
                    .unwrap_or_default();

                list.push(AudioDeviceInfo {
                    name,
                    is_default,
                    supported_sample_rates: sample_rates,
                });
            }
        }
        info!(count = list.len(), "Enumerated audio output devices");
        Ok(list)
    }
}
