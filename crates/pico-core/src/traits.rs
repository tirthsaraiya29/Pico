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
    #[serde(default)]
    pub selection_probability: f32,
    #[serde(default)]
    pub executability_probability: f32,
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

/// Sample encoding within Pico's internal normalized audio frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioSampleEncoding {
    F32,
    I16,
}

/// Explicit format descriptor attached to every internal audio buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: AudioSampleEncoding,
}

impl AudioFormat {
    pub const fn mono_16k_i16() -> Self {
        Self {
            sample_rate: 16_000,
            channels: 1,
            encoding: AudioSampleEncoding::I16,
        }
    }

    pub const fn new(sample_rate: u32, channels: u16, encoding: AudioSampleEncoding) -> Self {
        Self {
            sample_rate,
            channels,
            encoding,
        }
    }
}

/// Self-describing normalized audio waveform produced by any TTS engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesizedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub voice: String,
    pub engine: String,
}

impl SynthesizedAudio {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0;
        }
        let frames = self.samples.len() / self.channels as usize;
        (frames as u64 * 1000) / self.sample_rate as u64
    }

    /// Converts normalized float waveform into interleaved 16-bit signed little-endian PCM bytes
    /// preserving the buffer's explicit sample_rate and channels.
    pub fn to_i16_pcm_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.samples.len() * 2);
        for &s in &self.samples {
            let i = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            out.extend_from_slice(&i.to_le_bytes());
        }
        out
    }
}

/// Discovered TTS voice descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub name: String,
    pub language: String,
    pub engine: String,
}

/// Speech synthesis engine interface with explicit audio format metadata.
#[async_trait]
pub trait SpeechSynthesizer: Send + Sync {
    /// Active engine identifier (e.g. "winrt", "native_sapi", "piper", "espeak", "mock").
    fn engine_name(&self) -> &str {
        "default"
    }

    /// Lists discovered voices supported by this engine on the host OS.
    fn list_voices(&self) -> Vec<VoiceInfo> {
        Vec::new()
    }

    /// Synthesizes text into normalized self-describing audio samples with explicit sample rate and channels.
    async fn synthesize_audio(&self, text: &str) -> Result<SynthesizedAudio>;

    /// Compatibility wrapper returning 16-bit signed PCM bytes at the synthesizer's native format.
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let audio = self.synthesize_audio(text).await?;
        Ok(audio.to_i16_pcm_bytes())
    }
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

/// Historical success rate provider for tool ranking calibration.
pub trait ToolHistoryProvider: Send + Sync {
    /// Returns the empirical or Bayesian-smoothed historical success rate [0.0, 1.0] for the given tool.
    fn get_success_rate(&self, tool_name: &str) -> f32;
}
