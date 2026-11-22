pub mod config;
pub mod error;
pub mod events;
pub mod session;
pub mod traits;

pub use config::{
    AudioConfig, DesktopConfig, InferenceConfig, MemoryConfig, PicoConfig, SecurityConfig,
    VoiceConfig,
};
pub use error::{PicoError, Result};
pub use events::PicoEvent;
pub use session::{Message, Role, SessionId, ToolCall, ToolResult};
pub use traits::{
    InferenceProvider, InferenceResponse, MemoryStore, PermissionTier, SpeechRecognizer,
    SpeechSynthesizer, TokenStream, Tool,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_id_generation() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        assert_ne!(id1, id2);
        assert!(!id1.to_string().is_empty());
    }

    #[test]
    fn test_message_creation() {
        let sid = SessionId::new();
        let msg = Message::new_user(sid, "Hello Pico");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello Pico");
        assert_eq!(msg.session_id, sid);
    }

    #[test]
    fn test_config_defaults() {
        let config = PicoConfig::default();
        assert_eq!(config.inference.provider, "ollama");
        assert_eq!(config.audio.sample_rate, 16000);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_event_serialization() {
        let event = PicoEvent::VoiceActivityStarted {
            timestamp: chrono::Utc::now(),
        };
        let serialized = serde_json::to_string(&event).expect("Serialization failed");
        let deserialized: PicoEvent =
            serde_json::from_str(&serialized).expect("Deserialization failed");
        assert_eq!(event, deserialized);
    }
}
