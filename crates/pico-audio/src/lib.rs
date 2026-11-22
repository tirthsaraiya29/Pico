pub mod capture;
pub mod devices;
pub mod playback;
pub mod resampler;

pub use capture::AudioCapture;
pub use devices::{AudioDeviceInfo, AudioDeviceManager};
pub use playback::{AudioPlayback, PlaybackHandle};
pub use resampler::AudioConverter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_converter_resample() {
        let converter = AudioConverter::new(48000, 2);
        let stereo_input = vec![0.5f32; 96000];
        let pcm_16k = converter.convert_f32_to_16k_i16(&stereo_input);

        assert!((pcm_16k.len() as i32 - 16000).abs() < 50);
        assert!(!pcm_16k.is_empty());
    }

    #[test]
    fn test_enumerate_devices() {
        let input_res = AudioDeviceManager::list_input_devices();
        assert!(input_res.is_ok());

        let output_res = AudioDeviceManager::list_output_devices();
        assert!(output_res.is_ok());
    }
}
