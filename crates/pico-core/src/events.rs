use crate::session::{SessionId, ToolCall, ToolResult};
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
    VoiceActivityStarted {
        timestamp: DateTime<Utc>,
    },
    VoiceActivityStopped {
        timestamp: DateTime<Utc>,
        duration_ms: u64,
    },

    // Speech-to-Text
    TranscriptionPartial {
        partial_text: String,
    },
    TranscriptionFinalized {
        text: String,
        latency_ms: u64,
    },

    // Model Reasoning & Generation
    InferenceStarted {
        model: String,
        prompt_tokens: usize,
    },
    TokenChunk {
        token: String,
    },
    ThinkingChunk {
        thinking: String,
    },
    InferenceFinished {
        total_tokens: usize,
        latency_ms: u64,
    },

    // Structured Tools
    ToolCallRequested {
        call: ToolCall,
    },
    ToolExecutionStarted {
        call_id: String,
        tool: String,
    },
    ToolExecutionFinished {
        result: ToolResult,
    },
    ToolPermissionPrompt {
        call_id: String,
        tool: String,
        reason: String,
    },

    // Speech Synthesis & Playback
    SpeechSynthesisStarted {
        text_length: usize,
    },
    SpeechSynthesisFinished {
        audio_duration_ms: u64,
        latency_ms: u64,
    },
    PlaybackStarted,
    PlaybackFinished,
    PlaybackInterrupted {
        reason: String,
    },

    // System & Errors
    Error {
        component: String,
        message: String,
    },
    StatusUpdate {
        status: String,
    },
}
