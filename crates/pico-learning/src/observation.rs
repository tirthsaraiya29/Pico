//! Structured experience observation models and label quality hierarchy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Quality level of the training signal derived from human-machine interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LabelQuality {
    /// Inferred solely from passive signals (lowest confidence, weight 0.5)
    Inferred = 0,
    /// Automated tool execution outcome (weight 1.0)
    ExecutionOutcome = 1,
    /// Explicit user feedback e.g. thumbs up/down (weight 2.0)
    ExplicitFeedback = 2,
    /// Direct user correction e.g. "no, I meant kill chrome" (highest confidence, weight 3.5)
    UserCorrection = 3,
}

impl LabelQuality {
    pub fn sample_weight(&self) -> f32 {
        match self {
            LabelQuality::Inferred => 0.5,
            LabelQuality::ExecutionOutcome => 1.0,
            LabelQuality::ExplicitFeedback => 2.0,
            LabelQuality::UserCorrection => 3.5,
        }
    }
}

/// A structured decision observation record persisted for continuous learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationRecord {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub request_text: String,
    pub primary_intent: String,
    pub candidate_names: Vec<String>,
    pub candidate_scores: Vec<f32>,
    pub selected_action: String,
    pub confidence: f32,
    pub execution_status: String, // "success", "failure", "cancelled", "clarified"
    pub latency_ms: u64,
    pub user_feedback: Option<i32>, // +1 thumbs up, -1 thumbs down
    pub corrected_action: Option<String>,
    pub quality: LabelQuality,
}
