use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Machine-readable error codes for IPC, diagnostics, and retry classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PicoErrorCode {
    PicoConfigError,
    PicoInferenceError,
    PicoSecurityViolation,
    PicoToolExecution,
    PicoToolNotFound,
    PicoPermissionDenied,
    PicoAudioError,
    PicoSttError,
    PicoTtsError,
    PicoDatabaseError,
    PicoNotFound,
    PicoMemoryError,
    PicoPlatformError,
    PicoSerializationError,
    PicoTimeout,
    PicoCancelled,
    PicoInternalError,
}

impl PicoErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PicoConfigError => "PICO_CONFIG_ERROR",
            Self::PicoInferenceError => "PICO_INFERENCE_ERROR",
            Self::PicoSecurityViolation => "PICO_SECURITY_VIOLATION",
            Self::PicoToolExecution => "PICO_TOOL_EXECUTION",
            Self::PicoToolNotFound => "PICO_TOOL_NOT_FOUND",
            Self::PicoPermissionDenied => "PICO_PERMISSION_DENIED",
            Self::PicoAudioError => "PICO_AUDIO_ERROR",
            Self::PicoSttError => "PICO_STT_ERROR",
            Self::PicoTtsError => "PICO_TTS_ERROR",
            Self::PicoDatabaseError => "PICO_DATABASE_ERROR",
            Self::PicoNotFound => "PICO_NOT_FOUND",
            Self::PicoMemoryError => "PICO_MEMORY_ERROR",
            Self::PicoPlatformError => "PICO_PLATFORM_ERROR",
            Self::PicoSerializationError => "PICO_SERIALIZATION_ERROR",
            Self::PicoTimeout => "PICO_TIMEOUT",
            Self::PicoCancelled => "PICO_CANCELLED",
            Self::PicoInternalError => "PICO_INTERNAL_ERROR",
        }
    }
}

impl std::fmt::Display for PicoErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Universal error type for the Pico assistant platform.
#[derive(Error, Debug)]
pub enum PicoError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Inference provider error: {0}")]
    Inference(String),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Tool execution error in '{tool}': {message}")]
    ToolExecution { tool: String, message: String },

    #[error("Tool not found: '{0}'")]
    ToolNotFound(String),

    #[error("Permission denied for '{action}': {reason}")]
    PermissionDenied { action: String, reason: String },

    #[error("Audio device error: {0}")]
    Audio(String),

    #[error("Speech recognition error: {0}")]
    SpeechRecognition(String),

    #[error("Speech synthesis error: {0}")]
    SpeechSynthesis(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Memory storage error: {0}")]
    Memory(String),

    #[error("Platform integration error: {0}")]
    Platform(String),

    #[error("Serialization/deserialization failure: {0}")]
    Serialization(String),

    #[error("Operation timed out after {0} ms")]
    Timeout(u64),

    #[error("Operation cancelled: {0}")]
    Cancelled(String),

    #[error("Internal runtime error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, PicoError>;

impl PicoError {
    /// Returns the structured machine-readable error code for this error.
    pub const fn code(&self) -> PicoErrorCode {
        match self {
            Self::Config(_) => PicoErrorCode::PicoConfigError,
            Self::Inference(_) => PicoErrorCode::PicoInferenceError,
            Self::Security(_) => PicoErrorCode::PicoSecurityViolation,
            Self::ToolExecution { .. } => PicoErrorCode::PicoToolExecution,
            Self::ToolNotFound(_) => PicoErrorCode::PicoToolNotFound,
            Self::PermissionDenied { .. } => PicoErrorCode::PicoPermissionDenied,
            Self::Audio(_) => PicoErrorCode::PicoAudioError,
            Self::SpeechRecognition(_) => PicoErrorCode::PicoSttError,
            Self::SpeechSynthesis(_) => PicoErrorCode::PicoTtsError,
            Self::Database(_) => PicoErrorCode::PicoDatabaseError,
            Self::NotFound(_) => PicoErrorCode::PicoNotFound,
            Self::Memory(_) => PicoErrorCode::PicoMemoryError,
            Self::Platform(_) => PicoErrorCode::PicoPlatformError,
            Self::Serialization(_) => PicoErrorCode::PicoSerializationError,
            Self::Timeout(_) => PicoErrorCode::PicoTimeout,
            Self::Cancelled(_) => PicoErrorCode::PicoCancelled,
            Self::Internal(_) => PicoErrorCode::PicoInternalError,
        }
    }

    /// Returns the uppercase string representation of the machine-readable error code.
    pub const fn code_str(&self) -> &'static str {
        self.code().as_str()
    }
}

impl From<serde_json::Error> for PicoError {
    fn from(err: serde_json::Error) -> Self {
        PicoError::Serialization(err.to_string())
    }
}

impl From<toml::de::Error> for PicoError {
    fn from(err: toml::de::Error) -> Self {
        PicoError::Config(err.to_string())
    }
}

impl From<std::io::Error> for PicoError {
    fn from(err: std::io::Error) -> Self {
        PicoError::Internal(err.to_string())
    }
}
