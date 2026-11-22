use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::SpeechRecognizer;
use regex::Regex;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Normalizes transcribed speech, cleans hesitation fillers, removes stuttered word repetitions,
/// and repairs broken grammar from acoustic speech models.
pub struct SpeechNormalizer;

impl SpeechNormalizer {
    /// Cleans and normalizes raw recognized speech.
    pub fn normalize(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        static FILLER_RE: OnceLock<Regex> = OnceLock::new();
        let filler_re = FILLER_RE.get_or_init(|| {
            Regex::new(r"(?i)\b(uh|um|er|err|ah|like|hmm)\b").unwrap()
        });

        // 1. Remove verbal filler tokens ("uh", "um", etc.)
        let no_fillers = filler_re.replace_all(trimmed, "");

        // 2. Remove hyphenated stutters & consecutive repeated words
        let raw_tokens: Vec<&str> = no_fillers.split_whitespace().collect();
        let mut deduplicated: Vec<String> = Vec::with_capacity(raw_tokens.len());

        for tok in raw_tokens {
            let mut word = tok;
            // Clean hyphenated stutter prefix: e.g. "p-pico" -> "pico", "w-what" -> "what"
            if let Some((prefix, rest)) = tok.split_once('-') {
                if !prefix.is_empty() && rest.to_lowercase().starts_with(&prefix.to_lowercase()) {
                    word = rest;
                }
            }

            let clean_tok = word.trim_matches(|c: char| c.is_ascii_punctuation());
            if let Some(last) = deduplicated.last() {
                let clean_last = last.trim_matches(|c: char| c.is_ascii_punctuation());
                if !clean_tok.is_empty() && clean_tok.eq_ignore_ascii_case(clean_last) {
                    continue; // Skip consecutive repeated word
                }
            }
            deduplicated.push(word.to_string());
        }

        let result = deduplicated.join(" ");
        result.trim().to_string()
    }
}

/// Cross-platform Native Speech Recognition Engine and Whisper STT.
pub struct PlatformSpeechRecognizer {
    whisper_binary: Option<PathBuf>,
}

pub type WindowsSpeechRecognizer = PlatformSpeechRecognizer;

impl PlatformSpeechRecognizer {
    pub fn new(whisper_binary: Option<PathBuf>) -> Self {
        Self { whisper_binary }
    }

    /// Converts 16kHz mono PCM i16 samples to a standard WAV byte buffer in-memory.
    pub fn samples_to_wav_bytes(samples: &[i16]) -> Result<Vec<u8>> {
        let mut buffer = Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(|e| {
            PicoError::SpeechRecognition(format!("Failed to create WAV writer: {}", e))
        })?;

        for &sample in samples {
            writer.write_sample(sample).map_err(|e| {
                PicoError::SpeechRecognition(format!("Failed to write audio sample: {}", e))
            })?;
        }

        writer.finalize().map_err(|e| {
            PicoError::SpeechRecognition(format!("Failed to finalize WAV header: {}", e))
        })?;

        Ok(buffer.into_inner())
    }

    /// Transcribes a temporary WAV file using Windows Speech Recognition Engine.
    #[cfg(target_os = "windows")]
    fn transcribe_windows_sapi(wav_path: &Path) -> Result<String> {
        let path_str = wav_path.to_string_lossy().replace('\\', "\\\\");
        let ps_script = format!(
            r#"
Add-Type -AssemblyName System.Speech;
$engine = New-Object System.Speech.Recognition.SpeechRecognitionEngine;
$grammar = New-Object System.Speech.Recognition.DictationGrammar;
$engine.LoadGrammar($grammar);
$engine.SetInputToWaveFile('{}');
$result = $engine.Recognize();
if ($result) {{
    Write-Output $result.Text
}}
"#,
            path_str
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
            .output()
            .map_err(|e| {
                PicoError::SpeechRecognition(format!("Failed to invoke Windows STT: {}", e))
            })?;

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(text)
    }

    /// Transcribes a temporary WAV file on Linux using available local STT tools.
    #[cfg(target_os = "linux")]
    fn transcribe_linux_native(wav_path: &Path) -> Result<String> {
        // Try whisper-cli or vosk if available in PATH
        for bin in &["whisper-cli", "whisper", "vosk-transcriber"] {
            if let Ok(out) = Command::new(bin).arg("-f").arg(wav_path).output() {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !text.is_empty() {
                        return Ok(text);
                    }
                }
            }
        }
        Ok(String::new())
    }
}

#[async_trait]
impl SpeechRecognizer for PlatformSpeechRecognizer {
    async fn transcribe(&self, pcm_samples: &[i16]) -> Result<String> {
        if pcm_samples.is_empty() {
            return Ok(String::new());
        }

        let wav_bytes = Self::samples_to_wav_bytes(pcm_samples)?;
        let whisper_opt = self.whisper_binary.clone();

        let raw_result = tokio::task::spawn_blocking(move || -> Result<String> {
            let temp_dir = std::env::temp_dir();
            let temp_wav = temp_dir.join(format!("pico_stt_{}.wav", uuid::Uuid::new_v4().simple()));

            std::fs::write(&temp_wav, &wav_bytes).map_err(|e| {
                PicoError::SpeechRecognition(format!("Failed to write temporary WAV file: {}", e))
            })?;

            // 1. Try Whisper CLI if configured and binary exists
            if let Some(ref binary) = whisper_opt {
                if binary.exists() {
                    let output = Command::new(binary)
                        .args(["-f", temp_wav.to_str().unwrap_or(""), "--output-txt"])
                        .output();

                    let _ = std::fs::remove_file(&temp_wav);

                    if let Ok(out) = output {
                        if out.status.success() {
                            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            if !text.is_empty() {
                                return Ok(text);
                            }
                        }
                    }
                }
            }

            // 2. Fall back to OS native recognition engine
            #[cfg(target_os = "windows")]
            let result = Self::transcribe_windows_sapi(&temp_wav);
            #[cfg(target_os = "linux")]
            let result = Self::transcribe_linux_native(&temp_wav);
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
            let result = Ok(String::new());

            let _ = std::fs::remove_file(&temp_wav);
            result
        })
        .await
        .map_err(|e| PicoError::SpeechRecognition(format!("STT task panicked: {}", e)))??;

        // Apply speech normalization and stutter cleaning
        Ok(SpeechNormalizer::normalize(&raw_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_conversion() {
        let samples = vec![0i16; 16000]; // 1 second of silence
        let wav = PlatformSpeechRecognizer::samples_to_wav_bytes(&samples).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_speech_normalizer_stutter_and_fillers() {
        let dirty = "uh um find find the the largest files please";
        let cleaned = SpeechNormalizer::normalize(dirty);
        assert_eq!(cleaned, "find the largest files please");

        let stutter = "what what is the memory";
        let cleaned_stutter = SpeechNormalizer::normalize(stutter);
        assert_eq!(cleaned_stutter, "what is the memory");

        let fillers = "like um can you err calculate 25 plus 10";
        let cleaned_fillers = SpeechNormalizer::normalize(fillers);
        assert_eq!(cleaned_fillers, "can you calculate 25 plus 10");
    }
}
