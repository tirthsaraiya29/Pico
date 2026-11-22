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
    /// Safely streams text via standard input to eliminate PowerShell code injection.
    pub fn synthesize_sync(&self, text: &str) -> Result<Vec<u8>> {
        #[cfg(target_os = "windows")]
        {
            use std::io::Write;
            use std::process::Stdio;

            if text.trim().is_empty() {
                return Ok(Vec::new());
            }

            // Fixed compile-time script that reads text purely as data from standard input
            const STATIC_SAPI_SCRIPT: &str = r#"
[Console]::InputEncoding = [System.Text.Encoding]::UTF8;
$text = [Console]::In.ReadToEnd();
if ([string]::IsNullOrWhiteSpace($text)) { exit 0 };
Add-Type -AssemblyName System.Speech;
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer;
$rateStr = $env:PICO_TTS_RATE;
if (-not [string]::IsNullOrWhiteSpace($rateStr)) {
    $parsedRate = 0;
    if ([int]::TryParse($rateStr, [ref]$parsedRate)) {
        $synth.Rate = [Math]::Max(-10, [Math]::Min(10, $parsedRate));
    }
}
$voice = $env:PICO_TTS_VOICE;
if (-not [string]::IsNullOrWhiteSpace($voice)) {
    try { $synth.SelectVoice($voice) } catch {}
}
$stream = New-Object System.IO.MemoryStream;
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);
$synth.SetOutputToAudioStream($stream, $format);
$synth.Speak($text);
$synth.SetOutputToNull();
[System.Convert]::ToBase64String($stream.ToArray());
"#;

            let mut child = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    STATIC_SAPI_SCRIPT,
                ])
                .env("PICO_TTS_RATE", self.speech_rate.to_string())
                .env("PICO_TTS_VOICE", self.voice_name.as_deref().unwrap_or(""))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| {
                    PicoError::SpeechSynthesis(format!(
                        "Failed to invoke SAPI speech process: {}",
                        e
                    ))
                })?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes()).map_err(|e| {
                    PicoError::SpeechSynthesis(format!(
                        "Failed to pipe text to TTS standard input: {}",
                        e
                    ))
                })?;
            }

            let output = child.wait_with_output().map_err(|e| {
                PicoError::SpeechSynthesis(format!(
                    "Speech synthesis process execution failed: {}",
                    e
                ))
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

    #[tokio::test]
    async fn test_malicious_tts_inputs_do_not_inject() {
        // Voice synthesizer should handle backticks, quotes, dollar signs, and newlines safely without script injection
        let malicious_payloads = [
            "Hello `whoami` world",
            "Test with \"double quotes\" and 'single quotes'",
            "Dangerous $(calc.exe) expression and $env:COMPUTERNAME",
            "Multi\r\nline\ntext\rwith\ttabs",
            "Unicode test: 你好 世界! 🚀✨ Русская речь",
            &"A".repeat(2000), // Long payload
        ];

        let synth =
            NativeWindowsSynthesizer::new(Some("NonExistentVoice'; calc.exe; '".into()), 1.0);
        for payload in malicious_payloads {
            let res = synth.synthesize(payload).await;
            // Synthesis either succeeds generating speech or returns cleanly without shell execution
            assert!(res.is_ok());
        }
    }
}
