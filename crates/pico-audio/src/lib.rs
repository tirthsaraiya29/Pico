pub mod capture;
pub mod devices;
mod host_com;
pub mod playback;
pub mod resampler;
pub mod ringbuf;

pub use capture::AudioCapture;
pub use devices::{AudioDeviceInfo, AudioDeviceManager};
pub use playback::{AudioPlayback, PlaybackHandle};
pub use resampler::{
    decode_wav_bytes_to_f32, downmix_to_mono, resample_and_map_channels, AudioConverter,
    AudioResampler,
};
pub use ringbuf::{spsc_ring_buffer, SpscConsumer, SpscProducer};

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn build_wav_16bit_pcm(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in samples {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn test_audio_converter_chunked_resample_48k_to_16k() {
        let mut converter = AudioConverter::new(48000, 2);
        let stereo_input = vec![0.5f32; 96000];
        let pcm_16k = converter.convert_f32_to_16k_i16(&stereo_input);

        assert!((pcm_16k.len() as i32 - 16000).abs() < 120);
        assert!(!pcm_16k.is_empty());
    }

    #[test]
    fn test_playback_resampler_preserves_duration_and_frequency() {
        // Generate 1.0s of 440 Hz tone at 16,000 Hz mono (simulating SAPI output)
        let n_in = 16_000usize;
        let sine_16k: Vec<f32> = (0..n_in)
            .map(|i| (2.0 * PI * 440.0 * (i as f32 / 16_000.0)).sin() * 0.5)
            .collect();

        // Resample to 48,000 Hz stereo (typical Windows WASAPI / Linux PipeWire rate)
        let out_48k = resample_and_map_channels(&sine_16k, 16_000, 1, 48_000, 2);
        assert_eq!(out_48k.len(), 48_000 * 2);

        // Estimate fundamental frequency via zero-crossing count on left channel
        let mut zero_crossings = 0usize;
        for i in 1..48_000 {
            let prev = out_48k[(i - 1) * 2];
            let curr = out_48k[i * 2];
            if prev <= 0.0 && curr > 0.0 {
                zero_crossings += 1;
            }
        }
        assert!(
            (zero_crossings as i32 - 440).abs() <= 2,
            "Expected ~440 Hz after 16k->48k resample, got {} crossings/s",
            zero_crossings
        );

        // Also test 22,050 Hz (espeak/piper) -> 48,000 Hz stereo
        let n_espeak = 22_050usize;
        let sine_22k: Vec<f32> = (0..n_espeak)
            .map(|i| (2.0 * PI * 440.0 * (i as f32 / 22_050.0)).sin() * 0.5)
            .collect();
        let out_from_22k = resample_and_map_channels(&sine_22k, 22_050, 1, 48_000, 2);
        assert_eq!(out_from_22k.len(), 48_000 * 2);
    }

    #[test]
    fn test_wav_decoder_and_playback_handle_does_not_play_riff_header() {
        let zeros = vec![0i16; 2205]; // 100ms silence at 22.05 kHz
        let wav = build_wav_16bit_pcm(&zeros, 22_050, 1);

        let (handle, consumer) = PlaybackHandle::headless(48_000, 2);
        handle.play_pcm_bytes(&wav);

        // Resampled 100ms stereo at 48kHz = 4800 * 2 = 9600 floats; all near zero (NO RIFF ASCII spike!)
        let mut popped = vec![0.0f32; 10_000];
        let n = consumer.pop_slice(&mut popped);
        assert!(
            (n as i32 - 9600).abs() <= 4,
            "Expected ~9600 samples for 100ms 48kHz stereo, got {}",
            n
        );
        let peak = popped[..n].iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            peak < 1e-4,
            "RIFF WAV header leaked into PCM playback samples! peak={}",
            peak
        );

        // Verify raw PCM rejection of decode_wav_bytes_to_f32
        assert!(decode_wav_bytes_to_f32(&[0u8; 64]).is_err());
    }

    #[test]
    fn test_playback_interrupt_flushes_ring_immediately() {
        let (handle, consumer) = PlaybackHandle::headless(48_000, 1);
        let tone = vec![0.25f32; 8000];
        handle.play_audio(&tone, 16_000, 1);
        assert!(handle.is_playing());
        handle.interrupt();
        assert!(!handle.is_playing());
        let mut buf = [0.0f32; 64];
        assert_eq!(consumer.pop_slice(&mut buf), 0);
    }

    #[test]
    fn test_enumerate_devices() {
        match AudioDeviceManager::list_input_devices() {
            Ok(inputs) => {
                for dev in inputs {
                    assert!(dev.is_input);
                    assert!(!dev.name.is_empty());
                    assert!(dev.default_sample_rate >= 8_000);
                    assert!(dev.channels >= 1);
                }
            }
            Err(e) => {
                // Headless CI runners (Windows Server / Linux containers) may lack an active OS audio daemon.
                assert!(!e.to_string().is_empty());
            }
        }

        match AudioDeviceManager::list_output_devices() {
            Ok(outputs) => {
                for dev in outputs {
                    assert!(!dev.is_input);
                    assert!(!dev.name.is_empty());
                    assert!(dev.default_sample_rate >= 8_000);
                    assert!(dev.channels >= 1);
                }
            }
            Err(e) => {
                assert!(!e.to_string().is_empty());
            }
        }
    }
}
