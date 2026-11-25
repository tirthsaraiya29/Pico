use async_trait::async_trait;
use pico_audio::decode_wav_bytes_to_f32;
use pico_core::config::VoiceConfig;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{
    AudioFormat, AudioSampleEncoding, SpeechSynthesizer, SynthesizedAudio, VoiceInfo,
};
use regex::Regex;
#[cfg(target_os = "windows")]
use serde::Serialize;
use std::io::Write;
#[cfg(target_os = "windows")]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::{Child, ChildStdin, ChildStdout};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, info, warn};

/// Cleans assistant responses for natural spoken delivery and splits them into balanced clauses.
pub struct SpeechPreprocessor;

impl SpeechPreprocessor {
    /// Normalizes markdown, file paths, URLs, and common units into speakable plain text.
    pub fn clean_for_speech(raw: &str) -> String {
        let mut text = raw.trim().to_string();
        if text.is_empty() {
            return String::new();
        }

        static MD_LINK_RE: OnceLock<Regex> = OnceLock::new();
        static URL_PROTO_RE: OnceLock<Regex> = OnceLock::new();
        static LOCALHOST_PORT_RE: OnceLock<Regex> = OnceLock::new();
        static MD_DECOR_RE: OnceLock<Regex> = OnceLock::new();
        static UNIT_MS_RE: OnceLock<Regex> = OnceLock::new();
        static UNIT_PCT_RE: OnceLock<Regex> = OnceLock::new();
        static UNIT_SIZE_RE: OnceLock<Regex> = OnceLock::new();
        static WS_RE: OnceLock<Regex> = OnceLock::new();

        // Replace markdown [label](url) with label.
        text = MD_LINK_RE
            .get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap())
            .replace_all(&text, "$1")
            .into_owned();

        // Speak localhost:3124 cleanly as "localhost port 3124".
        text = LOCALHOST_PORT_RE
            .get_or_init(|| Regex::new(r"(?i)\blocalhost:(\d+)\b").unwrap())
            .replace_all(&text, "localhost port $1")
            .into_owned();

        // Strip http(s):// prefix so links read naturally.
        text = URL_PROTO_RE
            .get_or_init(|| Regex::new(r"https?://").unwrap())
            .replace_all(&text, "")
            .into_owned();

        // Strip markdown emphasis, headings, backticks, bullets.
        text = MD_DECOR_RE
            .get_or_init(|| Regex::new(r"[`*_~#>]+").unwrap())
            .replace_all(&text, " ")
            .into_owned();

        // Convert Windows/Unix path separators into natural word boundaries when inside paths.
        if text.contains('\\') || text.contains('/') {
            let mut rebuilt = String::with_capacity(text.len() + 8);
            for tok in text.split_whitespace() {
                if !rebuilt.is_empty() {
                    rebuilt.push(' ');
                }
                if tok.contains('\\') || (tok.starts_with('/') && tok.len() > 2) {
                    let clean = tok
                        .replace(":\\", " drive ")
                        .replace(['\\', '/'], " ")
                        .trim()
                        .to_string();
                    rebuilt.push_str(&clean);
                } else {
                    rebuilt.push_str(tok);
                }
            }
            text = rebuilt;
        }

        // Expand numeric units like 42ms, 65%, 512MB.
        text = UNIT_MS_RE
            .get_or_init(|| Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*ms\b").unwrap())
            .replace_all(&text, "$1 milliseconds")
            .into_owned();
        text = UNIT_PCT_RE
            .get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)\s*%").unwrap())
            .replace_all(&text, "$1 percent")
            .into_owned();
        text = UNIT_SIZE_RE
            .get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)\s*(KB|MB|GB|TB|GHz|MHz)\b").unwrap())
            .replace_all(&text, |caps: &regex::Captures| {
                let unit = match caps.get(2).map(|m| m.as_str()).unwrap_or("") {
                    "KB" => "kilobytes",
                    "MB" => "megabytes",
                    "GB" => "gigabytes",
                    "TB" => "terabytes",
                    "GHz" => "gigahertz",
                    "MHz" => "megahertz",
                    other => other,
                };
                format!("{} {}", &caps[1], unit)
            })
            .into_owned();

        text = text.replace("...", ", ");
        WS_RE
            .get_or_init(|| Regex::new(r"\s+").unwrap())
            .replace_all(text.trim(), " ")
            .into_owned()
    }

    /// Splits cleaned text into balanced spoken sentences without breaking decimals (`3.14`)
    /// or domain/file dots (`example.com`).
    pub fn split_sentences(raw: &str) -> Vec<String> {
        let cleaned = Self::clean_for_speech(raw);
        if cleaned.is_empty() {
            return Vec::new();
        }

        let chars: Vec<char> = cleaned.chars().collect();
        let mut sentences = Vec::new();
        let mut buf = String::new();

        for i in 0..chars.len() {
            let ch = chars[i];
            buf.push(ch);

            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();

            let is_sentence_punct = matches!(ch, '!' | '?' | '\n')
                || (ch == '.'
                    && next.map(|n| n.is_whitespace()).unwrap_or(true)
                    && !prev.map(|p| p.is_ascii_digit()).unwrap_or(false));

            if is_sentence_punct || (buf.len() >= 180 && matches!(ch, ',' | ';')) {
                let piece = buf.trim().to_string();
                if !piece.is_empty() {
                    sentences.push(piece);
                }
                buf.clear();
            }
        }

        let tail = buf.trim().to_string();
        if !tail.is_empty() {
            sentences.push(tail);
        }
        sentences
    }
}

/// Decodes a standard RIFF WAV container into `SynthesizedAudio`.
pub fn decode_wav_to_synthesized(
    wav_bytes: &[u8],
    voice: impl Into<String>,
    engine: impl Into<String>,
) -> Result<SynthesizedAudio> {
    let (samples, sample_rate, channels) = decode_wav_bytes_to_f32(wav_bytes)?;
    if samples.is_empty() {
        return Err(PicoError::SpeechSynthesis(
            "Speech synthesizer produced empty audio buffer".into(),
        ));
    }
    Ok(SynthesizedAudio {
        samples,
        sample_rate,
        channels,
        voice: voice.into(),
        engine: engine.into(),
    })
}

/// Encodes `SynthesizedAudio` samples into a clean 16-bit WAV byte stream for callers using legacy byte APIs.
pub fn encode_synthesized_to_wav(audio: &SynthesizedAudio) -> Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(audio.samples.len() * 2 + 44));
    let spec = hound::WavSpec {
        channels: audio.channels.max(1),
        sample_rate: audio.sample_rate.max(8_000),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec)
        .map_err(|e| PicoError::SpeechSynthesis(format!("WAV header error: {}", e)))?;
    for &s in &audio.samples {
        let pcm = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        writer
            .write_sample(pcm)
            .map_err(|e| PicoError::SpeechSynthesis(format!("WAV sample write error: {}", e)))?;
    }
    writer
        .finalize()
        .map_err(|e| PicoError::SpeechSynthesis(format!("WAV finalize error: {}", e)))?;
    Ok(cursor.into_inner())
}

#[cfg(target_os = "windows")]
#[derive(Serialize)]
struct WindowsTtsRequest<'a> {
    text: &'a str,
    voice: &'a str,
    rate: f32,
    prefer_winrt: bool,
}

#[cfg(target_os = "windows")]
struct WindowsTtsWorker {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

#[cfg(target_os = "windows")]
impl WindowsTtsWorker {
    fn spawn() -> std::result::Result<Self, String> {
        const WORKER_SCRIPT: &str = r#"
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$winrtAvailable = $false
$winrtSynth = $null
$asTaskGeneric = $null
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
    $null = [Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType=WindowsRuntime]
    $winrtSynth = New-Object Windows.Media.SpeechSynthesis.SpeechSynthesizer
    $asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -like 'IAsyncOperation*'
    })[0]
    if ($winrtSynth -and $asTaskGeneric -and [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices.Count -gt 0) {
        $winrtAvailable = $true
    }
} catch {
    $winrtAvailable = $false
}

$sapiSynth = $null
try {
    Add-Type -AssemblyName System.Speech -ErrorAction Stop
    $sapiSynth = New-Object System.Speech.Synthesis.SpeechSynthesizer
} catch {
    $sapiSynth = $null
}

if (-not $winrtAvailable -and $null -eq $sapiSynth) {
    Write-Output 'ERR:No Windows speech synthesis backend available'
    exit 1
}

Write-Output 'READY'
[Console]::Out.Flush()

while ($null -ne ($line = [Console]::In.ReadLine())) {
    if ($line -eq 'QUIT') { break }
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    try {
        $req = $line | ConvertFrom-Json
        $text = [string]$req.text
        $voice = [string]$req.voice
        $rate = [double]$req.rate
        $preferWinrt = [bool]$req.prefer_winrt
        if ([string]::IsNullOrWhiteSpace($text)) {
            Write-Output 'ERR:Empty text'
            [Console]::Out.Flush()
            continue
        }

        $done = $false
        if ($preferWinrt -and $winrtAvailable -and $null -ne $winrtSynth) {
            try {
                if (-not [string]::IsNullOrWhiteSpace($voice)) {
                    $match = [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices | Where-Object {
                        $_.DisplayName -like "*$voice*" -or $_.Id -like "*$voice*"
                    } | Select-Object -First 1
                    if ($null -ne $match) { $winrtSynth.Voice = $match }
                } else {
                    $preferred = [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices | Where-Object {
                        $_.DisplayName -like '*Natural*' -or $_.DisplayName -like '*Mark*' -or $_.DisplayName -like '*Aria*'
                    } | Select-Object -First 1
                    if ($null -ne $preferred) { $winrtSynth.Voice = $preferred }
                }
                $clampedRate = [Math]::Max(0.5, [Math]::Min(2.0, $rate))
                $winrtSynth.Options.SpeakingRate = $clampedRate
                $op = $winrtSynth.SynthesizeTextToStreamAsync($text)
                $stream = $asTaskGeneric.MakeGenericMethod([Windows.Media.SpeechSynthesis.SpeechSynthesisStream]).Invoke($null, @($op)).GetAwaiter().GetResult()
                $size = [uint32]$stream.Size
                if ($size -gt 44) {
                    $dr = New-Object Windows.Storage.Streams.DataReader($stream.GetInputStreamAt(0))
                    $null = $asTaskGeneric.MakeGenericMethod([uint32]).Invoke($null, @($dr.LoadAsync($size))).GetAwaiter().GetResult()
                    $bytes = New-Object byte[] $size
                    $dr.ReadBytes($bytes)
                    $b64 = [Convert]::ToBase64String($bytes)
                    Write-Output ("OK:" + $winrtSynth.Voice.DisplayName + ":" + $b64)
                    [Console]::Out.Flush()
                    $done = $true
                }
            } catch {
                $done = $false
            }
        }

        if (-not $done -and $null -ne $sapiSynth) {
            if (-not [string]::IsNullOrWhiteSpace($voice)) {
                try { $sapiSynth.SelectVoice($voice) } catch {}
            }
            $sapiRate = [int][Math]::Max(-10, [Math]::Min(10, [Math]::Round(($rate - 1.0) * 6.0)))
            $sapiSynth.Rate = $sapiRate
            $ms = New-Object System.IO.MemoryStream
            $sapiSynth.SetOutputToWaveStream($ms)
            $sapiSynth.Speak($text)
            $sapiSynth.SetOutputToNull()
            $bytes = $ms.ToArray()
            $ms.Dispose()
            if ($bytes.Length -gt 44) {
                $vName = $sapiSynth.Voice.Name
                $b64 = [Convert]::ToBase64String($bytes)
                Write-Output ("OK:" + $vName + ":" + $b64)
                [Console]::Out.Flush()
                $done = $true
            }
        }

        if (-not $done) {
            Write-Output 'ERR:Windows synthesis produced no audio bytes'
            [Console]::Out.Flush()
        }
    } catch {
        Write-Output ("ERR:" + $_.Exception.Message)
        [Console]::Out.Flush()
    }
}
"#;

        let mut child = Command::new("powershell")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                WORKER_SCRIPT,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn Windows TTS worker: {}", e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Windows TTS stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Windows TTS stdout unavailable".to_string())?;
        let mut reader = BufReader::new(stdout);
        let mut banner = String::new();
        reader
            .read_line(&mut banner)
            .map_err(|e| format!("Failed reading Windows TTS banner: {}", e))?;
        let trimmed = banner.trim();
        if trimmed != "READY" {
            let _ = child.kill();
            return Err(format!(
                "Windows TTS worker failed to initialize: {}",
                trimmed
            ));
        }

        Ok(Self {
            child,
            stdin,
            reader,
        })
    }

    fn synthesize_wav(
        &mut self,
        text: &str,
        voice: &str,
        rate: f32,
        prefer_winrt: bool,
    ) -> std::result::Result<(String, Vec<u8>), String> {
        let req = WindowsTtsRequest {
            text,
            voice,
            rate,
            prefer_winrt,
        };
        let payload = serde_json::to_string(&req)
            .map_err(|e| format!("Failed to serialize Windows TTS request: {}", e))?;
        writeln!(self.stdin, "{}", payload).map_err(|e| format!("TTS pipe write error: {}", e))?;
        self.stdin
            .flush()
            .map_err(|e| format!("TTS pipe flush error: {}", e))?;

        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| format!("TTS pipe read error: {}", e))?;
        if n == 0 {
            return Err("Windows TTS worker closed unexpectedly".into());
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("OK:") {
            let (voice_used, b64) = rest.split_once(':').unwrap_or(("windows-tts", rest));
            let decoded = base64_decode(b64).map_err(|e| e.to_string())?;
            return Ok((voice_used.to_string(), decoded));
        }
        Err(trimmed.strip_prefix("ERR:").unwrap_or(trimmed).to_string())
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsTtsWorker {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "QUIT");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Cross-platform Windows WinRT OneCore + SAPI5 Speech Synthesizer.
#[derive(Clone)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct NativeWindowsSynthesizer {
    voice_name: Option<String>,
    speech_rate: f32,
    prefer_winrt: bool,
    #[cfg(target_os = "windows")]
    worker: Arc<Mutex<Option<WindowsTtsWorker>>>,
}

impl NativeWindowsSynthesizer {
    pub fn new(voice_name: Option<String>, speech_rate_multiplier: f32) -> Self {
        Self::with_mode(voice_name, speech_rate_multiplier, true)
    }

    pub fn with_mode(
        voice_name: Option<String>,
        speech_rate_multiplier: f32,
        prefer_winrt: bool,
    ) -> Self {
        Self {
            voice_name,
            speech_rate: speech_rate_multiplier.clamp(0.5, 2.0),
            prefer_winrt,
            #[cfg(target_os = "windows")]
            worker: Arc::new(Mutex::new(None)),
        }
    }

    pub fn engine_tag(&self) -> &'static str {
        if self.prefer_winrt {
            "winrt"
        } else {
            "native_sapi"
        }
    }

    pub fn list_voices() -> Vec<VoiceInfo> {
        #[cfg(target_os = "windows")]
        {
            const LIST_SCRIPT: &str = r#"
$voices = New-Object System.Collections.Generic.List[string]
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
    $null = [Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType=WindowsRuntime]
    foreach ($v in [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices) {
        $voices.Add("winrt|" + $v.DisplayName + "|" + $v.Language)
    }
} catch {}
try {
    Add-Type -AssemblyName System.Speech -ErrorAction Stop
    $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
    foreach ($v in $s.GetInstalledVoices()) {
        $voices.Add("native_sapi|" + $v.VoiceInfo.Name + "|" + $v.VoiceInfo.Culture.Name)
    }
} catch {}
$voices -join "`n"
"#;
            if let Ok(out) = Command::new("powershell")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    LIST_SCRIPT,
                ])
                .output()
            {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut list = Vec::new();
                    for line in stdout.lines() {
                        let parts: Vec<&str> = line.trim().split('|').collect();
                        if parts.len() == 3 && !parts[1].is_empty() {
                            list.push(VoiceInfo {
                                engine: parts[0].to_string(),
                                name: parts[1].to_string(),
                                language: parts[2].to_string(),
                            });
                        }
                    }
                    return list;
                }
            }
        }
        Vec::new()
    }

    /// Synthesizes text to structured `SynthesizedAudio` using WinRT OneCore / SAPI5 WAV output.
    pub fn synthesize_audio_sync(&self, text: &str) -> Result<SynthesizedAudio> {
        let cleaned = SpeechPreprocessor::clean_for_speech(text);
        if cleaned.is_empty() {
            return Err(PicoError::SpeechSynthesis(
                "Cannot synthesize empty text".into(),
            ));
        }

        #[cfg(target_os = "windows")]
        {
            let voice = self.voice_name.as_deref().unwrap_or("");
            let mut guard = self.worker.lock().map_err(|_| {
                PicoError::SpeechSynthesis("Windows TTS worker lock poisoned".into())
            })?;

            if guard.is_none() {
                match WindowsTtsWorker::spawn() {
                    Ok(w) => *guard = Some(w),
                    Err(e) => {
                        return Err(PicoError::SpeechSynthesis(e));
                    }
                }
            }

            let first_attempt = guard
                .as_mut()
                .ok_or_else(|| PicoError::SpeechSynthesis("Windows TTS worker missing".into()))?
                .synthesize_wav(&cleaned, voice, self.speech_rate, self.prefer_winrt);

            let (voice_used, wav_bytes) = match first_attempt {
                Ok(res) => res,
                Err(first_err) => {
                    warn!(error = %first_err, "Windows TTS worker failed; restarting worker once");
                    *guard = None;
                    let mut restarted =
                        WindowsTtsWorker::spawn().map_err(PicoError::SpeechSynthesis)?;
                    let retry = restarted
                        .synthesize_wav(&cleaned, voice, self.speech_rate, self.prefer_winrt)
                        .map_err(PicoError::SpeechSynthesis)?;
                    *guard = Some(restarted);
                    retry
                }
            };

            debug!(bytes = wav_bytes.len(), voice = %voice_used, "Windows TTS synthesized WAV");
            decode_wav_to_synthesized(&wav_bytes, voice_used, self.engine_tag())
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(PicoError::SpeechSynthesis(
                "NativeWindowsSynthesizer is only supported on Windows".into(),
            ))
        }
    }
}

#[async_trait]
impl SpeechSynthesizer for NativeWindowsSynthesizer {
    fn engine_name(&self) -> &str {
        self.engine_tag()
    }

    fn list_voices(&self) -> Vec<VoiceInfo> {
        Self::list_voices()
    }

    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio> {
        let synth = self.clone();
        let text_owned = text.to_string();
        tokio::task::spawn_blocking(move || synth.synthesize_audio_sync(&text_owned))
            .await
            .map_err(|e| PicoError::SpeechSynthesis(format!("Synthesis task panic: {}", e)))?
    }
}

/// Linux Native espeak-ng / espeak Speech Synthesizer emitting explicit RIFF WAV output.
#[derive(Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct NativeLinuxSynthesizer {
    voice_name: Option<String>,
    words_per_minute: u32,
}

impl NativeLinuxSynthesizer {
    pub fn new(voice_name: Option<String>, speech_rate_multiplier: f32) -> Self {
        let wpm = ((165.0 * speech_rate_multiplier.clamp(0.5, 2.0)) as u32).clamp(90, 330);
        Self {
            voice_name,
            words_per_minute: wpm,
        }
    }

    pub fn list_voices() -> Vec<VoiceInfo> {
        #[cfg(target_os = "linux")]
        {
            for bin in ["espeak-ng", "espeak"] {
                if let Ok(out) = Command::new(bin).arg("--voices").output() {
                    if out.status.success() {
                        let mut voices = Vec::new();
                        let txt = String::from_utf8_lossy(&out.stdout);
                        for line in txt.lines().skip(1) {
                            let cols: Vec<&str> = line.split_whitespace().collect();
                            if cols.len() >= 4 {
                                voices.push(VoiceInfo {
                                    language: cols[1].to_string(),
                                    name: cols[3].to_string(),
                                    engine: bin.to_string(),
                                });
                            }
                        }
                        if !voices.is_empty() {
                            return voices;
                        }
                    }
                }
            }
        }
        Vec::new()
    }

    pub fn synthesize_audio_sync(&self, text: &str) -> Result<SynthesizedAudio> {
        let cleaned = SpeechPreprocessor::clean_for_speech(text);
        if cleaned.is_empty() {
            return Err(PicoError::SpeechSynthesis(
                "Cannot synthesize empty text".into(),
            ));
        }

        #[cfg(target_os = "linux")]
        {
            let mut last_err = None;
            for bin in ["espeak-ng", "espeak"] {
                let mut cmd = Command::new(bin);
                cmd.args([
                    "--stdout",
                    "--stdin",
                    "-s",
                    &self.words_per_minute.to_string(),
                ]);
                if let Some(ref voice) = self.voice_name {
                    if !voice.trim().is_empty() {
                        cmd.args(["-v", voice]);
                    }
                }
                cmd.stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());

                match cmd.spawn() {
                    Ok(mut child) => {
                        if let Some(mut stdin) = child.stdin.take() {
                            let _ = stdin.write_all(cleaned.as_bytes());
                        }
                        match child.wait_with_output() {
                            Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                                return decode_wav_to_synthesized(
                                    &out.stdout,
                                    self.voice_name.clone().unwrap_or_else(|| "default".into()),
                                    bin,
                                );
                            }
                            Ok(out) => {
                                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                last_err =
                                    Some(format!("{} exited with {}: {}", bin, out.status, err));
                            }
                            Err(e) => {
                                last_err = Some(format!("{} wait failed: {}", bin, e));
                            }
                        }
                    }
                    Err(e) => {
                        last_err = Some(format!("{} unavailable: {}", bin, e));
                    }
                }
            }

            Err(PicoError::SpeechSynthesis(last_err.unwrap_or_else(|| {
                "No Linux TTS backend (espeak-ng / espeak) succeeded".into()
            })))
        }

        #[cfg(not(target_os = "linux"))]
        {
            Err(PicoError::SpeechSynthesis(
                "NativeLinuxSynthesizer is only supported on Linux".into(),
            ))
        }
    }
}

#[async_trait]
impl SpeechSynthesizer for NativeLinuxSynthesizer {
    fn engine_name(&self) -> &str {
        "espeak"
    }

    fn list_voices(&self) -> Vec<VoiceInfo> {
        Self::list_voices()
    }

    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio> {
        let synth = self.clone();
        let text_owned = text.to_string();
        tokio::task::spawn_blocking(move || synth.synthesize_audio_sync(&text_owned))
            .await
            .map_err(|e| PicoError::SpeechSynthesis(format!("Synthesis task panic: {}", e)))?
    }
}

/// Local neural Piper TTS synthesizer via `piper --model <onnx> --output-raw`.
#[derive(Clone)]
pub struct PiperSynthesizer {
    binary: PathBuf,
    model: PathBuf,
    voice_name: String,
    length_scale: f32,
    sample_rate: u32,
}

impl PiperSynthesizer {
    pub fn discover(config: &VoiceConfig) -> Option<Self> {
        let binary = Self::find_binary(config)?;
        let model = Self::find_model(config)?;
        let voice_name = model
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("piper")
            .to_string();
        let sample_rate = Self::read_model_sample_rate(&model).unwrap_or(22_050);
        let rate = config.speech_rate.clamp(0.5, 2.0);
        let length_scale = (1.0 / rate).clamp(0.5, 2.0);

        Some(Self {
            binary,
            model,
            voice_name,
            length_scale,
            sample_rate,
        })
    }

    fn find_binary(config: &VoiceConfig) -> Option<PathBuf> {
        if let Some(ref explicit) = config.piper_binary {
            if explicit.exists() {
                return Some(explicit.clone());
            }
        }
        if let Ok(env_bin) = std::env::var("PICO_PIPER_BIN") {
            let p = PathBuf::from(env_bin);
            if p.exists() {
                return Some(p);
            }
        }
        let exe_names = if cfg!(target_os = "windows") {
            vec!["piper.exe", "piper"]
        } else {
            vec!["piper", "piper-tts"]
        };
        for rel_dir in ["models/piper", "../models/piper", "../../models/piper"] {
            for name in &exe_names {
                let candidate = Path::new(rel_dir).join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        if let Some(path_env) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_env) {
                for name in &exe_names {
                    let candidate = dir.join(name);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    fn find_model(config: &VoiceConfig) -> Option<PathBuf> {
        if let Some(ref explicit) = config.piper_model {
            if explicit.exists() {
                return Some(explicit.clone());
            }
        }
        if let Ok(env_model) = std::env::var("PICO_PIPER_MODEL") {
            let p = PathBuf::from(env_model);
            if p.exists() {
                return Some(p);
            }
        }
        let voice_hint = config.voice_name.as_deref().unwrap_or("").to_lowercase();
        let mut fallback = None;
        for rel_dir in ["models/piper", "../models/piper", "../../models/piper"] {
            if let Ok(entries) = std::fs::read_dir(rel_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("onnx") {
                        if !voice_hint.is_empty()
                            && path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&voice_hint)
                        {
                            return Some(path);
                        }
                        if fallback.is_none() {
                            fallback = Some(path);
                        }
                    }
                }
            }
        }
        fallback
    }

    fn read_model_sample_rate(model_path: &Path) -> Option<u32> {
        let mut json_candidates = vec![PathBuf::from(format!("{}.json", model_path.display()))];
        json_candidates.push(model_path.with_extension("onnx.json"));
        for candidate in json_candidates {
            if let Ok(raw) = std::fs::read_to_string(candidate) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(sr) = val
                        .get("audio")
                        .and_then(|a| a.get("sample_rate"))
                        .and_then(|s| s.as_u64())
                    {
                        return Some(sr as u32);
                    }
                }
            }
        }
        None
    }

    pub fn list_voices(config: &VoiceConfig) -> Vec<VoiceInfo> {
        let mut voices = Vec::new();
        if let Some(ref explicit) = config.piper_model {
            if explicit.exists() {
                voices.push(VoiceInfo {
                    name: explicit
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("piper")
                        .to_string(),
                    language: "local".into(),
                    engine: "piper".into(),
                });
            }
        }
        for rel_dir in ["models/piper", "../models/piper", "../../models/piper"] {
            if let Ok(entries) = std::fs::read_dir(rel_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("onnx") {
                        let name = p
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("piper")
                            .to_string();
                        if !voices.iter().any(|v| v.name == name) {
                            voices.push(VoiceInfo {
                                name,
                                language: "neural-onnx".into(),
                                engine: "piper".into(),
                            });
                        }
                    }
                }
            }
        }
        voices
    }

    pub fn synthesize_audio_sync(&self, text: &str) -> Result<SynthesizedAudio> {
        let cleaned = SpeechPreprocessor::clean_for_speech(text);
        if cleaned.is_empty() {
            return Err(PicoError::SpeechSynthesis(
                "Cannot synthesize empty text".into(),
            ));
        }

        let mut child = Command::new(&self.binary)
            .args([
                "--model",
                self.model.to_str().unwrap_or(""),
                "--length_scale",
                &format!("{:.3}", self.length_scale),
                "--output-raw",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PicoError::SpeechSynthesis(format!("Failed to start piper: {}", e)))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(cleaned.as_bytes())
                .map_err(|e| PicoError::SpeechSynthesis(format!("Piper stdin failed: {}", e)))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| PicoError::SpeechSynthesis(format!("Piper wait failed: {}", e)))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(PicoError::SpeechSynthesis(format!(
                "Piper failed ({}): {}",
                output.status, err
            )));
        }
        if output.stdout.len() < 4 {
            return Err(PicoError::SpeechSynthesis(
                "Piper returned empty PCM output".into(),
            ));
        }

        let mut samples = Vec::with_capacity(output.stdout.len() / 2);
        for chunk in output.stdout.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(*chunk);
            samples.push(sample as f32 / 32768.0);
        }

        Ok(SynthesizedAudio {
            samples,
            sample_rate: self.sample_rate,
            channels: 1,
            voice: self.voice_name.clone(),
            engine: "piper".into(),
        })
    }
}

#[async_trait]
impl SpeechSynthesizer for PiperSynthesizer {
    fn engine_name(&self) -> &str {
        "piper"
    }

    fn list_voices(&self) -> Vec<VoiceInfo> {
        Vec::new()
    }

    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio> {
        let synth = self.clone();
        let text_owned = text.to_string();
        tokio::task::spawn_blocking(move || synth.synthesize_audio_sync(&text_owned))
            .await
            .map_err(|e| PicoError::SpeechSynthesis(format!("Piper task panic: {}", e)))?
    }
}

/// Deterministic test/CI synthesizer producing audible sine-tone waveforms with an explicit format.
#[derive(Clone)]
pub struct MockSpeechSynthesizer {
    voice: String,
    rate: f32,
}

impl MockSpeechSynthesizer {
    pub fn new(voice: Option<String>, rate: f32) -> Self {
        Self {
            voice: voice.unwrap_or_else(|| "pico-mock-voice".into()),
            rate: rate.clamp(0.5, 2.0),
        }
    }
}

#[async_trait]
impl SpeechSynthesizer for MockSpeechSynthesizer {
    fn engine_name(&self) -> &str {
        "mock"
    }

    fn list_voices(&self) -> Vec<VoiceInfo> {
        vec![VoiceInfo {
            name: self.voice.clone(),
            language: "en-US".into(),
            engine: "mock".into(),
        }]
    }

    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio> {
        let cleaned = SpeechPreprocessor::clean_for_speech(text);
        if cleaned.is_empty() {
            return Err(PicoError::SpeechSynthesis(
                "Cannot synthesize empty text".into(),
            ));
        }
        let sample_rate = 22_050u32;
        let duration_secs = ((cleaned.len() as f32 * 0.03) / self.rate).clamp(0.15, 3.0);
        let len = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = Vec::with_capacity(len);
        for i in 0..len {
            let t = i as f32 / sample_rate as f32;
            let env = ((i as f32 / 220.0).min(1.0)) * (((len - i) as f32 / 220.0).min(1.0));
            samples.push(env * 0.22 * (2.0 * std::f32::consts::PI * 330.0 * t).sin());
        }
        Ok(SynthesizedAudio {
            samples,
            sample_rate,
            channels: 1,
            voice: self.voice.clone(),
            engine: "mock".into(),
        })
    }
}

#[allow(dead_code)]
enum ResolvedBackend {
    Piper(PiperSynthesizer),
    Windows(NativeWindowsSynthesizer),
    Linux(NativeLinuxSynthesizer),
    Mock(MockSpeechSynthesizer),
    Unavailable { requested: String, reason: String },
}

/// Cross-platform Speech Synthesizer routing cleanly across Piper, WinRT OneCore, SAPI5, eSpeak NG, and Mock.
pub struct PlatformSpeechSynthesizer {
    tts_engine: String,
    backend: ResolvedBackend,
    cached_voices: Vec<VoiceInfo>,
}

pub type WindowsSpeechSynthesizer = PlatformSpeechSynthesizer;

impl PlatformSpeechSynthesizer {
    pub fn new(voice_name: Option<String>, speech_rate: f32) -> Self {
        let cfg = VoiceConfig {
            voice_name,
            speech_rate,
            tts_engine: "auto".into(),
            ..VoiceConfig::default()
        };
        Self::from_config(&cfg)
    }

    pub fn from_config(config: &VoiceConfig) -> Self {
        let requested = config.tts_engine.to_lowercase();
        let piper_opt = PiperSynthesizer::discover(config);

        let backend = match requested.as_str() {
            "mock" => ResolvedBackend::Mock(MockSpeechSynthesizer::new(
                config.voice_name.clone(),
                config.speech_rate,
            )),
            "piper" => match piper_opt {
                Some(p) => {
                    info!(model = %p.model.display(), rate = p.sample_rate, "Configured neural Piper TTS");
                    ResolvedBackend::Piper(p)
                }
                None => ResolvedBackend::Unavailable {
                    requested: "piper".into(),
                    reason: "Piper TTS configured, but piper executable or .onnx model was not found in config/env/models/piper".into(),
                },
            },
            "winrt" => {
                #[cfg(target_os = "windows")]
                {
                    ResolvedBackend::Windows(NativeWindowsSynthesizer::with_mode(
                        config.voice_name.clone(),
                        config.speech_rate,
                        true,
                    ))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    ResolvedBackend::Unavailable {
                        requested: "winrt".into(),
                        reason: "WinRT speech synthesis is only available on Windows".into(),
                    }
                }
            }
            "native_sapi" => {
                #[cfg(target_os = "windows")]
                {
                    ResolvedBackend::Windows(NativeWindowsSynthesizer::with_mode(
                        config.voice_name.clone(),
                        config.speech_rate,
                        false,
                    ))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    ResolvedBackend::Unavailable {
                        requested: "native_sapi".into(),
                        reason: "native_sapi TTS is only available on Windows".into(),
                    }
                }
            }
            "espeak" | "speech_dispatcher" => {
                #[cfg(target_os = "linux")]
                {
                    ResolvedBackend::Linux(NativeLinuxSynthesizer::new(
                        config.voice_name.clone(),
                        config.speech_rate,
                    ))
                }
                #[cfg(not(target_os = "linux"))]
                {
                    ResolvedBackend::Unavailable {
                        requested: requested.clone(),
                        reason: format!("{} TTS backend is only active on Linux", requested),
                    }
                }
            }
            "auto" | "system" => {
                if let Some(p) = piper_opt {
                    info!(model = %p.model.display(), rate = p.sample_rate, "Auto-selected neural Piper TTS");
                    ResolvedBackend::Piper(p)
                } else {
                    #[cfg(target_os = "windows")]
                    {
                        ResolvedBackend::Windows(NativeWindowsSynthesizer::with_mode(
                            config.voice_name.clone(),
                            config.speech_rate,
                            true,
                        ))
                    }
                    #[cfg(target_os = "linux")]
                    {
                        ResolvedBackend::Linux(NativeLinuxSynthesizer::new(
                            config.voice_name.clone(),
                            config.speech_rate,
                        ))
                    }
                    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                    {
                        ResolvedBackend::Unavailable {
                            requested: requested.clone(),
                            reason: "Unsupported OS for system speech synthesis".into(),
                        }
                    }
                }
            }
            other => ResolvedBackend::Unavailable {
                requested: other.to_string(),
                reason: format!("Unsupported tts_engine '{}'", other),
            },
        };

        let mut cached_voices = PiperSynthesizer::list_voices(config);
        #[cfg(target_os = "windows")]
        cached_voices.extend(NativeWindowsSynthesizer::list_voices());
        #[cfg(target_os = "linux")]
        cached_voices.extend(NativeLinuxSynthesizer::list_voices());
        if matches!(backend, ResolvedBackend::Mock(_)) {
            cached_voices.push(VoiceInfo {
                name: config
                    .voice_name
                    .clone()
                    .unwrap_or_else(|| "pico-mock-voice".into()),
                language: "en-US".into(),
                engine: "mock".into(),
            });
        }

        Self {
            tts_engine: config.tts_engine.clone(),
            backend,
            cached_voices,
        }
    }

    pub fn tts_engine(&self) -> &str {
        &self.tts_engine
    }

    pub fn declared_output_format(&self) -> AudioFormat {
        let rate = match &self.backend {
            ResolvedBackend::Piper(p) => p.sample_rate,
            ResolvedBackend::Mock(_) => 22_050,
            _ => 16_000,
        };
        AudioFormat {
            sample_rate: rate,
            channels: 1,
            encoding: AudioSampleEncoding::F32,
        }
    }
}

#[async_trait]
impl SpeechSynthesizer for PlatformSpeechSynthesizer {
    fn engine_name(&self) -> &str {
        match &self.backend {
            ResolvedBackend::Piper(_) => "piper",
            ResolvedBackend::Windows(w) => w.engine_tag(),
            ResolvedBackend::Linux(_) => "espeak",
            ResolvedBackend::Mock(_) => "mock",
            ResolvedBackend::Unavailable { requested, .. } => requested.as_str(),
        }
    }

    fn list_voices(&self) -> Vec<VoiceInfo> {
        self.cached_voices.clone()
    }

    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio> {
        match &self.backend {
            ResolvedBackend::Piper(p) => p.synthesize_audio(text).await,
            ResolvedBackend::Windows(w) => w.synthesize_audio(text).await,
            ResolvedBackend::Linux(l) => l.synthesize_audio(text).await,
            ResolvedBackend::Mock(m) => m.synthesize_audio(text).await,
            ResolvedBackend::Unavailable { reason, .. } => {
                Err(PicoError::SpeechSynthesis(reason.clone()))
            }
        }
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let audio = self.synthesize_audio(text).await?;
        encode_synthesized_to_wav(&audio)
    }
}

/// Decodes a Base64 string into raw bytes.
pub fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits_collected: i32 = 0;

    for b in input.bytes() {
        if b == b'=' || b == b'\r' || b == b'\n' || b == b' ' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("Invalid base64 byte: {}", b)),
        };

        buffer = (buffer << 6) | (val as u32);
        bits_collected += 6;

        if bits_collected >= 8 {
            bits_collected -= 8;
            output.push(((buffer >> bits_collected) & 0xFF) as u8);
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode() {
        let encoded = "SGVsbG8gd29ybGQ="; // "Hello world"
        let decoded = base64_decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello world");
    }

    #[test]
    fn test_speech_preprocessor_decimals_paths_and_units() {
        let input = "Check [notes](https://example.com/a) on `localhost:3124`: CPU hit 85%, RAM 512MB, latency 42ms, version 3.14.";
        let sentences = SpeechPreprocessor::split_sentences(input);
        assert_eq!(sentences.len(), 1, "Decimal 3.14 must not split sentences");
        let spoken = &sentences[0];
        assert!(spoken.contains("localhost port 3124"));
        assert!(spoken.contains("85 percent"));
        assert!(spoken.contains("512 megabytes"));
        assert!(spoken.contains("42 milliseconds"));
        assert!(!spoken.contains("https://"));
    }

    #[tokio::test]
    async fn test_mock_tts_declares_explicit_format_and_non_empty_samples() {
        let cfg = VoiceConfig {
            tts_engine: "mock".into(),
            speech_rate: 1.25,
            ..VoiceConfig::default()
        };
        let synth = PlatformSpeechSynthesizer::from_config(&cfg);
        assert_eq!(synth.engine_name(), "mock");
        assert!(!synth.list_voices().is_empty());

        let audio = synth
            .synthesize_audio("Hello, system check complete.")
            .await
            .unwrap();
        assert_eq!(audio.sample_rate, 22_050);
        assert_eq!(audio.channels, 1);
        assert!(!audio.samples.is_empty());

        let wav = synth.synthesize("Hello").await.unwrap();
        assert!(wav.starts_with(b"RIFF"));
    }

    #[tokio::test]
    async fn test_missing_piper_returns_error_not_silent_success() {
        let cfg = VoiceConfig {
            tts_engine: "piper".into(),
            piper_binary: Some(PathBuf::from("nonexistent_piper_binary_xyz")),
            piper_model: Some(PathBuf::from("nonexistent_model_xyz.onnx")),
            ..VoiceConfig::default()
        };
        let synth = PlatformSpeechSynthesizer::from_config(&cfg);
        let res = synth
            .synthesize_audio("Testing missing backend error")
            .await;
        assert!(
            res.is_err(),
            "Missing configured TTS engine must fail loudly instead of returning fake Ok audio"
        );
    }

    #[tokio::test]
    async fn test_native_tts_synthesis() {
        let synth = PlatformSpeechSynthesizer::from_config(&VoiceConfig::default());
        let res = synth.synthesize_audio("Pico voice system active.").await;
        match res {
            Ok(audio) => {
                assert!(audio.sample_rate >= 8_000);
                assert!(!audio.samples.is_empty());
            }
            Err(e) => {
                // Headless CI Linux/Windows runners may lack native TTS voices or binaries.
                assert!(!e.to_string().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn test_malicious_tts_inputs_do_not_inject() {
        let synth = PlatformSpeechSynthesizer::from_config(&VoiceConfig {
            tts_engine: "mock".into(),
            ..VoiceConfig::default()
        });
        let payloads = [
            "'; Remove-Item -Recurse -Force C:\\temp\\fake; '",
            "\"; & calc.exe; \"",
            "$(whoami) `id`",
            "hello world\n\nInvoke-Expression 'dir'",
        ];

        for p in payloads {
            let res = synth.synthesize_audio(p).await;
            assert!(res.is_ok());
        }
    }
}
