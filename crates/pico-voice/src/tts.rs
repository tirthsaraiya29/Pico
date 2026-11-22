use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::SpeechSynthesizer;
use std::process::Command;
use tracing::debug;

/// Windows Native SAPI / System.Speech ultra-low latency Speech Synthesizer.
pub struct NativeWindowsSynthesizer {
    voice_name: Option<String>,
    speech_rate: i32, // -10 to 10
}

impl NativeWindowsSynthesizer {
    pub fn new(voice_name: Option<String>, speech_rate_multiplier: f32) -> Self {
        // Map 0.5..2.0 multiplier to Windows SAPI rate (-10 to 10)
        let rate = ((speech_rate_multiplier - 1.0) * 10.0).clamp(-10.0, 10.0) as i32;
        Self {
            voice_name,
            speech_rate: rate,
        }
    }

    /// Synthesizes text directly to 16kHz 16-bit mono PCM bytes via Windows SAPI.
    pub fn synthesize_sync(&self, text: &str) -> Result<Vec<u8>> {
        #[cfg(target_os = "windows")]
        {
            // Escape quotes in text
            let escaped_text = text.replace('"', "`\"").replace('$', "`$");

            let voice_clause = if let Some(ref v) = self.voice_name {
                format!(r#"$synth.SelectVoice('{}');"#, v)
            } else {
                String::new()
            };

            let ps_script = format!(
                r#"
Add-Type -AssemblyName System.Speech;
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer;
{}
$synth.Rate = {};
$stream = New-Object System.IO.MemoryStream;
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);
$synth.SetOutputToAudioStream($stream, $format);
$synth.Speak("{}");
$synth.SetOutputToNull();
[System.Convert]::ToBase64String($stream.ToArray());
"#,
                voice_clause, self.speech_rate, escaped_text
            );

            let output = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
                .output()
                .map_err(|e| {
                    PicoError::SpeechSynthesis(format!("Failed to invoke SAPI speech: {}", e))
                })?;

            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                return Err(PicoError::SpeechSynthesis(format!(
                    "Windows speech synthesis error: {}",
                    err
                )));
            }

            let b64 = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if b64.is_empty() {
                return Err(PicoError::SpeechSynthesis(
                    "Empty speech synthesis stream received".into(),
                ));
            }

            let decoded = base64_decode(&b64).map_err(|e| {
                PicoError::SpeechSynthesis(format!("Failed to decode audio base64: {}", e))
            })?;

            debug!(bytes = decoded.len(), "Synthesized speech audio bytes");
            Ok(decoded)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = text;
            Ok(Vec::new())
        }
    }
}

#[async_trait]
impl SpeechSynthesizer for NativeWindowsSynthesizer {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let text_owned = text.to_string();
        let synth = NativeWindowsSynthesizer {
            voice_name: self.voice_name.clone(),
            speech_rate: self.speech_rate,
        };

        // Run synthesis on blocking threadpool so Tokio workers remain free
        tokio::task::spawn_blocking(move || synth.synthesize_sync(&text_owned))
            .await
            .map_err(|e| PicoError::SpeechSynthesis(format!("Synthesis thread panic: {}", e)))?
    }
}

/// Helper function to decode base64 without heavy external dependencies
fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table_map = [255u8; 256];
    for (i, &b) in TABLE.iter().enumerate() {
        table_map[b as usize] = i as u8;
    }

    let clean: Vec<u8> = input
        .bytes()
        .filter(|&b| !b.is_ascii_whitespace() && b != b'=')
        .collect();

    let mut output = Vec::with_capacity(clean.len() * 3 / 4);

    for chunk in clean.chunks(4) {
        let mut buf = 0u32;
        let mut bits = 0;

        for &c in chunk {
            let val = table_map[c as usize];
            if val == 255 {
                return Err(format!("Invalid base64 character: {}", c as char));
            }
            buf = (buf << 6) | (val as u32);
            bits += 6;
        }

        while bits >= 8 {
            bits -= 8;
            output.push(((buf >> bits) & 0xFF) as u8);
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode() {
        let input = "SGVsbG8gV29ybGQ=";
        let decoded = base64_decode(input).unwrap();
        assert_eq!(String::from_utf8_lossy(&decoded), "Hello World");
    }

    #[tokio::test]
    async fn test_native_tts_synthesis() {
        let synth = NativeWindowsSynthesizer::new(None, 1.0);
        let audio = synth.synthesize("Pico online.").await;
        assert!(audio.is_ok());
        let bytes = audio.unwrap();
        assert!(!bytes.is_empty());
    }
}
