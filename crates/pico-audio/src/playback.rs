use crate::resampler::{decode_wav_bytes_to_f32, resample_and_map_channels};
use crate::ringbuf::{spsc_ring_buffer, SpscConsumer, SpscProducer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pico_core::error::{PicoError, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tracing::{debug, error, info, warn};

/// Thread-safe handle to the hardware playback stream.
/// Producer serialization stays strictly off the real-time audio callback thread;
/// the OS audio thread reads from a bounded lock-free SPSC ring buffer with zero locks/allocs.
#[derive(Clone)]
pub struct PlaybackHandle {
    producer: Arc<Mutex<SpscProducer<f32>>>,
    is_playing: Arc<AtomicBool>,
    interrupt_epoch: Arc<AtomicU64>,
    output_sample_rate: u32,
    output_channels: u16,
}

impl PlaybackHandle {
    /// Creates an offline testable handle configured for an explicit target hardware format.
    pub fn headless(output_sample_rate: u32, output_channels: u16) -> (Self, SpscConsumer<f32>) {
        let cap = (output_sample_rate.max(8000) as usize) * (output_channels.max(1) as usize) * 10;
        let (prod, cons) = spsc_ring_buffer::<f32>(cap);
        let handle = Self {
            producer: Arc::new(Mutex::new(prod)),
            is_playing: Arc::new(AtomicBool::new(false)),
            interrupt_epoch: Arc::new(AtomicU64::new(0)),
            output_sample_rate: output_sample_rate.max(8000),
            output_channels: output_channels.max(1),
        };
        (handle, cons)
    }

    pub fn output_sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    pub fn output_channels(&self) -> u16 {
        self.output_channels
    }

    /// Resamples and maps an explicit `(source_rate, source_channels)` f32 waveform to the device
    /// hardware format, streaming into the lock-free playback ring buffer until queued or interrupted.
    pub fn play_audio(&self, samples: &[f32], source_rate: u32, source_channels: u16) {
        if samples.is_empty() || source_rate == 0 || source_channels == 0 {
            return;
        }
        let converted = resample_and_map_channels(
            samples,
            source_rate,
            source_channels,
            self.output_sample_rate,
            self.output_channels,
        );
        if converted.is_empty() {
            return;
        }
        self.push_resampled_interleaved(&converted);
    }

    /// Decodes a RIFF/WAVE container explicitly (honoring its header rate/channels/bit depth)
    /// and resamples to the actual hardware device rate before playback.
    pub fn play_wav_bytes(&self, wav_bytes: &[u8]) -> Result<()> {
        let (samples, src_rate, src_ch) = decode_wav_bytes_to_f32(wav_bytes)?;
        self.play_audio(&samples, src_rate, src_ch);
        Ok(())
    }

    /// Enqueues 16kHz signed 16-bit mono PCM samples, converting to hardware output sample rate & channels.
    pub fn play_pcm_samples(&self, samples: &[i16]) {
        let f32_samples: Vec<f32> = samples
            .iter()
            .map(|&s| (s as f32 / 32768.0).clamp(-1.0, 1.0))
            .collect();
        self.play_audio(&f32_samples, 16_000, 1);
    }

    /// Enqueues raw 16kHz 16-bit LE mono PCM bytes, OR explicitly decodes RIFF WAV if framed with WAV headers.
    pub fn play_pcm_bytes(&self, bytes: &[u8]) {
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            if let Err(e) = self.play_wav_bytes(bytes) {
                warn!(error = %e, "Failed decoding WAV payload for playback");
            }
            return;
        }
        let mut samples = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.as_chunks::<2>().0 {
            samples.push(i16::from_le_bytes(*chunk));
        }
        self.play_pcm_samples(&samples);
    }

    fn push_resampled_interleaved(&self, data: &[f32]) {
        let my_epoch = self.interrupt_epoch.load(Ordering::Acquire);
        self.is_playing.store(true, Ordering::Release);

        let mut offset = 0usize;
        while offset < data.len() {
            if self.interrupt_epoch.load(Ordering::Acquire) != my_epoch {
                return;
            }
            let written = {
                let guard = self.producer.lock().unwrap();
                guard.push_slice(&data[offset..])
            };
            offset += written;
            if offset < data.len() {
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        }
    }

    /// Halts playback immediately using lock-free SPSC clear signaling (barge-in).
    pub fn interrupt(&self) {
        self.interrupt_epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(guard) = self.producer.lock() {
            guard.request_clear();
        }
        self.is_playing.store(false, Ordering::Release);
        debug!("Playback buffer cleared immediately via barge-in interrupt");
    }

    /// Returns true if audio samples remain queued or active in the hardware stream.
    pub fn is_playing(&self) -> bool {
        if let Ok(guard) = self.producer.lock() {
            if !guard.is_empty() {
                return true;
            }
        }
        self.is_playing.load(Ordering::Acquire)
    }
}

pub struct AudioPlayback {
    handle: PlaybackHandle,
    stream_id: Option<u64>,
    _headless_shutdown_tx: Option<mpsc::Sender<()>>,
}

impl AudioPlayback {
    /// Starts audio output stream on the specified or default audio device,
    /// using the device's actual negotiated sample rate and channel count cleanly.
    pub fn start(device_name: Option<&str>) -> Result<Self> {
        let dev_name_opt = device_name.map(|s| s.to_string());
        let (stream_id, handle) = crate::host_com::spawn_stream(move |host| {
            let device = if let Some(ref target_name) = dev_name_opt {
                match host.output_devices() {
                    Ok(mut devs) => devs
                        .find(|d| d.name().map(|n| n.contains(target_name)).unwrap_or(false))
                        .or_else(|| host.default_output_device()),
                    Err(_) => host.default_output_device(),
                }
            } else {
                host.default_output_device()
            };

            let Some(device) = device else {
                return Err(PicoError::Audio(
                    "No default audio output device found".to_string(),
                ));
            };

            let dev_name = device.name().unwrap_or_else(|_| "Unknown".into());
            let default_config = device
                .default_output_config()
                .map_err(|e| PicoError::Audio(format!("Failed to get output config: {}", e)))?;

            let sample_format = default_config.sample_format();
            let output_sample_rate = default_config.sample_rate().0;
            let output_channels = default_config.channels();
            let config: cpal::StreamConfig = default_config.into();

            // 16-second ring buffer at negotiated hardware rate prevents overflow and zero-allocates in RT callback
            let ring_cap = (output_sample_rate as usize * output_channels as usize * 16).max(32768);
            let (producer, consumer) = spsc_ring_buffer::<f32>(ring_cap);
            let is_playing = Arc::new(AtomicBool::new(false));

            let handle = PlaybackHandle {
                producer: Arc::new(Mutex::new(producer)),
                is_playing: is_playing.clone(),
                interrupt_epoch: Arc::new(AtomicU64::new(0)),
                output_sample_rate,
                output_channels,
            };

            let err_fn = |err| {
                error!("Audio output stream error: {}", err);
            };

            let playing_flag = is_playing;
            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let popped = consumer.pop_slice(data);
                        for slot in &mut data[popped..] {
                            *slot = 0.0;
                        }
                        playing_flag.store(popped > 0 && !consumer.is_empty(), Ordering::Release);
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        let mut active = false;
                        for dst in data.iter_mut() {
                            if let Some(sample) = consumer.pop_one() {
                                *dst = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                                active = true;
                            } else {
                                *dst = 0;
                            }
                        }
                        playing_flag.store(active && !consumer.is_empty(), Ordering::Release);
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I32 => device.build_output_stream(
                    &config,
                    move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                        let mut active = false;
                        for dst in data.iter_mut() {
                            if let Some(sample) = consumer.pop_one() {
                                *dst = (sample.clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
                                active = true;
                            } else {
                                *dst = 0;
                            }
                        }
                        playing_flag.store(active && !consumer.is_empty(), Ordering::Release);
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::U8 => device.build_output_stream(
                    &config,
                    move |data: &mut [u8], _: &cpal::OutputCallbackInfo| {
                        let mut active = false;
                        for dst in data.iter_mut() {
                            if let Some(sample) = consumer.pop_one() {
                                *dst = ((sample.clamp(-1.0, 1.0) + 1.0) * 127.5).round() as u8;
                                active = true;
                            } else {
                                *dst = 128;
                            }
                        }
                        playing_flag.store(active && !consumer.is_empty(), Ordering::Release);
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                        let mut active = false;
                        for dst in data.iter_mut() {
                            if let Some(sample) = consumer.pop_one() {
                                *dst = ((sample.clamp(-1.0, 1.0) + 1.0) * 32767.5).round() as u16;
                                active = true;
                            } else {
                                *dst = 32768;
                            }
                        }
                        playing_flag.store(active && !consumer.is_empty(), Ordering::Release);
                    },
                    err_fn,
                    None,
                ),
                other => {
                    return Err(PicoError::Audio(format!(
                        "Unsupported audio output sample format: {:?}",
                        other
                    )));
                }
            }
            .map_err(|e| PicoError::Audio(format!("Failed to build output stream: {}", e)))?;

            stream.play().map_err(|e| {
                PicoError::Audio(format!("Failed to play audio output stream: {}", e))
            })?;
            info!(
                device = %dev_name,
                sample_rate = output_sample_rate,
                channels = output_channels,
                format = ?sample_format,
                "Hardware audio playback initialized"
            );
            Ok((stream, handle))
        })?;

        Ok(Self {
            handle,
            stream_id: Some(stream_id),
            _headless_shutdown_tx: None,
        })
    }

    /// Gracefully returns a headless non-hardware fallback handle when audio output is disabled/unavailable in tests.
    pub fn headless() -> Self {
        let (handle, consumer) = PlaybackHandle::headless(48_000, 2);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
        let playing_flag = handle.is_playing.clone();

        std::thread::Builder::new()
            .name("pico-audio-headless-drain".into())
            .spawn(move || {
                let mut scratch = vec![0.0f32; 1920];
                loop {
                    match shutdown_rx.try_recv() {
                        Ok(_) | Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    let popped = consumer.pop_slice(&mut scratch);
                    playing_flag.store(!consumer.is_empty(), Ordering::Release);
                    if popped == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    } else {
                        // Drain 1920 interleaved 48kHz stereo frames (~20ms) cleanly.
                        std::thread::sleep(std::time::Duration::from_millis(15));
                    }
                }
            })
            .ok();

        Self {
            handle,
            stream_id: None,
            _headless_shutdown_tx: Some(shutdown_tx),
        }
    }

    pub fn handle(&self) -> PlaybackHandle {
        self.handle.clone()
    }

    pub fn play_audio(&self, samples: &[f32], source_rate: u32, source_channels: u16) {
        self.handle
            .play_audio(samples, source_rate, source_channels);
    }

    pub fn play_pcm_samples(&self, samples: &[i16]) {
        self.handle.play_pcm_samples(samples);
    }

    pub fn play_pcm_bytes(&self, bytes: &[u8]) {
        self.handle.play_pcm_bytes(bytes);
    }

    pub fn interrupt(&self) {
        self.handle.interrupt();
    }

    pub fn is_playing(&self) -> bool {
        self.handle.is_playing()
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.handle.interrupt();
        if let Some(id) = self.stream_id.take() {
            crate::host_com::drop_stream(id);
        }
        self._headless_shutdown_tx.take();
    }
}
