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
            content: result.content.to_string(),
            tool_calls: None,
            tool_call_id: Some(result.call_id.clone()),
            timestamp: Utc::now(),
            token_count: None,
        }
    }
}
