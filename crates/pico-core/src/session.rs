use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a conversational session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The role of a message participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::System => write!(f, "system"),
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::Tool => write!(f, "tool"),
        }
    }
}

/// A structured tool call request emitted by an LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Maximum characters stored verbatim per tool result message in session history.
const MAX_TOOL_HISTORY_CHARS: usize = 16_384;

/// Discriminates the type of pending session clarification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClarificationKind {
    /// Two or more competing candidate actions had close selection probabilities.
    CandidateChoice,
    /// One action was selected, but required arguments remain unspecified.
    MissingArguments {
        action_name: String,
        parameters_schema: serde_json::Value,
        partial_arguments: serde_json::Value,
        missing_fields: Vec<String>,
    },
    /// Action requires user yes/no confirmation before proceeding.
    ActionConfirmation {
        action_name: String,
        arguments: serde_json::Value,
        reason: String,
    },
}

/// Candidate snapshot saved for session-scoped clarification follow-ups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClarificationOption {
    pub action_name: String,
    pub description: String,
    pub probability: f32,
    pub selection_probability: f32,
    pub parameters_schema: serde_json::Value,
    pub pre_extracted_args: serde_json::Value,
    pub missing_required: Vec<String>,
}

/// Persistent, request-scoped and session-scoped clarification state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClarificationState {
    pub id: String,
    pub session_id: SessionId,
    pub original_request: String,
    pub kind: ClarificationKind,
    pub options: Vec<ClarificationOption>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub attempts: u8,
}

impl ClarificationState {
    pub fn new(
        session_id: SessionId,
        original_request: impl Into<String>,
        kind: ClarificationKind,
        options: Vec<ClarificationOption>,
        ttl_seconds: u64,
    ) -> Self {
        let now = Utc::now();
        let ttl = chrono::Duration::seconds(ttl_seconds.max(15) as i64);
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            original_request: original_request.into(),
            kind,
            options,
            created_at: now,
            expires_at: now + ttl,
            attempts: 0,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// The result of executing a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub success: bool,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub execution_duration_ms: u64,
}

/// A single message in the conversation history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub session_id: SessionId,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub token_count: Option<usize>,
}

impl Message {
    pub fn new_user(session_id: SessionId, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            timestamp: Utc::now(),
            token_count: None,
        }
    }

    pub fn new_assistant(
        session_id: SessionId,
        content: impl Into<String>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            timestamp: Utc::now(),
            token_count: None,
        }
    }

    pub fn new_system(session_id: SessionId, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            timestamp: Utc::now(),
            token_count: None,
        }
    }

    pub fn new_tool_result(session_id: SessionId, result: &ToolResult) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role: Role::Tool,
            content: result.bounded_content_string(MAX_TOOL_HISTORY_CHARS),
            tool_calls: None,
            tool_call_id: Some(result.call_id.clone()),
            timestamp: Utc::now(),
            token_count: None,
        }
    }
}

impl ToolResult {
    /// Serializes `content` and safely truncates it to at most `max_chars` Unicode scalar values
    /// (plus a truncation marker) so oversized tool outputs never bloat session history.
    pub fn bounded_content_string(&self, max_chars: usize) -> String {
        let serialized = self.content.to_string();
        if serialized.chars().count() > max_chars {
            let mut truncated: String = serialized.chars().take(max_chars).collect();
            truncated.push_str("...[truncated]");
            truncated
        } else {
            serialized
        }
    }
}
