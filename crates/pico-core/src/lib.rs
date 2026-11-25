pub mod config;
pub mod error;
pub mod events;
pub mod ipc;
pub mod session;
pub mod traits;

pub use config::{
    AudioConfig, DecisionConfig, DesktopConfig, LearningConfig, MemoryConfig, PicoConfig,
    SecurityConfig, VoiceConfig,
};
pub use error::{PicoError, Result};
pub use events::PicoEvent;
pub use ipc::{
    decode_ipc_envelope, default_ipc_endpoint, default_ipc_token_path, encode_ipc_line,
    read_ipc_token, send_ipc_request_sync, ClarificationView, DaemonSnapshot, IpcEnvelope,
    IpcRequest, IpcResponse, PendingConfirmationView, IPC_PROTOCOL_VERSION, MAX_IPC_FRAME_BYTES,
};
pub use session::{
    ClarificationKind, ClarificationOption, ClarificationState, Message, Role, SessionId, ToolCall,
    ToolResult,
};
pub use traits::{
    AudioFormat, AudioSampleEncoding, CandidateAction, CandidateScore, DecisionModel, MemoryStore,
    PermissionTier, SpeechRecognizer, SpeechSynthesizer, SynthesizedAudio, Tool, VoiceInfo,
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
        assert_eq!(config.decision.model_version, "pico-decision-v1.0.0");
        assert_eq!(config.audio.sample_rate, 16000);
        assert!(config.learning.enabled);
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
