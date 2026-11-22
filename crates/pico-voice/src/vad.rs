use std::time::{Duration, Instant};
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speaking,
}

pub struct VadProcessor {
    energy_threshold: f32,
    silence_duration: Duration,
    state: VadState,
    speech_start_time: Option<Instant>,
    last_speech_time: Option<Instant>,
    speech_buffer: Vec<i16>,
    noise_floor: f32,
}

impl VadProcessor {
    pub fn new(energy_threshold: f32, silence_duration_ms: u64) -> Self {
        Self {
            energy_threshold,
            silence_duration: Duration::from_millis(silence_duration_ms),
            state: VadState::Silence,
            speech_start_time: None,
            last_speech_time: None,
            speech_buffer: Vec::with_capacity(16000 * 5),
            noise_floor: 0.005,
        }
    }

    /// Computes Root Mean Square (RMS) energy normalized between 0.0 and 1.0.
    pub fn compute_rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let mut sum_sq = 0.0f64;
        for &s in samples {
            let normalized = s as f64 / 32768.0;
            sum_sq += normalized * normalized;
        }
        (sum_sq / samples.len() as f64).sqrt() as f32
    }

    /// Processes an incoming audio chunk. Returns Some(Vec<i16>) when a full speech utterance is finalized.
    pub fn process_chunk(&mut self, chunk: &[i16]) -> Option<Vec<i16>> {
        let rms = Self::compute_rms(chunk);
        let now = Instant::now();

        // Update adaptive noise floor during sustained silence
        if self.state == VadState::Silence {
            self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
        }

        let is_voice = rms > (self.energy_threshold.max(self.noise_floor * 2.5));

        match self.state {
            VadState::Silence => {
                if is_voice {
                    debug!(
                        rms,
                        noise_floor = self.noise_floor,
                        "Voice activity started"
                    );
                    self.state = VadState::Speaking;
                    self.speech_start_time = Some(now);
                    self.last_speech_time = Some(now);
                    self.speech_buffer.clear();
                    self.speech_buffer.extend_from_slice(chunk);
                }
            }
            VadState::Speaking => {
                self.speech_buffer.extend_from_slice(chunk);

                if is_voice {
                    self.last_speech_time = Some(now);
                } else if let Some(last_speech) = self.last_speech_time {
                    if now.duration_since(last_speech) >= self.silence_duration {
                        debug!(
                            duration_ms = self
                                .speech_start_time
                                .map(|t| now.duration_since(t).as_millis())
                                .unwrap_or(0),
                            samples = self.speech_buffer.len(),
                            "Voice activity ended"
                        );
                        self.state = VadState::Silence;
                        self.speech_start_time = None;
                        self.last_speech_time = None;

                        // Only return if speech buffer has significant duration (>= 250ms = 4000 samples at 16kHz)
                        if self.speech_buffer.len() >= 4000 {
                            let result = self.speech_buffer.clone();
                            self.speech_buffer.clear();
                            return Some(result);
                        } else {
                            self.speech_buffer.clear();
                        }
                    }
                }
            }
        }

        None
    }

    pub fn current_state(&self) -> VadState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = VadState::Silence;
        self.speech_buffer.clear();
        self.speech_start_time = None;
        self.last_speech_time = None;
    }
}
