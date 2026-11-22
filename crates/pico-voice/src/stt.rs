use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::SpeechRecognizer;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Windows Native Speech Recognition Engine and Whisper STT.
pub struct WindowsSpeechRecognizer {
    whisper_binary: Option<PathBuf>,
}

impl WindowsSpeechRecognizer {
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
    fn transcribe_windows_sapi(wav_path: &Path) -> Result<String> {
        #[cfg(target_os = "windows")]
        {
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
        #[cfg(not(target_os = "windows"))]
        {
            let _ = wav_path;
            Ok(String::new())
        }
    }
}

#[async_trait]
impl SpeechRecognizer for WindowsSpeechRecognizer {
    async fn transcribe(&self, pcm_samples: &[i16]) -> Result<String> {
        if pcm_samples.is_empty() {
            return Ok(String::new());
        }

        let wav_bytes = Self::samples_to_wav_bytes(pcm_samples)?;
        let whisper_opt = self.whisper_binary.clone();

        tokio::task::spawn_blocking(move || -> Result<String> {
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

            // 2. Fall back to Windows Native Dictation Engine
            let result = Self::transcribe_windows_sapi(&temp_wav);
            let _ = std::fs::remove_file(&temp_wav);
            result
        })
        .await
        .map_err(|e| PicoError::SpeechRecognition(format!("STT task panicked: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_conversion() {
        let samples = vec![0i16; 16000]; // 1 second of silence
        let wav = WindowsSpeechRecognizer::samples_to_wav_bytes(&samples).unwrap();
        assert!(wav.len() > 44);
        // Check RIFF header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
