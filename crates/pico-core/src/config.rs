use crate::error::{PicoError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Root configuration structure for Pico.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PicoConfig {
    #[serde(default)]
    pub decision: DecisionConfig,

    #[serde(default)]
    pub learning: LearningConfig,

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
        if self.decision.confidence_threshold <= 0.0 || self.decision.confidence_threshold > 1.0 {
            return Err(PicoError::Config(
                "Decision confidence threshold must be between 0.0 and 1.0".into(),
            ));
        }
        if self.decision.max_candidates == 0 {
            return Err(PicoError::Config(
                "Decision max candidates must be greater than zero".into(),
            ));
        }
        if self.audio.sample_rate == 0 {
            return Err(PicoError::Config(
                "Audio sample rate must be greater than zero".into(),
            ));
        }
        self.voice.validate()?;
        Ok(())
    }
}

fn default_clarification_timeout() -> u64 {
    120
}

/// Probabilistic Decision Model configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionConfig {
    /// Active decision model version
    pub model_version: String,
    /// Minimum calibrated probability threshold to automatically execute an action
    pub confidence_threshold: f32,
    /// Maximum candidate tools retrieved for ranking evaluation
    pub max_candidates: usize,
    /// Whether multi-feature ensemble candidate reranking is active
    pub enable_cross_encoder: bool,
    /// Temperature scaling factor for probability calibration
    pub calibration_temperature: f32,
    /// Margin between top 2 candidate probabilities below which clarification is requested
    pub clarification_margin: f32,
    /// Seconds before an unanswered session clarification prompt expires
    #[serde(default = "default_clarification_timeout")]
    pub clarification_timeout_seconds: u64,
    /// Path to model artifacts directory
    pub model_dir: PathBuf,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            model_version: "pico-decision-v1.0.0".into(),
            confidence_threshold: 0.55,
            max_candidates: 15,
            enable_cross_encoder: true,
            calibration_temperature: 1.0,
            clarification_margin: 0.08,
            clarification_timeout_seconds: default_clarification_timeout(),
            model_dir: PathBuf::from("models/decision"),
        }
    }
}

/// Continuous on-device learning & experience dataset configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Whether continuous experience collection is active
    pub enabled: bool,
    /// SQLite database path for persistent structured experiences
    pub database_path: PathBuf,
    /// Enable automatic retraining on idle when sufficient eligible experiences accumulate
    pub auto_train: bool,
    /// Minimum newly validated observations required to trigger candidate retraining
    pub min_observations_for_retraining: usize,
    /// Maximum retained observations in the experience database
    pub max_dataset_entries: usize,
    /// Fraction of observations reserved for held-out promotion evaluation
    pub evaluation_heldout_ratio: f32,
    /// Minimum Top-1 evaluation accuracy required to promote a newly trained candidate model
    pub min_promotion_accuracy: f32,
    /// Step size learning rate for gradient optimization
    pub learning_rate: f32,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            database_path: PathBuf::from("data/experiences.sqlite"),
            auto_train: false,
            min_observations_for_retraining: 25,
            max_dataset_entries: 50_000,
            evaluation_heldout_ratio: 0.20,
            min_promotion_accuracy: 0.85,
            learning_rate: 0.05,
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
    /// Speech-to-text engine: "auto", "parakeet", "moonshine", "tdt", "whisper", "local", or "mock"
    pub stt_engine: String,
    /// Text-to-speech engine: "auto", "winrt", "native_sapi", "piper", "speech_dispatcher", "espeak", "system", or "mock"
    pub tts_engine: String,
    /// TTS Voice identifier or name
    pub voice_name: Option<String>,
    /// Speech playback rate multiplier (0.5 to 2.0)
    pub speech_rate: f32,
    /// Optional explicit path to the Piper CLI executable
    #[serde(default)]
    pub piper_binary: Option<PathBuf>,
    /// Optional explicit path to the Piper `.onnx` voice model
    #[serde(default)]
    pub piper_model: Option<PathBuf>,
    /// Optional explicit path to the whisper.cpp / whisper executable
    #[serde(default)]
    pub whisper_binary: Option<PathBuf>,
    /// Optional explicit path to the Whisper `.bin` / ONNX model
    #[serde(default)]
    pub whisper_model: Option<PathBuf>,
    /// Optional explicit path to the Python interpreter for local ASR scripts
    #[serde(default)]
    pub python_binary: Option<PathBuf>,
    /// Enable push-to-talk mode rather than continuous VAD
    pub push_to_talk: bool,
    /// Keyboard key code for push-to-talk trigger
    pub ptt_key: String,
    /// Enable barge-in (interrupt playback when user speaks)
    pub enable_barge_in: bool,
}

impl VoiceConfig {
    pub fn validate(&self) -> Result<()> {
        let valid_stt = [
            "auto",
            "parakeet",
            "moonshine",
            "tdt",
            "whisper",
            "local",
            "system",
            "native_sapi",
            "mock",
        ];
        if !valid_stt.contains(&self.stt_engine.to_lowercase().as_str()) {
            return Err(PicoError::Config(format!(
                "Invalid stt_engine '{}': supported engines are {:?}",
                self.stt_engine, valid_stt
            )));
        }

        let valid_tts = [
            "auto",
            "winrt",
            "native_sapi",
            "piper",
            "speech_dispatcher",
            "espeak",
            "system",
            "mock",
        ];
        if !valid_tts.contains(&self.tts_engine.to_lowercase().as_str()) {
            return Err(PicoError::Config(format!(
                "Invalid tts_engine '{}': supported engines are {:?}",
                self.tts_engine, valid_tts
            )));
        }

        if !(0.5..=2.5).contains(&self.speech_rate) {
            return Err(PicoError::Config(format!(
                "speech_rate must be between 0.5 and 2.5, got {}",
                self.speech_rate
            )));
        }

        if !(0.0001..=0.5).contains(&self.vad_threshold) {
            return Err(PicoError::Config(format!(
                "vad_threshold must be between 0.0001 and 0.5, got {}",
                self.vad_threshold
            )));
        }

        if !(50..=10000).contains(&self.silence_duration_ms) {
            return Err(PicoError::Config(format!(
                "silence_duration_ms must be between 50 and 10000, got {}",
                self.silence_duration_ms
            )));
        }

        Ok(())
    }
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            vad_threshold: 0.015,
            silence_duration_ms: 700,
            stt_engine: "auto".into(),
            tts_engine: "auto".into(),
            voice_name: None,
            speech_rate: 1.0,
            piper_binary: None,
            piper_model: None,
            whisper_binary: None,
            whisper_model: None,
            python_binary: None,
            push_to_talk: false,
            ptt_key: "Space".into(),
            enable_barge_in: true,
        }
    }
}

/// Low-level audio hardware configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Target sample rate (Hz) for speech processing pipeline
    pub sample_rate: u32,
    /// Target channels: 1 = mono, 2 = stereo
    pub channels: u16,
    /// Internal audio chunk frame size (e.g. 512 for 32ms at 16kHz)
    pub chunk_size: usize,
    /// Hardware input device name override (None = OS default)
    pub input_device: Option<String>,
    /// Hardware output device name override (None = OS default)
    pub output_device: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            chunk_size: 512,
            input_device: None,
            output_device: None,
        }
    }
}

/// Zero-trust system security & sandboxing configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Policy enforcement mode: "Strict" or "Permissive"
    pub policy_mode: String,
    /// Require explicit interactive confirmation for high-risk operations
    pub require_confirmation_for_shell: bool,
    /// Whitelisted directories permitted for file operations
    pub allowed_directories: Vec<PathBuf>,
    /// Explicitly forbidden system directories
    pub blocked_directories: Vec<PathBuf>,
    /// Maximum execution time for tools in seconds
    pub command_timeout_seconds: u64,
    /// Audit log file path
    pub audit_log_path: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            policy_mode: "Strict".into(),
            require_confirmation_for_shell: true,
            allowed_directories: vec![PathBuf::from(".")],
            blocked_directories: vec![
                PathBuf::from("C:\\Windows\\System32"),
                PathBuf::from("C:\\Windows\\SysWOW64"),
                PathBuf::from("/etc"),
                PathBuf::from("/bin"),
            ],
            command_timeout_seconds: 30,
            audit_log_path: PathBuf::from("logs/security_audit.jsonl"),
        }
    }
}

/// SQLite persistent conversational memory configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// SQLite file location
    pub database_path: PathBuf,
    /// Maximum historical conversation messages budgeted for context
    pub max_history_messages: usize,
    /// Maximum tokens reserved for active conversation history
    pub max_history_tokens: usize,
    /// Maximum context token capacity
    pub token_budget: usize,
    /// Recency decay half-life in days
    pub decay_half_life_days: f32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            database_path: PathBuf::from("data/pico_memory.sqlite"),
            max_history_messages: 50,
            max_history_tokens: 4096,
            token_budget: 4096,
            decay_half_life_days: 14.0,
        }
    }
}

/// Desktop and local Web interface configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Local HTTP/WebSocket server port
    pub port: u16,
    /// Server bind address
    pub bind_address: String,
    /// Automatically open default browser on launch
    pub auto_open_browser: bool,
    /// Theme preference: "dark" or "light"
    pub theme: String,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            port: 3124,
            bind_address: "127.0.0.1".into(),
            auto_open_browser: true,
            theme: "dark".into(),
        }
    }
}
