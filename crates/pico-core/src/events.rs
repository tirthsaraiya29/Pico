use crate::session::{SessionId, ToolCall, ToolResult};
use crate::traits::CandidateScore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// High-resolution system events dispatched across Pico's runtime bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum PicoEvent {
    // Session Life Cycle
    SessionCreated {
        session_id: SessionId,
    },
    SessionCleared {
        session_id: SessionId,
    },

    // Audio & Voice Activity
    AudioDeviceChanged {
        device_name: String,
        is_input: bool,
    },
    VoiceBackendStatus {
        stt_engine: String,
        stt_ready: bool,
        tts_engine: String,
        tts_ready: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stt_error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tts_error: Option<String>,
    },
    VoiceActivityStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        timestamp: DateTime<Utc>,
    },
    VoiceActivityStopped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        timestamp: DateTime<Utc>,
        duration_ms: u64,
    },

    // Speech-to-Text
    TranscriptionPartial {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        partial_text: String,
    },
    TranscriptionFinalized {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        text: String,
        latency_ms: u64,
    },

    // Probabilistic Decision Intelligence
    DecisionStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        request: String,
        candidate_count: usize,
    },
    CandidatesRanked {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        candidates: Vec<CandidateScore>,
    },
    DecisionSelected {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        action: Option<String>,
        confidence: f32,
        latency_ms: u64,
    },
    ClarificationRequested {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        question: String,
        candidates: Vec<String>,
    },
    ClarificationResolved {
        session_id: SessionId,
        selected_action: Option<String>,
        cancelled: bool,
    },

    // Continuous Learning & Experience
    ExperienceObserved {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        observation_id: String,
        action: String,
        success: bool,
    },
    ModelTrainingStarted {
        version: String,
        eligible_observations: usize,
    },
    ModelTrainingCompleted {
        version: String,
        top1_accuracy: f32,
        top3_accuracy: f32,
        promoted: bool,
    },
    ModelRollbackOccurred {
        target_version: String,
    },

    // Structured Tools
    ToolCallRequested {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        call: ToolCall,
    },
    ToolExecutionStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        call_id: String,
        tool: String,
    },
    ToolExecutionFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        result: ToolResult,
    },
    ObjectiveVerificationCompleted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        tool: String,
        verified: bool,
        evidence: String,
    },
    ToolPermissionPrompt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        call_id: String,
        tool: String,
        arguments: serde_json::Value,
        reason: String,
    },

    // Speech Synthesis & Playback
    SpeechSynthesisStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        text_length: usize,
    },
    SpeechSynthesisFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        audio_duration_ms: u64,
        latency_ms: u64,
    },
    PlaybackStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
    },
    PlaybackFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
    },
    PlaybackInterrupted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        reason: String,
    },

    // System & Errors
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        component: String,
        message: String,
    },
    StatusUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        status: String,
    },
}

impl PicoEvent {
    /// Returns the attributed `SessionId` for session-scoped events, if present.
    pub fn session_id(&self) -> Option<SessionId> {
        match self {
            Self::SessionCreated { session_id }
            | Self::SessionCleared { session_id }
            | Self::ClarificationResolved { session_id, .. } => Some(*session_id),
            Self::VoiceActivityStarted { session_id, .. }
            | Self::VoiceActivityStopped { session_id, .. }
            | Self::TranscriptionPartial { session_id, .. }
            | Self::TranscriptionFinalized { session_id, .. }
            | Self::DecisionStarted { session_id, .. }
            | Self::CandidatesRanked { session_id, .. }
            | Self::DecisionSelected { session_id, .. }
            | Self::ClarificationRequested { session_id, .. }
            | Self::ExperienceObserved { session_id, .. }
            | Self::ToolCallRequested { session_id, .. }
            | Self::ToolExecutionStarted { session_id, .. }
            | Self::ToolExecutionFinished { session_id, .. }
            | Self::ObjectiveVerificationCompleted { session_id, .. }
            | Self::ToolPermissionPrompt { session_id, .. }
            | Self::SpeechSynthesisStarted { session_id, .. }
            | Self::SpeechSynthesisFinished { session_id, .. }
            | Self::PlaybackStarted { session_id }
            | Self::PlaybackFinished { session_id }
            | Self::PlaybackInterrupted { session_id, .. }
            | Self::Error { session_id, .. }
            | Self::StatusUpdate { session_id, .. } => *session_id,
            Self::AudioDeviceChanged { .. }
            | Self::VoiceBackendStatus { .. }
            | Self::ModelTrainingStarted { .. }
            | Self::ModelTrainingCompleted { .. }
            | Self::ModelRollbackOccurred { .. } => None,
        }
    }
}
