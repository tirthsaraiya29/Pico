/// High-performance audio resampler and channel converter to standard 16kHz mono PCM.
pub struct AudioConverter {
    source_rate: u32,
    target_rate: u32,
    channels: u16,
}

impl AudioConverter {
    pub fn new(source_rate: u32, channels: u16) -> Self {
        Self {
            source_rate,
            target_rate: 16000,
            channels: channels.max(1),
        }
    }

    /// Converts interleaved multi-channel f32 samples to 16kHz mono i16 PCM.
    pub fn convert_f32_to_16k_i16(&self, input: &[f32]) -> Vec<i16> {
        if input.is_empty() {
            return Vec::new();
        }

        // 1. Downmix channels to mono
        let ch = self.channels as usize;
        let mono_len = input.len() / ch;
        let mut mono = Vec::with_capacity(mono_len);

        for i in 0..mono_len {
            let mut sum = 0.0f32;
            for c in 0..ch {
                sum += input[i * ch + c];
            }
            mono.push(sum / ch as f32);
        }

        // 2. Resample to 16,000 Hz
        if self.source_rate == self.target_rate {
            return mono
                .into_iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
        }

        let ratio = self.source_rate as f64 / self.target_rate as f64;
        let output_len = (mono.len() as f64 / ratio).floor() as usize;
        let mut resampled = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_idx = i as f64 * ratio;
            let idx_floor = src_idx.floor() as usize;
            let idx_ceil = (idx_floor + 1).min(mono.len() - 1);
            let frac = (src_idx - idx_floor as f64) as f32;

            let sample = if idx_floor < mono.len() {
                mono[idx_floor] * (1.0 - frac) + mono[idx_ceil] * frac
            } else {
                0.0
            };

            let clamped = sample.clamp(-1.0, 1.0);
            resampled.push((clamped * 32767.0) as i16);
        }

        resampled
    }
}
