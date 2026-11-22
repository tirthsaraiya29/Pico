use crate::stt::PlatformSpeechRecognizer;
use crate::tts::PlatformSpeechSynthesizer;
use crate::vad::{VadProcessor, VadState};
use pico_audio::{AudioCapture, AudioPlayback, PlaybackHandle};
use pico_core::config::VoiceConfig;
use pico_core::error::Result;
use pico_core::events::PicoEvent;
use pico_core::traits::{SpeechRecognizer, SpeechSynthesizer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

pub struct VoicePipeline {
    config: VoiceConfig,
    _playback_engine: Arc<AudioPlayback>,
    playback: PlaybackHandle,
    recognizer: Arc<dyn SpeechRecognizer>,
    synthesizer: Arc<dyn SpeechSynthesizer>,
    event_sender: broadcast::Sender<PicoEvent>,
    is_running: Arc<AtomicBool>,
    is_speaking: Arc<AtomicBool>,
}

impl VoicePipeline {
    pub fn new(config: VoiceConfig, event_sender: broadcast::Sender<PicoEvent>) -> Result<Self> {
        let playback_engine = Arc::new(AudioPlayback::start(None)?);
        let playback = playback_engine.handle();
        let recognizer = Arc::new(PlatformSpeechRecognizer::new(None));
        let synthesizer = Arc::new(PlatformSpeechSynthesizer::new(
            config.voice_name.clone(),
            config.speech_rate,
        ));

        Ok(Self {
            config,
            _playback_engine: playback_engine,
            playback,
            recognizer,
            synthesizer,
            event_sender,
            is_running: Arc::new(AtomicBool::new(false)),
            is_speaking: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Starts the continuous audio loop. Incoming transcribed utterances are forwarded to `speech_tx`.
    pub fn start_listening(&self, speech_tx: mpsc::Sender<String>) -> Result<AudioCapture> {
        let (audio_chunk_tx, mut audio_chunk_rx) = mpsc::channel::<Vec<i16>>(64);
        let capture = AudioCapture::start(None, audio_chunk_tx)?;

        let mut vad = VadProcessor::new(self.config.vad_threshold, self.config.silence_duration_ms);
        let recognizer = self.recognizer.clone();
        let playback = self.playback.clone();
        let events = self.event_sender.clone();
        let barge_in_enabled = self.config.enable_barge_in;
        let is_running = self.is_running.clone();
        let is_speaking = self.is_speaking.clone();
        is_running.store(true, Ordering::Relaxed);

        tokio::spawn(async move {
            info!("Voice pipeline event loop initialized");
            while is_running.load(Ordering::Relaxed) {
                if let Some(chunk) = audio_chunk_rx.recv().await {
                    let prev_state = vad.current_state();

                    // Check for speech completion
                    if let Some(speech_samples) = vad.process_chunk(&chunk) {
                        is_speaking.store(false, Ordering::Relaxed);
                        let _ = events.send(PicoEvent::VoiceActivityStopped {
                            timestamp: chrono::Utc::now(),
                            duration_ms: (speech_samples.len() as u64 * 1000) / 16000,
                        });

                        info!(
                            samples = speech_samples.len(),
                            "Transcribing user speech utterance"
                        );
                        let t0 = Instant::now();
                        match recognizer.transcribe(&speech_samples).await {
                            Ok(text) => {
                                let latency = t0.elapsed().as_millis() as u64;
                                let trimmed = text.trim().to_string();
                                if !trimmed.is_empty() {
                                    info!(text = %trimmed, latency_ms = latency, "Speech recognized");
                                    let _ = events.send(PicoEvent::TranscriptionFinalized {
                                        text: trimmed.clone(),
                                        latency_ms: latency,
                                    });
                                    let _ = speech_tx.send(trimmed).await;
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Speech transcription failed");
                            }
                        }
                    } else if vad.current_state() == VadState::Speaking
                        && prev_state == VadState::Silence
                    {
                        is_speaking.store(true, Ordering::Relaxed);
                        let _ = events.send(PicoEvent::VoiceActivityStarted {
                            timestamp: chrono::Utc::now(),
                        });

                        // Barge-in: if assistant is currently speaking, halt playback immediately!
                        if barge_in_enabled && playback.is_playing() {
                            info!("Barge-in detected: interrupting ongoing speech playback");
                            playback.interrupt();
                            let _ = events.send(PicoEvent::PlaybackInterrupted {
                                reason: "User spoke during playback (barge-in)".into(),
                            });
                        }
                    }
                }
            }
        });

        Ok(capture)
    }

    /// Synthesizes text and streams output into the audio playback buffer.
    pub async fn speak(&self, text: &str) -> Result<()> {
        let _ = self.event_sender.send(PicoEvent::SpeechSynthesisStarted {
            text_length: text.len(),
        });

        let t0 = Instant::now();
        let pcm_bytes = self.synthesizer.synthesize(text).await?;
        let synth_ms = t0.elapsed().as_millis() as u64;

        let duration_ms = (pcm_bytes.len() as u64 * 1000) / (16000 * 2);
        let _ = self.event_sender.send(PicoEvent::SpeechSynthesisFinished {
            audio_duration_ms: duration_ms,
            latency_ms: synth_ms,
        });

        let _ = self.event_sender.send(PicoEvent::PlaybackStarted);
        self.playback.play_pcm_bytes(&pcm_bytes);
        Ok(())
    }

    /// Instantly interrupts ongoing speech playback.
    pub fn interrupt_playback(&self) {
        self.playback.interrupt();
        let _ = self.event_sender.send(PicoEvent::PlaybackInterrupted {
            reason: "Manual interruption".into(),
        });
    }

    pub fn is_playing(&self) -> bool {
        self.playback.is_playing()
    }
}
