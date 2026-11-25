use async_trait::async_trait;
use pico_core::config::VoiceConfig;
use pico_core::error::{PicoError, Result};
use pico_core::traits::SpeechRecognizer;
use regex::Regex;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing::{debug, warn};

/// Cleans transcribed speech: strips utterance-initial hesitation fillers, hyphenated stutters,
/// consecutive duplicates, and acoustic silence hallucinations while preserving operators and numbers.
pub struct SpeechNormalizer;

impl SpeechNormalizer {
    /// Cleans and normalizes raw recognized speech.
    pub fn normalize(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        static LEADING_LIKE_FILLER_RE: OnceLock<Regex> = OnceLock::new();
        static FILLER_RE: OnceLock<Regex> = OnceLock::new();

        // Only remove "like" when used as an utterance-initial filler, preserving "looks like notepad".
        let after_leading = LEADING_LIKE_FILLER_RE
            .get_or_init(|| Regex::new(r"(?i)^\s*like\b(?:\s+(?:uh|um|er|err|ah|hmm)\b)?").unwrap())
            .replace(trimmed, "");

        let filler_re =
            FILLER_RE.get_or_init(|| Regex::new(r"(?i)\b(uh|um|er|err|ah|hmm)\b").unwrap());
        let no_fillers = filler_re.replace_all(&after_leading, "");

        let raw_tokens: Vec<&str> = no_fillers.split_whitespace().collect();
        let mut deduplicated: Vec<String> = Vec::with_capacity(raw_tokens.len());

        for tok in raw_tokens {
            let mut word = tok;
            // Clean stutter prefix: "c-can" -> "can", "o-open" -> "open", while preserving math "-5".
            if let Some((prefix, rest)) = tok.split_once('-') {
                if !prefix.is_empty()
                    && prefix.chars().all(|c| c.is_ascii_alphabetic())
                    && rest.to_lowercase().starts_with(&prefix.to_lowercase())
                {
                    word = rest;
                }
            }

            // Preserve standalone math symbols (+, -, *, /, =, %, ^) and paths/decimals.
            let clean_tok = if word
                .chars()
                .all(|c| matches!(c, '+' | '-' | '*' | '/' | '=' | '%' | '^'))
            {
                word
            } else {
                // Trim only surrounding sentence/quote punctuation, keeping internal dots/slashes.
                word.trim_matches(|c: char| {
                    matches!(
                        c,
                        ',' | ';' | ':' | '"' | '\'' | '(' | ')' | '[' | ']' | '?' | '!'
                    )
                })
                .trim_end_matches('.')
            };

            if clean_tok.is_empty() {
                continue;
            }

            if let Some(last) = deduplicated.last() {
                if clean_tok.eq_ignore_ascii_case(last)
                    && !clean_tok.chars().all(|c| c.is_ascii_digit())
                {
                    continue;
                }
            }

            deduplicated.push(clean_tok.to_string());
        }

        // Collapse adjacent 2-word and 3-word stutters (e.g. "set the set the volume")
        // while preserving numeric sequences (e.g. "10 20 10 20").
        for n in [3usize, 2usize] {
            let mut i = 0;
            let mut next = Vec::with_capacity(deduplicated.len());
            while i < deduplicated.len() {
                if i + 2 * n <= deduplicated.len() {
                    let a = &deduplicated[i..i + n];
                    let b = &deduplicated[i + n..i + 2 * n];
                    let all_digits = a
                        .iter()
                        .all(|t| t.chars().all(|c| c.is_ascii_digit() || c == '.'));
                    if !all_digits
                        && a.iter()
                            .zip(b.iter())
                            .all(|(x, y)| x.eq_ignore_ascii_case(y))
                    {
                        next.extend(a.iter().cloned());
                        i += 2 * n;
                        continue;
                    }
                }
                next.push(deduplicated[i].clone());
                i += 1;
            }
            deduplicated = next;
        }

        let cleaned = deduplicated.join(" ").trim().to_string();
        if cleaned.is_empty() {
            return String::new();
        }

        let lower = cleaned.to_lowercase();
        let stripped = lower.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
        let noise_artifacts = [
            "mm-hmm",
            "mm hmm",
            "mhm",
            "huh",
            "uh huh",
            "uh-huh",
            "thank you",
            "thanks for watching",
            "subscribe",
            "opening day of",
            "silence",
        ];
        if noise_artifacts.contains(&stripped) {
            return String::new();
        }

        cleaned
    }
}

/// RAII temporary WAV file guard ensuring files are always cleaned up on early exit or panic.
struct TempWavGuard(PathBuf);

impl TempWavGuard {
    fn create(wav_bytes: &[u8]) -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("pico_stt_{}.wav", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, wav_bytes).map_err(|e| {
            PicoError::SpeechRecognition(format!("Failed to write temporary WAV file: {}", e))
        })?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWavGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Long-running daemon wrapper for local Python ASR service with bounded response timeouts.
struct AsrDaemon {
    child: Child,
    stdin: ChildStdin,
    line_rx: mpsc::Receiver<std::result::Result<String, String>>,
    _reader_thread: std::thread::JoinHandle<()>,
}

impl AsrDaemon {
    fn find_script(name: &str) -> Option<PathBuf> {
        let candidates = [
            PathBuf::from("scripts").join(name),
            PathBuf::from("../scripts").join(name),
            PathBuf::from("../../scripts").join(name),
        ];
        candidates.into_iter().find(|p| p.exists())
    }

    fn find_python(explicit: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            if p.exists() {
                return Some(p.to_path_buf());
            }
            if !p.to_string_lossy().contains(['/', '\\'])
                && Command::new(p)
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            {
                return Some(p.to_path_buf());
            }
            // Explicit path was provided and is invalid: do not silently mask misconfiguration.
            return None;
        }
        if let Ok(env_py) = std::env::var("PICO_PYTHON") {
            let p = PathBuf::from(env_py);
            if p.exists() {
                return Some(p);
            }
        }
        let venv_candidates = if cfg!(target_os = "windows") {
            vec![".venv/Scripts/python.exe", "../.venv/Scripts/python.exe"]
        } else {
            vec![".venv/bin/python", "../.venv/bin/python"]
        };
        for c in venv_candidates {
            let p = PathBuf::from(c);
            if p.exists() {
                return Some(p);
            }
        }
        for cmd in ["python3", "python"] {
            if let Ok(out) = Command::new(cmd).arg("--version").output() {
                if out.status.success() {
                    return Some(PathBuf::from(cmd));
                }
            }
        }
        None
    }

    fn spawn(engine: &str, python_bin: Option<&Path>) -> std::result::Result<Self, String> {
        let script = Self::find_script("asr_service.py")
            .ok_or_else(|| "scripts/asr_service.py not found".to_string())?;
        let py = Self::find_python(python_bin)
            .ok_or_else(|| "No working Python interpreter found for ASR daemon".to_string())?;

        let mut child = Command::new(&py)
            .args([
                script.to_str().unwrap_or("scripts/asr_service.py"),
                "--daemon",
                "--engine",
                engine,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed spawning ASR daemon ({}): {}", py.display(), e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ASR daemon stdin missing".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "ASR daemon stdout missing".to_string())?;
        let stderr = child.stderr.take();

        let (line_tx, line_rx) = mpsc::channel();
        let reader_thread = std::thread::Builder::new()
            .name("pico-asr-stdout".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            let err_detail = stderr
                                .and_then(|mut s| {
                                    let mut buf = String::new();
                                    use std::io::Read;
                                    let _ = s.read_to_string(&mut buf);
                                    let trimmed = buf.trim().to_string();
                                    if trimmed.is_empty() {
                                        None
                                    } else {
                                        Some(trimmed)
                                    }
                                })
                                .unwrap_or_else(|| "ASR stdout EOF".into());
                            let _ = line_tx.send(Err(err_detail));
                            break;
                        }
                        Ok(_) => {
                            let _ = line_tx.send(Ok(line));
                        }
                        Err(e) => {
                            let _ = line_tx.send(Err(format!("ASR stdout read error: {}", e)));
                            break;
                        }
                    }
                }
            })
            .map_err(|e| format!("Failed spawning ASR reader thread: {}", e))?;

        match line_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(ready_line)) if ready_line.starts_with("READY:") => Ok(Self {
                child,
                stdin,
                line_rx,
                _reader_thread: reader_thread,
            }),
            Ok(Ok(other)) => {
                let _ = child.kill();
                Err(format!(
                    "ASR daemon failed readiness handshake: {}",
                    other.trim()
                ))
            }
            Ok(Err(err)) => {
                let _ = child.kill();
                Err(format!("ASR daemon exited during startup: {}", err))
            }
            Err(_) => {
                let _ = child.kill();
                Err("ASR daemon startup timed out after 20s".into())
            }
        }
    }

    fn transcribe(&mut self, wav_path: &Path) -> std::result::Result<String, String> {
        let path_str = wav_path
            .to_str()
            .ok_or_else(|| "Non-UTF8 temp WAV path".to_string())?;
        writeln!(self.stdin, "{}", path_str)
            .map_err(|e| format!("ASR daemon stdin write failed: {}", e))?;
        self.stdin
            .flush()
            .map_err(|e| format!("ASR daemon flush failed: {}", e))?;

        match self.line_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(line)) => {
                let trimmed = line.trim().to_string();
                if let Some(err) = trimmed.strip_prefix("ERROR:") {
                    return Err(err.trim().to_string());
                }
                Ok(trimmed)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("ASR daemon inference timed out after 15s".into()),
        }
    }
}

impl Drop for AsrDaemon {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "QUIT");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Cross-platform Speech Recognition Engine supporting ASR daemon, whisper.cpp CLI, and Mock mode.
pub struct PlatformSpeechRecognizer {
    stt_engine: String,
    whisper_binary: Option<PathBuf>,
    whisper_model: Option<PathBuf>,
    python_binary: Option<PathBuf>,
    explicit_python: Option<PathBuf>,
    daemon: Arc<Mutex<Option<AsrDaemon>>>,
}

pub type WindowsSpeechRecognizer = PlatformSpeechRecognizer;

impl PlatformSpeechRecognizer {
    pub fn new(whisper_binary: Option<PathBuf>) -> Self {
        let whisper_model = Self::discover_whisper_model(None);
        Self {
            stt_engine: "whisper".into(),
            whisper_binary,
            whisper_model,
            python_binary: None,
            explicit_python: None,
            daemon: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_config(config: &VoiceConfig) -> Self {
        let whisper_binary = config
            .whisper_binary
            .clone()
            .filter(|p| p.exists())
            .or_else(|| Self::discover_whisper_binary(&config.stt_engine));
        let whisper_model = Self::discover_whisper_model(config.whisper_model.as_deref());
        let explicit_python = config.python_binary.clone();
        let python_binary = AsrDaemon::find_python(explicit_python.as_deref());

        let daemon = if !config.stt_engine.eq_ignore_ascii_case("mock")
            && !config.stt_engine.eq_ignore_ascii_case("whisper")
            && (explicit_python.is_none() || python_binary.is_some())
        {
            match AsrDaemon::spawn(&config.stt_engine, python_binary.as_deref()) {
                Ok(d) => Some(d),
                Err(e) => {
                    debug!(error = %e, engine = %config.stt_engine, "Deferred ASR daemon spawn");
                    None
                }
            }
        } else {
            None
        };

        Self {
            stt_engine: config.stt_engine.clone(),
            whisper_binary,
            whisper_model,
            python_binary,
            explicit_python,
            daemon: Arc::new(Mutex::new(daemon)),
        }
    }

    fn discover_whisper_binary(engine: &str) -> Option<PathBuf> {
        if let Ok(env_bin) = std::env::var("PICO_WHISPER_BIN") {
            let p = PathBuf::from(env_bin);
            if p.exists() {
                return Some(p);
            }
        }
        let _ = engine;
        let candidates = [
            "models/whisper/whisper-cli.exe",
            "models/whisper/whisper-cli",
            "whisper-cli.exe",
            "whisper-cli",
            "whisper.exe",
        ];
        for c in candidates {
            let p = PathBuf::from(c);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    fn discover_whisper_model(explicit: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }
        if let Ok(env_model) = std::env::var("PICO_WHISPER_MODEL") {
            let p = PathBuf::from(env_model);
            if p.exists() {
                return Some(p);
            }
        }
        for name in [
            "models/whisper/ggml-base.en.bin",
            "models/whisper/ggml-small.en.bin",
            "models/whisper/ggml-tiny.en.bin",
        ] {
            let p = PathBuf::from(name);
            if p.exists() {
                return Some(p);
            }
        }
        if let Ok(entries) = std::fs::read_dir("models/whisper") {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("bin") {
                    return Some(p);
                }
            }
        }
        None
    }

    pub fn stt_engine(&self) -> &str {
        &self.stt_engine
    }

    pub fn whisper_binary(&self) -> Option<&Path> {
        self.whisper_binary.as_deref()
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

    fn transcribe_asr_service(
        wav_path: &Path,
        engine: &str,
        python_bin: Option<&Path>,
    ) -> std::result::Result<String, String> {
        let script = AsrDaemon::find_script("asr_service.py")
            .ok_or_else(|| "scripts/asr_service.py missing".to_string())?;
        let py = AsrDaemon::find_python(python_bin)
            .ok_or_else(|| "No Python interpreter found".to_string())?;

        let output = Command::new(py)
            .args([
                script.to_str().unwrap_or("scripts/asr_service.py"),
                wav_path.to_str().unwrap_or(""),
                "--engine",
                engine,
            ])
            .output()
            .map_err(|e| format!("ASR script spawn failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success()
            || stdout.starts_with("INIT_ERROR:")
            || stdout.starts_with("ERROR:")
        {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "ASR service failed ({}): {} {}",
                output.status, stdout, stderr
            ));
        }
        Ok(stdout)
    }

    fn transcribe_whisper_cli(
        binary: &Path,
        model: Option<&Path>,
        wav_path: &Path,
    ) -> std::result::Result<String, String> {
        let mut cmd = Command::new(binary);
        if let Some(m) = model {
            cmd.args(["-m", m.to_str().unwrap_or("")]);
        }
        cmd.args(["-f", wav_path.to_str().unwrap_or(""), "-nt"]);
        let output = cmd
            .output()
            .map_err(|e| format!("Failed invoking whisper CLI {}: {}", binary.display(), e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("whisper CLI exited {}: {}", output.status, err));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait]
impl SpeechRecognizer for PlatformSpeechRecognizer {
    async fn transcribe(&self, pcm_samples: &[i16]) -> Result<String> {
        if pcm_samples.is_empty() {
            return Ok(String::new());
        }

        if self.stt_engine.eq_ignore_ascii_case("mock") {
            return Ok(String::new());
        }

        let wav_bytes = Self::samples_to_wav_bytes(pcm_samples)?;
        let whisper_opt = self.whisper_binary.clone();
        let whisper_model = self.whisper_model.clone();
        let python_target = self
            .explicit_python
            .clone()
            .or_else(|| self.python_binary.clone());
        let engine_name = self.stt_engine.clone();
        let daemon_arc = self.daemon.clone();

        let raw_result = tokio::task::spawn_blocking(move || -> Result<String> {
            let temp_wav = TempWavGuard::create(&wav_bytes)?;
            let mut errors: Vec<String> = Vec::new();

            // 1. Primary: Persistent ASR daemon when configured for local ONNX/Python engines.
            if !engine_name.eq_ignore_ascii_case("whisper") || whisper_opt.is_none() {
                let mut guard = daemon_arc
                    .lock()
                    .map_err(|_| PicoError::SpeechRecognition("ASR daemon lock poisoned".into()))?;
                if guard.is_none() {
                    match AsrDaemon::spawn(&engine_name, python_target.as_deref()) {
                        Ok(d) => *guard = Some(d),
                        Err(e) => errors.push(e),
                    }
                }
                if let Some(ref mut d) = *guard {
                    match d.transcribe(temp_wav.path()) {
                        Ok(t) => return Ok(t),
                        Err(e) => {
                            warn!(error = %e, "ASR daemon inference failed; trying recovery");
                            errors.push(e);
                            *guard = None;
                        }
                    }
                }
            }

            // 2. Secondary: Whisper CLI when configured/available.
            if let Some(ref binary) = whisper_opt {
                if binary.exists() {
                    match Self::transcribe_whisper_cli(
                        binary,
                        whisper_model.as_deref(),
                        temp_wav.path(),
                    ) {
                        Ok(t) => return Ok(t),
                        Err(e) => errors.push(e),
                    }
                }
            }

            // 3. Tertiary: One-shot ASR script execution.
            match Self::transcribe_asr_service(
                temp_wav.path(),
                &engine_name,
                python_target.as_deref(),
            ) {
                Ok(t) => return Ok(t),
                Err(e) => errors.push(e),
            }

            Err(PicoError::SpeechRecognition(format!(
                "No working speech-to-text backend succeeded for engine '{}': {}",
                engine_name,
                errors.join(" | ")
            )))
        })
        .await
        .map_err(|e| PicoError::SpeechRecognition(format!("STT task panicked: {}", e)))??;

        Ok(SpeechNormalizer::normalize(&raw_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_conversion() {
        let samples = vec![0i16; 1600]; // 100ms silence
        let wav = PlatformSpeechRecognizer::samples_to_wav_bytes(&samples).unwrap();
        assert!(wav.starts_with(b"RIFF"));
        assert!(wav.len() > 3200);
    }

    #[test]
    fn test_speech_normalizer_stutter_and_fillers() {
        let dirty = "c-can you o-open chrome chrome please";
        let cleaned = SpeechNormalizer::normalize(dirty);
        assert_eq!(cleaned, "can you open chrome please");

        let stutter = "p-pico w-what time is it it";
        let cleaned_stutter = SpeechNormalizer::normalize(stutter);
        assert_eq!(cleaned_stutter, "pico what time is it");

        let multi_word = "set the set the volume to 50";
        assert_eq!(
            SpeechNormalizer::normalize(multi_word),
            "set the volume to 50"
        );

        let three_word = "can you please can you please open notepad";
        assert_eq!(
            SpeechNormalizer::normalize(three_word),
            "can you please open notepad"
        );

        let numeric_seq = "10 20 10 20";
        assert_eq!(SpeechNormalizer::normalize(numeric_seq), "10 20 10 20");

        let fillers = "like um can you err calculate 25 plus 10";
        let cleaned_fillers = SpeechNormalizer::normalize(fillers);
        assert_eq!(cleaned_fillers, "can you calculate 25 plus 10");
    }

    #[test]
    fn test_speech_normalizer_preserves_meaningful_like_math_and_paths() {
        assert_eq!(
            SpeechNormalizer::normalize("open something like notepad."),
            "open something like notepad"
        );
        assert_eq!(
            SpeechNormalizer::normalize("calculate 3.14 * 2 + 5?"),
            "calculate 3.14 * 2 + 5"
        );
        assert_eq!(
            SpeechNormalizer::normalize("ping network host 8.8.8.8!"),
            "ping network host 8.8.8.8"
        );
        assert_eq!(
            SpeechNormalizer::normalize("read /etc/hosts please."),
            "read /etc/hosts please"
        );
    }

    #[test]
    fn test_speech_normalizer_trailing_punctuation() {
        let dirty = "open browser.";
        let cleaned = SpeechNormalizer::normalize(dirty);
        assert_eq!(cleaned, "open browser");

        let dirty2 = "check cpu usage, please!";
        let cleaned2 = SpeechNormalizer::normalize(dirty2);
        assert_eq!(cleaned2, "check cpu usage please");
    }

    #[tokio::test]
    async fn test_mock_stt_returns_clean_empty_and_missing_backend_errors() {
        let mock = PlatformSpeechRecognizer::from_config(&VoiceConfig {
            stt_engine: "mock".into(),
            ..VoiceConfig::default()
        });
        assert!(mock.transcribe(&[1000i16; 1600]).await.unwrap().is_empty());

        let broken = PlatformSpeechRecognizer {
            stt_engine: "whisper".into(),
            whisper_binary: Some(PathBuf::from("nonexistent_whisper_bin_xyz")),
            whisper_model: None,
            python_binary: None,
            explicit_python: Some(PathBuf::from("nonexistent_python_xyz")),
            daemon: Arc::new(Mutex::new(None)),
        };
        let err = broken.transcribe(&[1000i16; 1600]).await;
        assert!(err.is_err());
    }
}
