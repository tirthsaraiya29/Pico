use crate::resampler::AudioConverter;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use pico_core::error::{PicoError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use tokio::sync::mpsc as tokio_mpsc;
use tracing::error;

pub struct AudioCapture {
    is_capturing: Arc<AtomicBool>,
    _shutdown_tx: Option<mpsc::Sender<()>>,
}

// Guarantee Send and Sync since AudioCapture holds only atomic flag and channel sender
unsafe impl Send for AudioCapture {}
unsafe impl Sync for AudioCapture {}

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
            let device_res = match dev_name_opt {
                Some(name) => {
                    let mut found = None;
                    if let Ok(devices) = host.input_devices() {
                        for d in devices {
                            if d.name().map(|n| n == name).unwrap_or(false) {
                                found = Some(d);
                                break;
                            }
                        }
                    }
                    found.ok_or_else(|| {
                        PicoError::Audio(format!("Input device '{}' not found", name))
                    })
                }
                None => host.default_input_device().ok_or_else(|| {
                    PicoError::Audio("No default audio input device found".to_string())
                }),
            };

            let device = match device_res {
                Ok(d) => d,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            let default_config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ =
                        init_tx.send(Err(PicoError::Audio(format!("Failed input config: {}", e))));
                    return;
                }
            };

            let sample_rate = default_config.sample_rate().0;
            let channels = default_config.channels();
            let sample_format = default_config.sample_format();

            let converter = Arc::new(AudioConverter::new(sample_rate, channels));
            let is_capturing = Arc::new(AtomicBool::new(true));
            let flag = is_capturing.clone();

            let err_fn = |err| {
                error!(error = %err, "Audio capture stream error");
            };

            let stream_res: Result<Stream> = match sample_format {
                SampleFormat::F32 => {
                    let conv = converter.clone();
                    let sender = sample_sender.clone();
                    let active = flag.clone();
                    device
                        .build_input_stream(
                            &default_config.into(),
                            move |data: &[f32], _| {
                                if active.load(Ordering::Relaxed) {
                                    let samples = conv.convert_f32_to_16k_i16(data);
                                    if !samples.is_empty() {
                                        let _ = sender.try_send(samples);
                                    }
                                }
                            },
                            err_fn,
                            None,
                        )
                        .map_err(|e| PicoError::Audio(format!("Build input stream failed: {}", e)))
                }
                SampleFormat::I16 => {
                    let conv = converter.clone();
                    let sender = sample_sender.clone();
                    let active = flag.clone();
                    device
                        .build_input_stream(
                            &default_config.into(),
                            move |data: &[i16], _| {
                                if active.load(Ordering::Relaxed) {
                                    let f32_data: Vec<f32> =
                                        data.iter().map(|&s| s as f32 / 32768.0).collect();
                                    let samples = conv.convert_f32_to_16k_i16(&f32_data);
                                    if !samples.is_empty() {
                                        let _ = sender.try_send(samples);
                                    }
                                }
                            },
                            err_fn,
                            None,
                        )
                        .map_err(|e| PicoError::Audio(format!("Build input stream failed: {}", e)))
                }
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
                let _ = init_tx.send(Err(PicoError::Audio(format!("Failed start stream: {}", e))));
                return;
            }

            let _ = init_tx.send(Ok(is_capturing));

            // Keep stream alive until AudioCapture drops
            let _ = shutdown_rx.recv();
        });

        let is_capturing = init_rx.recv().map_err(|_| {
            PicoError::Audio("Capture worker thread terminated unexpectedly".into())
        })??;

        Ok(Self {
            is_capturing,
            _shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn stop(&self) {
        self.is_capturing.store(false, Ordering::Relaxed);
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(tx) = self._shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}
