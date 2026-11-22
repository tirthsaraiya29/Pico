//! Non-generative probabilistic Decision Engine.
//! Orchestrates intent classification, candidate tool retrieval, semantic ranking,
//! probability calibration, and structured argument resolution.

use crate::argument::{resolve_arguments, ArgumentResolution};
use crate::index::ToolIndex;
use crate::intent::{IntentClassification, IntentClassifier};
use crate::synthesizer::ResponseSynthesizer;
use async_trait::async_trait;
use pico_core::config::DecisionConfig;
use pico_core::error::Result;
use pico_core::traits::{CandidateAction, CandidateScore, DecisionModel};
use pico_inference::ranking::{CrossEncoderRanker, RankingWeights};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tracing::{debug, info};

/// Full decision outcome for an incoming user request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionResult {
    /// High-confidence tool action selected and resolved.
    ActionSelected {
        action_name: String,
        arguments: serde_json::Value,
        confidence: f32,
        ranked_scores: Vec<CandidateScore>,
    },
    /// Request requires clarification between top ambiguous actions or missing arguments.
    ClarificationRequired {
        prompt: String,
        top_candidates: Vec<CandidateScore>,
    },
    /// Request is a direct conversational interaction (no tool required).
    DirectConversation { response: String },
}

/// The core probabilistic decision engine.
pub struct DecisionEngine {
    model_name: String,
    model_version: Arc<RwLock<String>>,
    classifier: IntentClassifier,
    index: Arc<RwLock<ToolIndex>>,
    ranker: Arc<RwLock<CrossEncoderRanker>>,
    config: Arc<RwLock<DecisionConfig>>,
}

impl DecisionEngine {
    pub fn new(config: DecisionConfig) -> Self {
        Self {
            model_name: "pico-probabilistic-engine".to_string(),
            model_version: Arc::new(RwLock::new("pico-decision-v1.0.0".to_string())),
            classifier: IntentClassifier::new(),
            index: Arc::new(RwLock::new(ToolIndex::new())),
            ranker: Arc::new(RwLock::new(CrossEncoderRanker::default())),
            config: Arc::new(RwLock::new(config)),
        }
    }

    /// Access the tool index for registering tools.
    pub fn tool_index(&self) -> Arc<RwLock<ToolIndex>> {
        Arc::clone(&self.index)
    }

    /// Access the ranking weights for continuous model learning updates.
    pub fn update_weights(&self, weights: RankingWeights, version: String) {
        let mut r = self.ranker.write().unwrap();
        r.set_weights(weights);
        let mut v = self.model_version.write().unwrap();
        *v = version;
        info!("Updated decision model ranking weights to version {}", *v);
    }

    /// Gets current model weights.
    pub fn current_weights(&self) -> RankingWeights {
        self.ranker.read().unwrap().weights().clone()
    }

    /// Primary decision-making method evaluating user query into an actionable decision.
    pub async fn decide(&self, request: &str) -> Result<DecisionResult> {
        let trimmed = request.trim();
        debug!(
            request = trimmed,
            "Evaluating user request in Decision Engine"
        );

        // Step 1: Fast non-generative intent classification
        let classification: IntentClassification = self.classifier.classify(trimmed);
        debug!(
            primary_intent = classification.primary_intent.as_str(),
            requires_tool = classification.requires_tool,
            confidence = classification.confidence,
            "Intent classification complete"
        );

        // If direct conversation without tool requirements
        if !classification.requires_tool {
            let response =
                ResponseSynthesizer::format_direct_response(trimmed, classification.primary_intent);
            return Ok(DecisionResult::DirectConversation { response });
        }

        // Step 2: Retrieve candidate actions from compact ToolIndex
        let target_os = if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        };
        let index = self.index.read().unwrap();
        let candidate_actions = index.retrieve_candidates(
            trimmed,
            target_os,
            Some(classification.primary_intent.as_str()),
            15,
        );
        if candidate_actions.is_empty() {
            // No tools match; direct response fallback
            let response =
                ResponseSynthesizer::format_direct_response(trimmed, classification.primary_intent);
            return Ok(DecisionResult::DirectConversation { response });
        }

        // Step 3: Score candidates via multi-factor cross-encoder
        let scored =
            self.score_candidates_internal(trimmed, &candidate_actions, &classification)?;

        if scored.is_empty() {
            let response =
                ResponseSynthesizer::format_direct_response(trimmed, classification.primary_intent);
            return Ok(DecisionResult::DirectConversation { response });
        }

        let cfg = self.config.read().unwrap();
        let best = &scored[0];

        // Step 4: Check if best candidate meets decision confidence threshold
        if best.probability < cfg.confidence_threshold {
            debug!(
                best_action = %best.action_name,
                prob = best.probability,
                threshold = cfg.confidence_threshold,
                "Best action below confidence threshold"
            );
            let prompt = if scored.len() >= 2 {
                ResponseSynthesizer::format_clarification_prompt(
                    &scored[0].action_name,
                    scored[0].probability,
                    &scored[1].action_name,
                    scored[1].probability,
                )
            } else {
                format!(
                    "I am not confident enough ({:.0}% sure) to execute '{}'. Could you please rephrase?",
                    best.probability * 100.0,
                    best.action_name
                )
            };
            return Ok(DecisionResult::ClarificationRequired {
                prompt,
                top_candidates: scored.iter().take(3).cloned().collect(),
            });
        }

        // Step 5: Check ambiguity margin between Top 1 and Top 2
        if scored.len() >= 2 {
            let margin = best.probability - scored[1].probability;
            if margin < cfg.clarification_margin {
                debug!(
                    top1 = %best.action_name,
                    top2 = %scored[1].action_name,
                    margin = margin,
                    "Ambiguous candidates detected within clarification margin"
                );
                let prompt = ResponseSynthesizer::format_clarification_prompt(
                    &scored[0].action_name,
                    scored[0].probability,
                    &scored[1].action_name,
                    scored[1].probability,
                );
                return Ok(DecisionResult::ClarificationRequired {
                    prompt,
                    top_candidates: scored.iter().take(2).cloned().collect(),
                });
            }
        }

        // Step 6: Find matching tool entry for parameter schema
        let entry_opt = index.get_entry(&best.action_name);
        let schema = entry_opt
            .map(|e| e.parameters_schema.clone())
            .unwrap_or_else(|| serde_json::json!({}));

        // Step 7: Deterministic argument resolution
        let resolution: ArgumentResolution = resolve_arguments(trimmed, &best.action_name, &schema);

        // If required arguments are missing, request clarification
        if !resolution.missing_required.is_empty() {
            let prompt = ResponseSynthesizer::format_missing_args_prompt(
                &best.action_name,
                &resolution.missing_required,
            );
            return Ok(DecisionResult::ClarificationRequired {
                prompt,
                top_candidates: vec![best.clone()],
            });
        }

        Ok(DecisionResult::ActionSelected {
            action_name: best.action_name.clone(),
            arguments: resolution.resolved_args,
            confidence: best.probability,
            ranked_scores: scored,
        })
    }

    fn score_candidates_internal(
        &self,
        request: &str,
        candidates: &[CandidateAction],
        classification: &IntentClassification,
    ) -> Result<Vec<CandidateScore>> {
        let intent_affinities: Vec<f32> = candidates
            .iter()
            .map(|c| {
                classification
                    .primary_intent
                    .affinity_for_category(&c.category)
            })
            .collect();
        let history_scores = vec![0.8f32; candidates.len()];
        let cfg = self.config.read().unwrap();
        let ranker = self.ranker.read().unwrap();
        let ranked = ranker.rank_candidates(
            request,
            candidates,
            &intent_affinities,
            &history_scores,
            cfg.calibration_temperature,
        );
        Ok(ranked)
    }
}

#[async_trait]
impl DecisionModel for DecisionEngine {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn model_version(&self) -> &str {
        "pico-decision-v1.0.0"
    }

    async fn score_candidates(
        &self,
        request: &str,
        candidates: &[CandidateAction],
    ) -> Result<Vec<CandidateScore>> {
        let classification = self.classifier.classify(request);
        self.score_candidates_internal(request, candidates, &classification)
    }
}
