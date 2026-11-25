//! Stateful anti-aliased windowed-sinc resampler, channel mixer, and WAV container decoder.

use pico_core::error::{PicoError, Result};
use std::f64::consts::PI;
use std::io::Cursor;

const LANCZOS_LOBES: isize = 3;

/// Stateful polyphase / windowed-sinc audio resampler preserving continuity across chunks.
#[derive(Debug, Clone)]
pub struct AudioResampler {
    source_rate: u32,
    target_rate: u32,
    ratio: f64,
    phase: f64,
    tail: Vec<f32>,
}

impl AudioResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        let src = source_rate.max(1);
        let dst = target_rate.max(1);
        Self {
            source_rate: src,
            target_rate: dst,
            ratio: src as f64 / dst as f64,
            phase: 0.0,
            tail: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.tail.clear();
    }

    /// Processes a chunk of mono `f32` audio maintaining filter state across chunk boundaries.
    pub fn process_mono_chunk(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if self.source_rate == self.target_rate {
            return input.to_vec();
        }

        let tail_len = self.tail.len();
        let mut combined = Vec::with_capacity(tail_len + input.len());
        combined.extend_from_slice(&self.tail);
        combined.extend_from_slice(input);

        let guard = (LANCZOS_LOBES as f64 * self.ratio.max(1.0)).ceil() as usize + 1;
        if combined.len() <= guard {
            self.tail = combined;
            return Vec::new();
        }

        let cutoff = if self.ratio > 1.0 {
            0.90 / self.ratio
        } else {
            1.0
        };
        let support = (LANCZOS_LOBES as f64) / cutoff;
        let max_pos = (combined.len() - guard) as f64;
        let est_out = ((max_pos - self.phase) / self.ratio).ceil().max(0.0) as usize + 4;
        let mut out = Vec::with_capacity(est_out);

        while self.phase < max_pos {
            let center = self.phase;
            let k_start = (center - support).floor().max(0.0) as isize;
            let k_end = (center + support).ceil().min((combined.len() - 1) as f64) as isize;

            let mut acc = 0.0f64;
            let mut w_sum = 0.0f64;
            for k in k_start..=k_end {
                let d = center - (k as f64);
                let x = d * cutoff;
                let w = lanczos_kernel(x);
                acc += (combined[k as usize] as f64) * w;
                w_sum += w;
            }
            let sample = if w_sum.abs() > 1e-9 {
                (acc / w_sum) as f32
            } else {
                0.0
            };
            out.push(sample.clamp(-1.0, 1.0));
            self.phase += self.ratio;
        }

        // Preserve enough unprocessed history for the next chunk's FIR filter window
        let keep_from = combined.len().saturating_sub(guard * 2);
        self.tail = combined[keep_from..].to_vec();
        self.phase = (self.phase - keep_from as f64).max(0.0);

        out
    }

    /// Resamples a complete finite mono buffer including zero-padded tail drain cleanly.
    pub fn process_mono_complete(source_rate: u32, target_rate: u32, input: &[f32]) -> Vec<f32> {
        if input.is_empty() || source_rate == 0 || target_rate == 0 {
            return Vec::new();
        }
        if source_rate == target_rate {
            return input.to_vec();
        }
        let mut resampler = Self::new(source_rate, target_rate);
        let pad = (LANCZOS_LOBES as f64 * (source_rate as f64 / target_rate as f64).max(1.0)).ceil()
            as usize
            + 2;
        let mut out = resampler.process_mono_chunk(input);
        let zeros = vec![0.0f32; pad * 2];
        out.extend(resampler.process_mono_chunk(&zeros));
        let expected_len =
            ((input.len() as f64) * (target_rate as f64) / (source_rate as f64)).round() as usize;
        out.truncate(expected_len);
        out
    }
}

#[inline]
fn lanczos_kernel(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 1e-8 {
        return 1.0;
    }
    let lobes = LANCZOS_LOBES as f64;
    if ax >= lobes {
        return 0.0;
    }
    let pi_x = PI * ax;
    let sinc = pi_x.sin() / pi_x;
    let win = (pi_x / lobes).sin() / (pi_x / lobes);
    sinc * win
}

/// Converts multi-channel interleaved `f32` audio into normalized mono `f32`.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(ch)
        .map(|frame| frame.iter().copied().sum::<f32>() / ch as f32)
        .collect()
}

/// Resamples and maps channel count from arbitrary source `(sample_rate, channels)` to output format.
pub fn resample_and_map_channels(
    samples: &[f32],
    src_rate: u32,
    src_channels: u16,
    dst_rate: u32,
    dst_channels: u16,
) -> Vec<f32> {
    if samples.is_empty() || src_rate == 0 || dst_rate == 0 {
        return Vec::new();
    }
    let src_ch = src_channels.max(1) as usize;
    let dst_ch = dst_channels.max(1) as usize;

    if src_rate == dst_rate && src_ch == dst_ch {
        return samples.to_vec();
    }

    // Resample each channel independently or downmix cleanly when dst_ch == 1
    if dst_ch == 1 {
        let mono = downmix_to_mono(samples, src_channels);
        return AudioResampler::process_mono_complete(src_rate, dst_rate, &mono);
    }

    if src_ch == 1 {
        let mono_resampled = AudioResampler::process_mono_complete(src_rate, dst_rate, samples);
        let mut interleaved = Vec::with_capacity(mono_resampled.len() * dst_ch);
        for s in mono_resampled {
            for _ in 0..dst_ch {
                interleaved.push(s);
            }
        }
        return interleaved;
    }

    // De-interleave -> resample per channel -> map to dst_ch
    let frame_count = samples.len() / src_ch;
    let mut channel_bufs: Vec<Vec<f32>> = vec![Vec::with_capacity(frame_count); src_ch];
    for frame in samples.chunks_exact(src_ch) {
        for (c, &v) in frame.iter().enumerate() {
            channel_bufs[c].push(v);
        }
    }
    let resampled_channels: Vec<Vec<f32>> = channel_bufs
        .into_iter()
        .map(|ch_data| AudioResampler::process_mono_complete(src_rate, dst_rate, &ch_data))
        .collect();
    let out_frames = resampled_channels.first().map(|v| v.len()).unwrap_or(0);
    let mut out = Vec::with_capacity(out_frames * dst_ch);
    for f in 0..out_frames {
        for c in 0..dst_ch {
            let val = if c < src_ch {
                resampled_channels[c][f]
            } else {
                // Repeat left/right or center safely on >2 channel outputs
                resampled_channels[c % src_ch][f]
            };
            out.push(val);
        }
    }
    out
}

/// Decodes a complete RIFF WAV container explicitly into `(samples_f32, sample_rate, channels)`.
/// Rejects raw unframed PCM cleanly instead of silently interpreting binary payloads wrongly.
pub fn decode_wav_bytes_to_f32(wav_bytes: &[u8]) -> Result<(Vec<f32>, u32, u16)> {
    if wav_bytes.len() < 12 || &wav_bytes[0..4] != b"RIFF" || &wav_bytes[8..12] != b"WAVE" {
        return Err(PicoError::Audio(
            "Buffer is not a valid RIFF/WAVE container".into(),
        ));
    }
    let reader = hound::WavReader::new(Cursor::new(wav_bytes))
        .map_err(|e| PicoError::Audio(format!("Failed parsing WAV header: {}", e)))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .map(|s| s.clamp(-1.0, 1.0))
            .collect(),
        hound::SampleFormat::Int => match spec.bits_per_sample {
            8 => reader
                .into_samples::<i8>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 128.0)
                .collect(),
            16 => reader
                .into_samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| (s as f32 / 32768.0).clamp(-1.0, 1.0))
                .collect(),
            24 => reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| (s as f32 / 8_388_608.0).clamp(-1.0, 1.0))
                .collect(),
            32 => reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| (s as f32 / 2_147_483_648.0).clamp(-1.0, 1.0))
                .collect(),
            other => {
                return Err(PicoError::Audio(format!(
                    "Unsupported WAV PCM bit depth: {}",
                    other
                )));
            }
        },
    };
    Ok((samples, spec.sample_rate, spec.channels))
}

/// Stateful capture converter maintaining continuous resampler history from hardware format to 16kHz mono i16.
pub struct AudioConverter {
    source_rate: u32,
    channels: u16,
    resampler: AudioResampler,
}

impl AudioConverter {
    pub fn new(source_rate: u32, channels: u16) -> Self {
        Self {
            source_rate: source_rate.max(1),
            channels: channels.max(1),
            resampler: AudioResampler::new(source_rate.max(1), 16_000),
        }
    }

    pub fn convert_f32_to_16k_i16(&mut self, input: &[f32]) -> Vec<i16> {
        let mono = downmix_to_mono(input, self.channels);
        let resampled = if self.source_rate == 16_000 {
            mono
        } else {
            self.resampler.process_mono_chunk(&mono)
        };
        resampled
            .into_iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect()
    }

    pub fn convert_i16_to_16k_i16(&mut self, input: &[i16]) -> Vec<i16> {
        let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / 32768.0).collect();
        self.convert_f32_to_16k_i16(&f32_samples)
    }
}
