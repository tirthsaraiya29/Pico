use crate::resampler::AudioConverter;
use crate::ringbuf::spsc_ring_buffer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pico_core::error::{PicoError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{error, info};

/// Captures live audio on OS thread using a zero-lock, zero-allocation SPSC ring buffer
/// inside the hardware callback and converts on a dedicated consumer worker to 16 kHz mono PCM i16.
pub struct AudioCapture {
    is_capturing: Arc<AtomicBool>,
    stream_id: u64,
    worker_stop: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

impl AudioCapture {
    pub fn start(
        device_name: Option<&str>,
        sample_sender: tokio_mpsc::Sender<Vec<i16>>,
    ) -> Result<Self> {
        let dev_name_opt = device_name.map(|s| s.to_string());
        let (stream_id, (is_capturing, sample_rate, channels, consumer)) =
            crate::host_com::spawn_stream(move |host| {
                let device = if let Some(ref target_name) = dev_name_opt {
                    match host.input_devices() {
                        Ok(mut devs) => devs
                            .find(|d| d.name().map(|n| n.contains(target_name)).unwrap_or(false))
                            .or_else(|| host.default_input_device()),
                        Err(_) => host.default_input_device(),
                    }
                } else {
                    host.default_input_device()
                };

                let Some(device) = device else {
                    return Err(PicoError::Audio(
                        "No default audio input device found".to_string(),
                    ));
                };

                let dev_name = device.name().unwrap_or_else(|_| "Unknown".into());
                let default_config = device
                    .default_input_config()
                    .map_err(|e| PicoError::Audio(format!("Failed to get input config: {}", e)))?;

                let sample_format = default_config.sample_format();
                let sample_rate = default_config.sample_rate().0;
                let channels = default_config.channels();
                let config: cpal::StreamConfig = default_config.into();

                // Capture RT callback writes raw normalized f32 directly into lock-free ring (no alloc/mutex)
                let ring_cap = (sample_rate as usize * channels as usize * 4).max(16384);
                let (producer, consumer) = spsc_ring_buffer::<f32>(ring_cap);
                let is_capturing = Arc::new(AtomicBool::new(true));
                let active_cb = is_capturing.clone();

                let err_fn = |err| {
                    error!("Audio capture stream error: {}", err);
                };

                let stream = match sample_format {
                    cpal::SampleFormat::F32 => device.build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if active_cb.load(Ordering::Relaxed) {
                                let _ = producer.push_slice(data);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I16 => device.build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if active_cb.load(Ordering::Relaxed) {
                                // Push directly via fixed stack chunk (0 heap allocation)
                                let mut tmp = [0.0f32; 512];
                                for ch in data.chunks(512) {
                                    for (dst, &s) in tmp.iter_mut().zip(ch.iter()) {
                                        *dst = (s as f32) / 32768.0;
                                    }
                                    let _ = producer.push_slice(&tmp[..ch.len()]);
                                }
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I32 => device.build_input_stream(
                        &config,
                        move |data: &[i32], _: &cpal::InputCallbackInfo| {
                            if active_cb.load(Ordering::Relaxed) {
                                let mut tmp = [0.0f32; 512];
                                for ch in data.chunks(512) {
                                    for (dst, &s) in tmp.iter_mut().zip(ch.iter()) {
                                        *dst = (s as f32) / 2_147_483_648.0;
                                    }
                                    let _ = producer.push_slice(&tmp[..ch.len()]);
                                }
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::U8 => device.build_input_stream(
                        &config,
                        move |data: &[u8], _: &cpal::InputCallbackInfo| {
                            if active_cb.load(Ordering::Relaxed) {
                                let mut tmp = [0.0f32; 512];
                                for ch in data.chunks(512) {
                                    for (dst, &s) in tmp.iter_mut().zip(ch.iter()) {
                                        *dst = (s as f32 - 128.0) / 128.0;
                                    }
                                    let _ = producer.push_slice(&tmp[..ch.len()]);
                                }
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_input_stream(
                        &config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            if active_cb.load(Ordering::Relaxed) {
                                let mut tmp = [0.0f32; 512];
                                for ch in data.chunks(512) {
                                    for (dst, &s) in tmp.iter_mut().zip(ch.iter()) {
                                        *dst = (s as f32 - 32768.0) / 32768.0;
                                    }
                                    let _ = producer.push_slice(&tmp[..ch.len()]);
                                }
                            }
                        },
                        err_fn,
                        None,
                    ),
                    other => {
                        return Err(PicoError::Audio(format!(
                            "Unsupported audio input sample format: {:?}",
                            other
                        )));
                    }
                }
                .map_err(|e| {
                    PicoError::Audio(format!("Failed to build audio input stream: {}", e))
                })?;

                stream.play().map_err(|e| {
                    PicoError::Audio(format!("Failed to play audio input stream: {}", e))
                })?;
                info!(
                    device = %dev_name,
                    sample_rate,
                    channels,
                    format = ?sample_format,
                    "Hardware audio capture initialized (lock-free SPSC)"
                );
                Ok((stream, (is_capturing, sample_rate, channels, consumer)))
            })?;

        let worker_stop = Arc::new(AtomicBool::new(false));
        let worker_stop_rx = worker_stop.clone();
        let active_worker = is_capturing.clone();

        let worker_handle = std::thread::spawn(move || {
            let mut converter = AudioConverter::new(sample_rate, channels);
            let frame_samples = ((sample_rate as usize / 50).max(128)) * (channels as usize);
            let mut raw_buf = vec![0.0f32; frame_samples];

            while !worker_stop_rx.load(Ordering::Acquire) {
                if !active_worker.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
                let popped = consumer.pop_slice(&mut raw_buf);
                if popped == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                let pcm_16k = converter.convert_f32_to_16k_i16(&raw_buf[..popped]);
                if !pcm_16k.is_empty() && sample_sender.try_send(pcm_16k).is_err() {
                    // Backpressure: drop only if downstream consumer is paused/full
                }
            }
        });

        Ok(Self {
            is_capturing,
            stream_id,
            worker_stop,
            worker_handle: Some(worker_handle),
        })
    }

    pub fn set_paused(&self, paused: bool) {
        self.is_capturing.store(!paused, Ordering::Release);
    }

    pub fn is_capturing(&self) -> bool {
        self.is_capturing.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.is_capturing.store(false, Ordering::Release);
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
        crate::host_com::drop_stream(self.stream_id);
        self.worker_stop.store(true, Ordering::Release);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
