use crate::stt::PlatformSpeechRecognizer;
use crate::tts::{PlatformSpeechSynthesizer, SpeechPreprocessor};
use crate::vad::{VadProcessor, VadState};
use chrono::Utc;
use pico_audio::{AudioCapture, AudioPlayback, PlaybackHandle};
use pico_core::config::VoiceConfig;
use pico_core::error::{PicoError, Result};
use pico_core::events::PicoEvent;
use pico_core::session::SessionId;
use pico_core::traits::{SpeechRecognizer, SpeechSynthesizer, VoiceInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Notify};
use tracing::{debug, info, warn};

/// End-to-End Voice Pipeline coordinating hardware audio capture, sample-counted VAD with pre-roll,
/// multi-engine STT, sentence-segmented TTS with explicit audio format negotiation, and barge-in.
pub struct VoicePipeline {
    config: VoiceConfig,
    _playback_engine: Arc<AudioPlayback>,
    playback_handle: PlaybackHandle,
    synthesizer: Arc<PlatformSpeechSynthesizer>,
    recognizer: Arc<PlatformSpeechRecognizer>,
    event_tx: broadcast::Sender<PicoEvent>,
    speak_cancelled: Arc<AtomicBool>,
    shutdown_flag: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    echo_guard_until_ms: Arc<std::sync::atomic::AtomicU64>,
}

impl VoicePipeline {
    /// Initializes the voice pipeline using the host audio output device, falling back to
    /// headless playback on servers without an audio output endpoint.
    pub fn new(config: VoiceConfig, event_tx: broadcast::Sender<PicoEvent>) -> Result<Self> {
        config.validate()?;
        let playback_engine = match AudioPlayback::start(None) {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                warn!(error = %e, "Hardware audio output unavailable; falling back to headless playback");
                Arc::new(AudioPlayback::headless())
            }
        };
        Ok(Self::from_playback(config, playback_engine, event_tx))
    }

    /// Initializes the voice pipeline with headless audio playback (for CI and unit tests).
    pub fn new_headless(
        config: VoiceConfig,
        event_tx: broadcast::Sender<PicoEvent>,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self::from_playback(
            config,
            Arc::new(AudioPlayback::headless()),
            event_tx,
        ))
    }

    fn from_playback(
        config: VoiceConfig,
        playback_engine: Arc<AudioPlayback>,
        event_tx: broadcast::Sender<PicoEvent>,
    ) -> Self {
        let playback_handle = playback_engine.handle();
        let synthesizer = Arc::new(PlatformSpeechSynthesizer::from_config(&config));
        let recognizer = Arc::new(PlatformSpeechRecognizer::from_config(&config));

        Self {
            config,
            _playback_engine: playback_engine,
            playback_handle,
            synthesizer,
            recognizer,
            event_tx,
            speak_cancelled: Arc::new(AtomicBool::new(false)),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            echo_guard_until_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn now_epoch_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Starts asynchronous microphone capture wired into the sample-counted VAD, barge-in monitor,
    /// and local speech recognition engine, emitting finalized transcribed utterances.
    pub fn start_listening(&self, speech_tx: mpsc::Sender<String>) -> Result<AudioCapture> {
        self.start_listening_in_session(None, speech_tx)
    }

    /// Session-attributed variant of [`Self::start_listening`].
    pub fn start_listening_in_session(
        &self,
        session_id: Option<SessionId>,
        speech_tx: mpsc::Sender<String>,
    ) -> Result<AudioCapture> {
        let (pcm_tx, mut pcm_rx) = mpsc::channel::<Vec<i16>>(16);
        let capture = self.start_listening_pcm_in_session(session_id, pcm_tx)?;
        let recognizer = Arc::clone(&self.recognizer);
        let event_tx = self.event_tx.clone();
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);

        tokio::spawn(async move {
            loop {
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
                let utterance_pcm = tokio::select! {
                    _ = shutdown_notify.notified() => break,
                    maybe_pcm = pcm_rx.recv() => match maybe_pcm {
                        Some(pcm) => pcm,
                        None => break,
                    }
                };
                let t0 = Instant::now();
                match recognizer.transcribe(&utterance_pcm).await {
                    Ok(text) => {
                        let clean = text.trim();
                        if !clean.is_empty() {
                            let _ = event_tx.send(PicoEvent::TranscriptionFinalized {
                                session_id,
                                text: clean.to_string(),
                                latency_ms: t0.elapsed().as_millis() as u64,
                            });
                            if speech_tx.send(clean.to_string()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "ASR transcription failed for captured utterance");
                        let _ = event_tx.send(PicoEvent::Error {
                            session_id,
                            component: "stt".into(),
                            message: e.to_string(),
                        });
                    }
                }
            }
        });

        Ok(capture)
    }

    /// Starts asynchronous microphone capture wired into sample-counted VAD & barge-in monitor,
    /// emitting segmented 16 kHz mono PCM utterances.
    pub fn start_listening_pcm(&self, speech_tx: mpsc::Sender<Vec<i16>>) -> Result<AudioCapture> {
        self.start_listening_pcm_in_session(None, speech_tx)
    }

    /// Session-attributed variant of [`Self::start_listening_pcm`].
    pub fn start_listening_pcm_in_session(
        &self,
        session_id: Option<SessionId>,
        speech_tx: mpsc::Sender<Vec<i16>>,
    ) -> Result<AudioCapture> {
        let (raw_tx, mut raw_rx) = mpsc::channel::<Vec<i16>>(128);
        let capture = AudioCapture::start(None, raw_tx)?;

        let mut vad = VadProcessor::new(self.config.vad_threshold, self.config.silence_duration_ms);
        let enable_barge_in = self.config.enable_barge_in;
        let barge_in_threshold = (self.config.vad_threshold * 3.2).max(0.065);
        let playback_handle = self.playback_handle.clone();
        let speak_cancelled = self.speak_cancelled.clone();
        let echo_guard_until_ms = self.echo_guard_until_ms.clone();
        let event_tx = self.event_tx.clone();
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);

        tokio::spawn(async move {
            let mut speech_started_at: Option<Instant> = None;
            let mut barge_in_frames: usize = 0;

            loop {
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
                let chunk = tokio::select! {
                    _ = shutdown_notify.notified() => break,
                    maybe_chunk = raw_rx.recv() => match maybe_chunk {
                        Some(c) => c,
                        None => break,
                    }
                };
                let rms = VadProcessor::compute_rms(&chunk);
                let playing = playback_handle.is_playing();
                let in_echo_tail =
                    !playing && Self::now_epoch_ms() < echo_guard_until_ms.load(Ordering::Relaxed);

                if playing || in_echo_tail {
                    if enable_barge_in && rms > barge_in_threshold {
                        barge_in_frames += 1;
                        if barge_in_frames >= 2 {
                            info!(
                                rms,
                                "Sustained barge-in detected; interrupting assistant speech"
                            );
                            speak_cancelled.store(true, Ordering::SeqCst);
                            playback_handle.interrupt();
                            let _ = event_tx.send(PicoEvent::PlaybackInterrupted {
                                session_id,
                                reason: "User barge-in".into(),
                            });
                            vad.reset();
                            barge_in_frames = 0;
                        }
                    } else {
                        barge_in_frames = 0;
                        continue;
                    }
                } else {
                    barge_in_frames = 0;
                }

                let prev_state = vad.current_state();
                let completed_utterance = vad.process_chunk(&chunk);
                let new_state = vad.current_state();

                if prev_state == VadState::Silence && new_state == VadState::Speaking {
                    speech_started_at = Some(Instant::now());
                    let _ = event_tx.send(PicoEvent::VoiceActivityStarted {
                        session_id,
                        timestamp: Utc::now(),
                    });
                }

                if let Some(utterance_pcm) = completed_utterance {
                    let duration_ms = speech_started_at
                        .take()
                        .map(|s| s.elapsed().as_millis() as u64)
                        .unwrap_or_else(|| (utterance_pcm.len() as u64 * 1000) / 16_000);
                    let _ = event_tx.send(PicoEvent::VoiceActivityStopped {
                        session_id,
                        timestamp: Utc::now(),
                        duration_ms,
                    });
                    if speech_tx.send(utterance_pcm).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(capture)
    }

    /// Synthesizes and streams spoken text sentence-by-sentence with proper sample-rate conversion
    /// and immediate cancellation support.
    pub async fn speak(&self, text: &str) -> Result<()> {
        self.speak_in_session(None, text).await
    }

    /// Session-attributed variant of [`Self::speak`].
    pub async fn speak_in_session(&self, session_id: Option<SessionId>, text: &str) -> Result<()> {
        let sentences = SpeechPreprocessor::split_sentences(text);
        if sentences.is_empty() {
            return Ok(());
        }

        self.speak_cancelled.store(false, Ordering::SeqCst);
        let t0 = Instant::now();
        let _ = self.event_tx.send(PicoEvent::SpeechSynthesisStarted {
            session_id,
            text_length: text.len(),
        });

        let mut total_audio_ms: u64 = 0;
        let mut playback_started_sent = false;

        for (idx, sentence) in sentences.iter().enumerate() {
            if self.speak_cancelled.load(Ordering::SeqCst)
                || self.shutdown_flag.load(Ordering::Relaxed)
            {
                debug!(
                    "TTS halted before sentence {} due to cancellation/barge-in",
                    idx
                );
                break;
            }

            let audio = match self.synthesizer.synthesize_audio(sentence).await {
                Ok(a) => a,
                Err(e) => {
                    warn!(sentence = %sentence, error = %e, "TTS synthesis failed for sentence");
                    let _ = self.event_tx.send(PicoEvent::Error {
                        session_id,
                        component: "tts".into(),
                        message: e.to_string(),
                    });
                    return Err(e);
                }
            };

            if self.speak_cancelled.load(Ordering::SeqCst)
                || self.shutdown_flag.load(Ordering::Relaxed)
            {
                break;
            }

            let duration_ms = if audio.sample_rate > 0 && audio.channels > 0 {
                (audio.samples.len() as u64 * 1000)
                    / (audio.sample_rate as u64 * audio.channels as u64)
            } else {
                0
            };
            total_audio_ms += duration_ms;

            if !playback_started_sent {
                let _ = self
                    .event_tx
                    .send(PicoEvent::PlaybackStarted { session_id });
                playback_started_sent = true;
            }

            self.playback_handle
                .play_audio(&audio.samples, audio.sample_rate, audio.channels);

            // Insert a gentle 110ms pause between multi-sentence segments at the same source rate.
            if idx + 1 < sentences.len() && audio.sample_rate > 0 {
                let pause_samples =
                    (audio.sample_rate as usize / 9) * audio.channels.max(1) as usize;
                let silence = vec![0.0f32; pause_samples];
                self.playback_handle
                    .play_audio(&silence, audio.sample_rate, audio.channels);
            }
        }

        let _ = self.event_tx.send(PicoEvent::SpeechSynthesisFinished {
            session_id,
            audio_duration_ms: total_audio_ms,
            latency_ms: t0.elapsed().as_millis() as u64,
        });

        // Wait cleanly until audio finishes draining or caller interrupts playback.
        while self.playback_handle.is_playing() {
            if self.speak_cancelled.load(Ordering::SeqCst)
                || self.shutdown_flag.load(Ordering::Relaxed)
            {
                self.playback_handle.interrupt();
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // Guard trailing acoustic echo for 180ms after audio completes.
        self.echo_guard_until_ms
            .store(Self::now_epoch_ms().saturating_add(180), Ordering::Relaxed);

        if playback_started_sent {
            let _ = self
                .event_tx
                .send(PicoEvent::PlaybackFinished { session_id });
        }

        Ok(())
    }

    /// Transcribes 16kHz mono PCM audio using the active speech recognizer.
    pub async fn transcribe(&self, pcm_samples: &[i16]) -> Result<String> {
        self.transcribe_in_session(None, pcm_samples).await
    }

    /// Session-attributed variant of [`Self::transcribe`].
    pub async fn transcribe_in_session(
        &self,
        session_id: Option<SessionId>,
        pcm_samples: &[i16],
    ) -> Result<String> {
        let t0 = Instant::now();
        let text = match self.recognizer.transcribe(pcm_samples).await {
            Ok(t) => t,
            Err(e) => {
                let _ = self.event_tx.send(PicoEvent::Error {
                    session_id,
                    component: "stt".into(),
                    message: e.to_string(),
                });
                return Err(e);
            }
        };
        let latency_ms = t0.elapsed().as_millis() as u64;
        if !text.is_empty() {
            let _ = self.event_tx.send(PicoEvent::TranscriptionFinalized {
                session_id,
                text: text.clone(),
                latency_ms,
            });
        }
        Ok(text)
    }

    /// Immediately halts active audio synthesis & clears pending playback samples in O(1).
    pub fn interrupt_playback(&self) {
        self.interrupt_playback_in_session(None);
    }

    /// Session-attributed variant of [`Self::interrupt_playback`].
    pub fn interrupt_playback_in_session(&self, session_id: Option<SessionId>) {
        self.speak_cancelled.store(true, Ordering::SeqCst);
        self.playback_handle.interrupt();
        let _ = self.event_tx.send(PicoEvent::PlaybackInterrupted {
            session_id,
            reason: "Explicit interruption".into(),
        });
    }

    /// Signals any active voice listener workers to exit and interrupts active playback.
    pub fn shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
        self.interrupt_playback();
    }

    /// Returns whether audio samples remain queued for output.
    pub fn is_playing(&self) -> bool {
        self.playback_handle.is_playing()
    }

    /// Returns discovered voices for the active host platform.
    pub fn available_voices(&self) -> Vec<VoiceInfo> {
        self.synthesizer.list_voices()
    }

    pub fn tts_engine(&self) -> &str {
        self.synthesizer.engine_name()
    }

    pub fn stt_engine(&self) -> &str {
        self.recognizer.stt_engine()
    }

    /// Emits a `PicoEvent::VoiceBackendStatus` summarizing STT and TTS readiness and errors.
    pub fn emit_backend_status(&self) {
        let tts_voices = self.synthesizer.list_voices();
        let tts_ready = self.tts_engine() == "mock" || !tts_voices.is_empty();
        let tts_error = if tts_ready {
            None
        } else {
            Some(format!(
                "TTS engine '{}' has no discovered voices",
                self.tts_engine()
            ))
        };

        let stt_engine = self.stt_engine().to_string();
        let (stt_ready, stt_error) = match self.validate_config() {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        };

        let _ = self.event_tx.send(PicoEvent::VoiceBackendStatus {
            stt_engine,
            stt_ready,
            tts_engine: self.tts_engine().to_string(),
            tts_ready,
            stt_error,
            tts_error,
        });
    }

    /// Checks whether a configuration error prevents audio capture or voice validation.
    pub fn output_sample_rate(&self) -> u32 {
        self.playback_handle.output_sample_rate()
    }

    pub fn validate_config(&self) -> Result<()> {
        self.config
            .validate()
            .map_err(|e| PicoError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_headless_voice_pipeline_mock_speak_and_interrupt() {
        let (tx, _rx) = broadcast::channel(32);
        let cfg = VoiceConfig {
            stt_engine: "mock".into(),
            tts_engine: "mock".into(),
            speech_rate: 1.0,
            ..VoiceConfig::default()
        };
        let pipeline = VoicePipeline::new_headless(cfg, tx).unwrap();
        assert_eq!(pipeline.tts_engine(), "mock");
        assert_eq!(pipeline.stt_engine(), "mock");
        assert!(!pipeline.available_voices().is_empty());

        pipeline
            .speak("Hello from headless pipeline. Check CPU at 42%.")
            .await
            .unwrap();
        assert!(!pipeline.is_playing());

        pipeline.interrupt_playback();
        assert!(!pipeline.is_playing());
    }

    #[tokio::test]
    async fn test_concurrent_session_voice_event_attribution() {
        let (tx, mut rx) = broadcast::channel(64);
        let cfg = VoiceConfig {
            stt_engine: "mock".into(),
            tts_engine: "mock".into(),
            speech_rate: 1.0,
            ..VoiceConfig::default()
        };
        let pipeline = Arc::new(VoicePipeline::new_headless(cfg, tx).unwrap());
        let sid_a = SessionId::new();
        let sid_b = SessionId::new();

        let p1 = Arc::clone(&pipeline);
        let p2 = Arc::clone(&pipeline);
        let (r1, r2) = tokio::join!(
            p1.speak_in_session(Some(sid_a), "Session A speaking."),
            p2.speak_in_session(Some(sid_b), "Session B speaking."),
        );
        r1.unwrap();
        r2.unwrap();
        pipeline.interrupt_playback_in_session(Some(sid_a));

        let mut saw_a_synth = false;
        let mut saw_b_synth = false;
        let mut saw_a_interrupt = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                PicoEvent::SpeechSynthesisStarted { session_id, .. }
                | PicoEvent::SpeechSynthesisFinished { session_id, .. }
                | PicoEvent::PlaybackStarted { session_id }
                | PicoEvent::PlaybackFinished { session_id } => {
                    assert!(
                        session_id == Some(sid_a) || session_id == Some(sid_b),
                        "Unexpected session attribution: {:?}",
                        session_id
                    );
                    if session_id == Some(sid_a) {
                        saw_a_synth = true;
                    }
                    if session_id == Some(sid_b) {
                        saw_b_synth = true;
                    }
                }
                PicoEvent::PlaybackInterrupted { session_id, .. } => {
                    assert_eq!(session_id, Some(sid_a));
                    saw_a_interrupt = true;
                }
                _ => {}
            }
        }
        assert!(saw_a_synth && saw_b_synth && saw_a_interrupt);
    }
}
