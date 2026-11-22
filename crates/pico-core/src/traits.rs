use crate::error::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Classification of tool security impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PermissionTier {
    ReadOnly = 0,
    LowRisk = 1,
    ConfirmationRequired = 2,
    Restricted = 3,
}

/// Contract for executable tools in Pico's ecosystem.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique identifier for the tool (e.g. "sys.process_list", "fs.read_file").
    fn name(&self) -> &str;

    /// Human-readable description for candidate ranking and discovery.
    fn description(&self) -> &str;

    /// JSON Schema describing input arguments.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Required permission tier to run this tool.
    fn permission_tier(&self) -> PermissionTier;

    /// Execute the tool with validated arguments.
    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value>;
}

/// Candidate action metadata considered by the Decision Model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateAction {
    pub name: String,
    pub description: String,
    pub category: String,
    pub permission_tier: PermissionTier,
    pub parameters_schema: serde_json::Value,
}

/// Scored candidate action with calibrated probability and feature attributions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScore {
    pub action_name: String,
    pub raw_score: f32,
    pub probability: f32,
    pub intent_match: f32,
    pub lexical_match: f32,
    pub semantic_match: f32,
    pub historical_success_rate: f32,
}

/// Abstract contract for non-generative probabilistic Decision Models.
#[async_trait]
pub trait DecisionModel: Send + Sync {
    /// Identifier or architecture name of the active decision model.
    fn model_name(&self) -> &str;

    /// Active model version (e.g. "pico-decision-v1.0.0").
    fn model_version(&self) -> &str;

    /// Evaluates and scores candidate actions for a given user objective.
    async fn score_candidates(
        &self,
        request: &str,
        candidates: &[CandidateAction],
    ) -> Result<Vec<CandidateScore>>;
}

/// Speech synthesis engine interface.
#[async_trait]
pub trait SpeechSynthesizer: Send + Sync {
    /// Synthesizes text into 16-bit 16kHz mono PCM audio bytes.
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>>;
}

/// Speech-to-text recognition interface.
#[async_trait]
pub trait SpeechRecognizer: Send + Sync {
    /// Transcribes 16kHz mono PCM audio samples into text.
    async fn transcribe(&self, pcm_samples: &[i16]) -> Result<String>;
}

/// Persistent memory and context retrieval interface.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a message in the active session.
    async fn save_message(&self, message: &crate::session::Message) -> Result<()>;

    /// Retrieve chronological messages for a session.
    async fn get_session_history(
        &self,
        session_id: &crate::session::SessionId,
        limit: usize,
    ) -> Result<Vec<crate::session::Message>>;

    /// Search long-term memory facts via keyword / semantic match.
    async fn search_relevant_context(&self, query: &str, limit: usize) -> Result<Vec<String>>;

    /// Persist a key fact or user preference.
    async fn remember_fact(&self, category: &str, fact: &str) -> Result<()>;
}
