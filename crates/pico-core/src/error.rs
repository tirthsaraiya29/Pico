use thiserror::Error;

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
