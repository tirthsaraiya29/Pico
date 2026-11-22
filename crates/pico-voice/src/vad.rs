use std::time::{Duration, Instant};
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speaking,
}

/// Advanced Voice Activity Detector and Speech Turn Segmenter.
/// Distinguishes between brief mid-sentence thinking/stutter pauses and true end-of-speech.
pub struct VadProcessor {
    energy_threshold: f32,
    min_silence_duration: Duration,
    max_silence_duration: Duration,
    state: VadState,
    speech_start_time: Option<Instant>,
    last_speech_time: Option<Instant>,
    speech_buffer: Vec<i16>,
    noise_floor: f32,
    consecutive_voice_frames: usize,
}

impl VadProcessor {
    pub fn new(energy_threshold: f32, silence_duration_ms: u64) -> Self {
        let base_silence = Duration::from_millis(silence_duration_ms.max(400));
        Self {
            energy_threshold,
            min_silence_duration: base_silence,
            max_silence_duration: base_silence + Duration::from_millis(400),
            state: VadState::Silence,
            speech_start_time: None,
            last_speech_time: None,
            speech_buffer: Vec::with_capacity(16000 * 10), // Up to 10 seconds capacity
            noise_floor: 0.005,
            consecutive_voice_frames: 0,
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

    /// Processes an incoming audio chunk (16kHz mono).
    /// Returns Some(Vec<i16>) when an utterance is conclusively finalized.
    pub fn process_chunk(&mut self, chunk: &[i16]) -> Option<Vec<i16>> {
        let rms = Self::compute_rms(chunk);
        let now = Instant::now();

        // Dynamically track ambient background noise floor during silence
        if self.state == VadState::Silence {
            self.noise_floor = self.noise_floor * 0.96 + rms * 0.04;
        }

        let dynamic_threshold = self.energy_threshold.max(self.noise_floor * 2.8);
        let is_voice = rms > dynamic_threshold;

        if is_voice {
            self.consecutive_voice_frames += 1;
        } else {
            self.consecutive_voice_frames = 0;
        }

        match self.state {
            VadState::Silence => {
                // Require at least 2 consecutive voice frames (approx 60-100ms) to reject pops/clicks
                if self.consecutive_voice_frames >= 2 {
                    debug!(
                        rms,
                        noise_floor = self.noise_floor,
                        "Voice activity detected: starting utterance buffer"
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
                } else if let (Some(start_time), Some(last_speech)) =
                    (self.speech_start_time, self.last_speech_time)
                {
                    let elapsed_speech = now.duration_since(start_time);
                    let silence_elapsed = now.duration_since(last_speech);

                    // Adaptive silence timeout:
                    // If the utterance has just started (< 1.5s), allow a longer thinking/hesitation pause (up to 1000ms).
                    // If the utterance is already substantial, a shorter pause (min_silence_duration) triggers completion.
                    let adaptive_silence_threshold = if elapsed_speech < Duration::from_millis(1500)
                    {
                        self.max_silence_duration
                    } else {
                        self.min_silence_duration
                    };

                    // Maximum turn duration safeguard (e.g. 25 seconds) to avoid indefinite holding
                    let max_turn_exceeded = elapsed_speech >= Duration::from_secs(25);

                    if silence_elapsed >= adaptive_silence_threshold || max_turn_exceeded {
                        debug!(
                            duration_ms = elapsed_speech.as_millis(),
                            silence_ms = silence_elapsed.as_millis(),
                            samples = self.speech_buffer.len(),
                            "Conclusive end-of-speech detected"
                        );
                        self.state = VadState::Silence;
                        self.speech_start_time = None;
                        self.last_speech_time = None;
                        self.consecutive_voice_frames = 0;

                        // Only emit if total speech duration is >= 250ms (4000 samples at 16kHz)
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
        self.consecutive_voice_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_computation() {
        let silence = vec![0i16; 1600];
        assert_eq!(VadProcessor::compute_rms(&silence), 0.0);

        let loud = vec![16384i16; 1600];
        let rms = VadProcessor::compute_rms(&loud);
        assert!((rms - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_vad_speech_and_silence_transition() {
        let mut vad = VadProcessor::new(0.02, 300);
        assert_eq!(vad.current_state(), VadState::Silence);

        // Feed silence chunks
        let silence = vec![0i16; 800];
        assert!(vad.process_chunk(&silence).is_none());

        // Feed speech chunks (above threshold)
        let speech = vec![10000i16; 800];
        vad.process_chunk(&speech);
        vad.process_chunk(&speech);
        assert_eq!(vad.current_state(), VadState::Speaking);
    }
}
