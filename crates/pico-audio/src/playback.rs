use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use pico_core::error::{PicoError, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tracing::{error, info};

/// Thread-safe handle for enqueueing and interrupting audio playback.
#[derive(Clone)]
pub struct PlaybackHandle {
    buffer: Arc<Mutex<VecDeque<f32>>>,
    is_playing: Arc<AtomicBool>,
}

impl PlaybackHandle {
    pub fn play_pcm_samples(&self, samples: &[i16]) {
        let mut q = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        for &s in samples {
            q.push_back(s as f32 / 32768.0);
        }
        self.is_playing.store(true, Ordering::Relaxed);
    }

    pub fn play_pcm_bytes(&self, bytes: &[u8]) {
        let mut q = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        for chunk in bytes.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(*chunk);
            q.push_back(sample as f32 / 32768.0);
        }
        self.is_playing.store(true, Ordering::Relaxed);
    }

    pub fn interrupt(&self) {
        let mut q = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        q.clear();
        self.is_playing.store(false, Ordering::Relaxed);
        info!("Audio playback interrupted");
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }
}

/// Safe audio playback runner whose OS audio stream is isolated on a dedicated thread.
pub struct AudioPlayback {
    handle: PlaybackHandle,
    _shutdown_tx: Option<mpsc::Sender<()>>,
}

// Guarantee Send and Sync since AudioPlayback holds only a thread-safe handle and channel sender
unsafe impl Send for AudioPlayback {}
unsafe impl Sync for AudioPlayback {}

impl AudioPlayback {
    pub fn start(device_name: Option<&str>) -> Result<Self> {
        let dev_name_opt = device_name.map(|s| s.to_string());
        let (init_tx, init_rx) = mpsc::channel::<Result<PlaybackHandle>>();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device_res = match dev_name_opt {
                Some(name) => {
                    let mut found = None;
                    if let Ok(devices) = host.output_devices() {
                        for d in devices {
                            if d.name().map(|n| n == name).unwrap_or(false) {
                                found = Some(d);
                                break;
                            }
                        }
                    }
                    found.ok_or_else(|| {
                        PicoError::Audio(format!("Output device '{}' not found", name))
                    })
                }
                None => host.default_output_device().ok_or_else(|| {
                    PicoError::Audio("No default audio output device found".to_string())
                }),
            };

            let device = match device_res {
                Ok(d) => d,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            let default_config = match device.default_output_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ =
                        init_tx.send(Err(PicoError::Audio(format!("Failed query config: {}", e))));
                    return;
                }
            };

            let channels = default_config.channels() as usize;
            let sample_format = default_config.sample_format();

            let buffer = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(32768)));
            let is_playing = Arc::new(AtomicBool::new(false));

            let buf_cb = buffer.clone();
            let playing_flag = is_playing.clone();

            let err_fn = |err| {
                error!(error = %err, "Audio playback stream error");
            };

            let stream_res: Result<Stream> = match sample_format {
                SampleFormat::F32 => device
                    .build_output_stream(
                        &default_config.into(),
                        move |data: &mut [f32], _| {
                            let mut q = buf_cb.lock().unwrap_or_else(|p| p.into_inner());
                            for frame in data.chunks_mut(channels) {
                                let sample = q.pop_front().unwrap_or(0.0);
                                for ch in frame.iter_mut() {
                                    *ch = sample;
                                }
                            }
                            playing_flag.store(!q.is_empty(), Ordering::Relaxed);
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| PicoError::Audio(format!("Build stream failed: {}", e))),
                SampleFormat::I16 => device
                    .build_output_stream(
                        &default_config.into(),
                        move |data: &mut [i16], _| {
                            let mut q = buf_cb.lock().unwrap_or_else(|p| p.into_inner());
                            for frame in data.chunks_mut(channels) {
                                let sample = q.pop_front().unwrap_or(0.0);
                                let i16_sample = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                                for ch in frame.iter_mut() {
                                    *ch = i16_sample;
                                }
                            }
                            playing_flag.store(!q.is_empty(), Ordering::Relaxed);
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| PicoError::Audio(format!("Build stream failed: {}", e))),
                other => Err(PicoError::Audio(format!("Unsupported format: {:?}", other))),
            };

            let stream = match stream_res {
                Ok(s) => s,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            if let Err(e) = stream.play() {
                let _ = init_tx.send(Err(PicoError::Audio(format!("Failed play stream: {}", e))));
                return;
            }

            let handle = PlaybackHandle { buffer, is_playing };
            let _ = init_tx.send(Ok(handle));

            // Keep stream alive until AudioPlayback drops
            let _ = shutdown_rx.recv();
        });

        let handle = init_rx.recv().map_err(|_| {
            PicoError::Audio("Playback worker thread terminated unexpectedly".into())
        })??;

        Ok(Self {
            handle,
            _shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn handle(&self) -> PlaybackHandle {
        self.handle.clone()
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
        if let Some(tx) = self._shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}
