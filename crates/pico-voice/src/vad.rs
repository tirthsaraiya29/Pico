use std::collections::VecDeque;
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speaking,
}

/// Deterministic Voice Activity Detector and Speech Turn Segmenter.
///
/// Uses exact sample counts (rather than wall-clock `Instant::now()`) so speech pauses
/// behave identically across real-time microphone capture, batch WAV files, and unit tests,
/// while preserving a 160ms circular pre-roll buffer so initial consonants are never clipped.
pub struct VadProcessor {
    sample_rate: u32,
    energy_threshold: f32,
    min_silence_samples: usize,
    max_silence_samples: usize,
    max_turn_samples: usize,
    min_utterance_samples: usize,
    pre_roll_capacity: usize,
    pre_roll: VecDeque<i16>,
    state: VadState,
    utterance_samples: usize,
    silence_tail_samples: usize,
    speech_buffer: Vec<i16>,
    noise_floor: f32,
    consecutive_voice_frames: usize,
    total_voiced_frames: usize,
}

impl VadProcessor {
    /// Creates a VAD calibrated for 16 kHz mono PCM frames.
    pub fn new(energy_threshold: f32, silence_duration_ms: u64) -> Self {
        Self::new_with_rate(16_000, energy_threshold, silence_duration_ms)
    }

    /// Creates a VAD calibrated for an explicit mono sample rate (Hz).
    pub fn new_with_rate(sample_rate: u32, energy_threshold: f32, silence_duration_ms: u64) -> Self {
        let rate = sample_rate.max(8_000);
        let min_silence_ms = silence_duration_ms.max(400);
        let max_silence_ms = min_silence_ms + 400;
        let ms_to_samples = |ms: u64| ((ms * rate as u64) / 1000) as usize;

        let pre_roll_capacity = ms_to_samples(160);
        Self {
            sample_rate: rate,
            energy_threshold: energy_threshold.clamp(0.001, 0.9),
            min_silence_samples: ms_to_samples(min_silence_ms),
            max_silence_samples: ms_to_samples(max_silence_ms),
            max_turn_samples: rate as usize * 25,
            min_utterance_samples: ms_to_samples(350),
            pre_roll_capacity,
            pre_roll: VecDeque::with_capacity(pre_roll_capacity + 1024),
            state: VadState::Silence,
            utterance_samples: 0,
            silence_tail_samples: 0,
            speech_buffer: Vec::with_capacity(rate as usize * 10),
            noise_floor: 0.005,
            consecutive_voice_frames: 0,
            total_voiced_frames: 0,
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

    /// Appends samples to the rolling pre-roll buffer while preserving `pre_roll_capacity`.
    fn push_pre_roll(&mut self, chunk: &[i16]) {
        if self.pre_roll_capacity == 0 || chunk.is_empty() {
            return;
        }
        if chunk.len() >= self.pre_roll_capacity {
            self.pre_roll.clear();
            let tail = &chunk[chunk.len() - self.pre_roll_capacity..];
            self.pre_roll.extend(tail.iter().copied());
            return;
        }
        let excess = (self.pre_roll.len() + chunk.len()).saturating_sub(self.pre_roll_capacity);
        for _ in 0..excess {
            self.pre_roll.pop_front();
        }
        self.pre_roll.extend(chunk.iter().copied());
    }

    /// Processes an incoming audio chunk at the configured mono sample rate.
    /// Returns `Some(Vec<i16>)` when an utterance is conclusively finalized.
    pub fn process_chunk(&mut self, chunk: &[i16]) -> Option<Vec<i16>> {
        if chunk.is_empty() {
            return None;
        }

        let rms = Self::compute_rms(chunk);
        if self.state == VadState::Silence {
            self.noise_floor = (self.noise_floor * 0.96 + rms * 0.04).clamp(0.002, 0.25);
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
                if self.consecutive_voice_frames >= 2 {
                    debug!(
                        rms,
                        noise_floor = self.noise_floor,
                        pre_roll_samples = self.pre_roll.len(),
                        "Voice activity detected: starting utterance buffer with pre-roll"
                    );
                    self.state = VadState::Speaking;
                    self.speech_buffer.clear();
                    self.speech_buffer.extend(self.pre_roll.iter().copied());
                    self.pre_roll.clear();
                    self.speech_buffer.extend_from_slice(chunk);
                    self.utterance_samples = self.speech_buffer.len();
                    self.silence_tail_samples = 0;
                    self.total_voiced_frames = self.consecutive_voice_frames;
                } else {
                    self.push_pre_roll(chunk);
                }
            }
            VadState::Speaking => {
                self.speech_buffer.extend_from_slice(chunk);
                self.utterance_samples += chunk.len();

                if is_voice {
                    self.silence_tail_samples = 0;
                    self.total_voiced_frames += 1;
                } else {
                    self.silence_tail_samples += chunk.len();

                    // Allow longer thinking pauses during the opening 1.5s of speech.
                    let early_speech_cutoff = (self.sample_rate as usize * 3) / 2;
                    let silence_limit = if self.utterance_samples < early_speech_cutoff {
                        self.max_silence_samples
                    } else {
                        self.min_silence_samples
                    };

                    if self.silence_tail_samples >= silence_limit
                        || self.utterance_samples >= self.max_turn_samples
                    {
                        return self.finalize_utterance();
                    }
                }
            }
        }

        None
    }

    /// Manually finalizes and returns any buffered push-to-talk speech if non-trivial.
    pub fn flush_utterance(&mut self) -> Option<Vec<i16>> {
        if self.state != VadState::Speaking {
            return None;
        }
        self.finalize_utterance()
    }

    fn finalize_utterance(&mut self) -> Option<Vec<i16>> {
        let elapsed_ms = (self.utterance_samples as u64 * 1000) / self.sample_rate as u64;
        let silence_ms = (self.silence_tail_samples as u64 * 1000) / self.sample_rate as u64;
        debug!(
            duration_ms = elapsed_ms,
            silence_ms,
            samples = self.speech_buffer.len(),
            total_voiced_frames = self.total_voiced_frames,
            "Conclusive end-of-speech detected"
        );

        let valid = self.speech_buffer.len() >= self.min_utterance_samples
            && self.total_voiced_frames >= 4;
        let output = if valid {
            Some(std::mem::take(&mut self.speech_buffer))
        } else {
            self.speech_buffer.clear();
            None
        };

        self.reset();
        output
    }

    pub fn current_state(&self) -> VadState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = VadState::Silence;
        self.speech_buffer.clear();
        self.pre_roll.clear();
        self.utterance_samples = 0;
        self.silence_tail_samples = 0;
        self.consecutive_voice_frames = 0;
        self.total_voiced_frames = 0;
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
        let mut vad = VadProcessor::new(0.02, 400);
        assert_eq!(vad.current_state(), VadState::Silence);

        let silence = vec![0i16; 800];
        assert!(vad.process_chunk(&silence).is_none());

        let speech = vec![10000i16; 800];
        vad.process_chunk(&speech);
        vad.process_chunk(&speech);
        assert_eq!(vad.current_state(), VadState::Speaking);
    }

    #[test]
    fn test_vad_preserves_pre_roll_and_finalizes_deterministically() {
        let mut vad = VadProcessor::new(0.02, 400);
        // Feed 150ms of faint speech onset (inside pre-roll, below 2 consecutive voice frames)
        let onset = vec![400i16; 800];
        assert!(vad.process_chunk(&onset).is_none());

        // Now trigger voice activity with 5 voiced frames (250ms @ 50ms/chunk)
        let voiced = vec![9000i16; 800];
        for _ in 0..5 {
            assert!(vad.process_chunk(&voiced).is_none());
        }
        assert_eq!(vad.current_state(), VadState::Speaking);

        // Feed trailing silence deterministically without sleeping (needs max_silence_ms = 800ms since < 1.5s)
        let silence = vec![0i16; 1600]; // 100ms chunks
        let mut finalized = None;
        for _ in 0..10 {
            if let Some(utt) = vad.process_chunk(&silence) {
                finalized = Some(utt);
                break;
            }
        }

        let audio = finalized.expect("VAD must finalize utterance via sample count clock");
        assert!(audio.contains(&400), "Pre-roll onset audio should be retained");
        assert_eq!(vad.current_state(), VadState::Silence);
    }

    #[test]
    fn test_vad_rejects_single_click_spike() {
        let mut vad = VadProcessor::new(0.02, 400);
        let click = vec![15000i16; 512];
        let silence = vec![0i16; 512];
        assert!(vad.process_chunk(&click).is_none());
        assert!(vad.process_chunk(&silence).is_none());
        assert_eq!(vad.current_state(), VadState::Silence);
    }

    #[test]
    fn test_vad_flush_utterance() {
        let mut vad = VadProcessor::new(0.02, 400);
        let speech = vec![9000i16; 1600]; // 100ms chunks
        for _ in 0..4 {
            assert!(vad.process_chunk(&speech).is_none());
        }
        let flushed = vad.flush_utterance().expect("Flush should emit >= 350ms speech");
        assert!(flushed.len() >= 6400);
        assert_eq!(vad.current_state(), VadState::Silence);
    }
}
