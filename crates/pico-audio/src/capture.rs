use crate::resampler::AudioConverter;
use crate::ringbuf::spsc_ring_buffer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pico_core::error::{PicoError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{error, info, warn};

/// Captures live audio on OS thread using a zero-lock, zero-allocation SPSC ring buffer
/// inside the hardware callback and converts on a dedicated consumer worker to 16 kHz mono PCM i16.
pub struct AudioCapture {
    is_capturing: Arc<AtomicBool>,
    _shutdown_tx: Option<mpsc::Sender<()>>,
}

impl AudioCapture {
    pub fn start(
        device_name: Option<&str>,
        sample_sender: tokio_mpsc::Sender<Vec<i16>>,
    ) -> Result<Self> {
        let dev_name_opt = device_name.map(|s| s.to_string());
        let (init_tx, init_rx) = mpsc::channel::<Result<Arc<AtomicBool>>>();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        std::thread::spawn(move || {
            let host = cpal::default_host();
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
                let _ = init_tx.send(Err(PicoError::Audio(
                    "No default audio input device found".to_string(),
                )));
                return;
            };

            let dev_name = device.name().unwrap_or_else(|_| "Unknown".into());
            let default_config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(PicoError::Audio(format!(
                        "Failed to get input config: {}",
                        e
                    ))));
                    return;
                }
            };

            let sample_format = default_config.sample_format();
            let sample_rate = default_config.sample_rate().0;
            let channels = default_config.channels();
            let config: cpal::StreamConfig = default_config.into();

            // Capture RT callback writes raw normalized f32 directly into lock-free ring (no alloc/mutex)
            let ring_cap = (sample_rate as usize * channels as usize * 4).max(16384);
            let (producer, consumer) = spsc_ring_buffer::<f32>(ring_cap);
            let is_capturing = Arc::new(AtomicBool::new(true));
            let active_cb = is_capturing.clone();
            let active_worker = is_capturing.clone();

            let worker_stop = Arc::new(AtomicBool::new(false));
            let worker_stop_rx = worker_stop.clone();

            let worker = std::thread::spawn(move || {
                let mut converter = AudioConverter::new(sample_rate, channels);
                // Fixed pre-allocated buffer (20ms at device rate * channels)
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

            let err_fn = |err| {
                error!("Audio capture stream error: {}", err);
            };

            let stream_res = match sample_format {
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
                    worker_stop.store(true, Ordering::Release);
                    let _ = worker.join();
                    let _ = init_tx.send(Err(PicoError::Audio(format!(
                        "Unsupported audio input sample format: {:?}",
                        other
                    ))));
                    return;
                }
            };

            match stream_res {
                Ok(stream) => {
                    if let Err(e) = stream.play() {
                        worker_stop.store(true, Ordering::Release);
                        let _ = worker.join();
                        let _ = init_tx.send(Err(PicoError::Audio(format!(
                            "Failed to play audio input stream: {}",
                            e
                        ))));
                        return;
                    }
                    info!(
                        device = %dev_name,
                        sample_rate,
                        channels,
                        format = ?sample_format,
                        "Hardware audio capture initialized (lock-free SPSC)"
                    );
                    let _ = init_tx.send(Ok(is_capturing));
                    let _ = shutdown_rx.recv();
                    worker_stop.store(true, Ordering::Release);
                    drop(stream);
                    let _ = worker.join();
                }
                Err(e) => {
                    worker_stop.store(true, Ordering::Release);
                    let _ = worker.join();
                    warn!("Audio capture stream could not open: {}", e);
                    let _ = init_tx.send(Err(PicoError::Audio(format!(
                        "Failed to build audio input stream: {}",
                        e
                    ))));
                }
            }
        });

        match init_rx.recv() {
            Ok(Ok(is_capturing)) => Ok(Self {
                is_capturing,
                _shutdown_tx: Some(shutdown_tx),
            }),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(PicoError::Audio(format!(
                "Audio capture thread exited prematurely: {}",
                e
            ))),
        }
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
        self._shutdown_tx.take();
    }
}
