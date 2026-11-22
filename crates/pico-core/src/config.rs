use crate::error::{PicoError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Root configuration structure for Pico.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PicoConfig {
    #[serde(default)]
    pub inference: InferenceConfig,

    #[serde(default)]
    pub voice: VoiceConfig,

    #[serde(default)]
    pub audio: AudioConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub memory: MemoryConfig,

    #[serde(default)]
    pub desktop: DesktopConfig,
}

impl PicoConfig {
    /// Loads configuration from a TOML file on disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            PicoError::Config(format!(
                "Failed to read config file {}: {}",
                path.as_ref().display(),
                e
            ))
        })?;
        let config: Self = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validates configuration parameters.
    pub fn validate(&self) -> Result<()> {
        if self.inference.endpoint.is_empty() {
            return Err(PicoError::Config(
                "Inference endpoint cannot be empty".into(),
            ));
        }
        if self.inference.model.is_empty() {
            return Err(PicoError::Config(
                "Inference model name cannot be empty".into(),
            ));
        }
        if self.audio.sample_rate == 0 {
            return Err(PicoError::Config(
                "Audio sample rate must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

/// Inference engine configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Provider type: "ollama", "llamacpp", or "openai"
    pub provider: String,
    /// Base URL endpoint (e.g. "http://127.0.0.1:11434")
    pub endpoint: String,
    /// Model identifier (e.g. "granite4.2:latest", "gpt-oss:20b-cloud")
    pub model: String,
    /// Optional API key for remote providers
    pub api_key: Option<String>,
    /// Sampling temperature (0.0 to 1.0)
    pub temperature: f32,
    /// Maximum context window size in tokens
    pub context_window: usize,
    /// Request timeout in seconds
    pub timeout_seconds: u64,
    /// System prompt persona
    pub system_prompt: String,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "granite4.2:latest".into(),
            api_key: None,
            temperature: 0.2,
            context_window: 8192,
            timeout_seconds: 60,
            system_prompt: "You are Pico, an ultra-fast, capable, local AI voice assistant. Answer concisely, directly, and naturally. When tools are necessary to complete a user task, select and call them directly.".into(),
        }
    }
}

/// Real-time voice engine configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// VAD sensitivity energy threshold (0.001 to 0.1)
    pub vad_threshold: f32,
    /// Silence duration in ms required to trigger speech end
    pub silence_duration_ms: u64,
    /// Speech-to-text engine: "whisper", "local", or "cloud"
    pub stt_engine: String,
    /// Text-to-speech engine: "native_sapi", "piper", or "cloud"
    pub tts_engine: String,
    /// TTS Voice identifier or name
    pub voice_name: Option<String>,
    /// Speech playback rate multiplier (0.5 to 2.0)
    pub speech_rate: f32,
    /// Enable push-to-talk mode rather than continuous VAD
    pub push_to_talk: bool,
    /// Keyboard key code for push-to-talk trigger
    pub ptt_key: String,
    /// Enable barge-in (interrupt playback when user speaks)
    pub enable_barge_in: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            vad_threshold: 0.015,
            silence_duration_ms: 700,
            stt_engine: "whisper".into(),
            tts_engine: "native_sapi".into(),
            voice_name: None,
            speech_rate: 1.0,
            push_to_talk: false,
            ptt_key: "Space".into(),
            enable_barge_in: true,
        }
    }
}

/// Low-level audio hardware configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Audio sampling rate in Hertz (standard 16000 for voice)
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono)
    pub channels: u16,
    /// Specific input audio device name (or default if None)
    pub input_device: Option<String>,
    /// Specific output audio device name (or default if None)
    pub output_device: Option<String>,
    /// Audio buffer frame chunk size
    pub buffer_size: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            input_device: None,
            output_device: None,
            buffer_size: 512,
        }
    }
}

/// Security and permission settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Default execution permission policy: "Strict", "ConfirmationPrompt", "Permissive"
    pub policy_mode: String,
    /// Directories allowed for file read/write operations
    pub allowed_directories: Vec<PathBuf>,
    /// Explicitly forbidden system paths
    pub blocked_directories: Vec<PathBuf>,
    /// Always prompt for confirmation before executing shell commands
    pub require_confirmation_for_shell: bool,
    /// Path to security audit log
    pub audit_log_path: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            policy_mode: "ConfirmationPrompt".into(),
            allowed_directories: vec![PathBuf::from(".")],
            blocked_directories: vec![
                PathBuf::from("C:\\Windows\\System32"),
                PathBuf::from("C:\\Windows"),
            ],
            require_confirmation_for_shell: true,
            audit_log_path: PathBuf::from("pico_audit.log"),
        }
    }
}

/// Long-term memory and context budgeting settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Path to SQLite database file
    pub database_path: PathBuf,
    /// Maximum tokens reserved for active conversation history
    pub max_history_tokens: usize,
    /// Total context window budget
    pub context_budget: usize,
    /// Number of turns before generating a summary
    pub auto_summarize_turns: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            database_path: PathBuf::from("pico_memory.sqlite"),
            max_history_tokens: 4096,
            context_budget: 8192,
            auto_summarize_turns: 12,
        }
    }
}

/// Local web/desktop interface server settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Local HTTP/WebSocket server port
    pub port: u16,
    /// Local address to bind (defaults to 127.0.0.1 for security)
    pub bind_address: String,
    /// Auto-launch browser on start
    pub auto_open_browser: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            port: 3124,
            bind_address: "127.0.0.1".into(),
            auto_open_browser: true,
        }
    }
}
