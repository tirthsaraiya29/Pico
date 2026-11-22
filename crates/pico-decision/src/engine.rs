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

use pico_core::traits::{MemoryStore, ToolHistoryProvider};

/// The core probabilistic decision engine.
pub struct DecisionEngine {
    model_name: String,
    model_version: Arc<RwLock<String>>,
    classifier: IntentClassifier,
    index: Arc<RwLock<ToolIndex>>,
    ranker: Arc<RwLock<CrossEncoderRanker>>,
    config: Arc<RwLock<DecisionConfig>>,
    history_provider: Arc<RwLock<Option<Arc<dyn ToolHistoryProvider>>>>,
    memory_store: Arc<RwLock<Option<Arc<dyn MemoryStore>>>>,
}

impl DecisionEngine {
    pub fn new(config: DecisionConfig) -> Self {
        // Load trained weights by default or load saved checkpoint from model_dir if exists
        let weights_path = config.model_dir.join("weights.json");
        let initial_weights = if weights_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&weights_path) {
                serde_json::from_str::<RankingWeights>(&content)
                    .unwrap_or_else(|_| RankingWeights::trained())
            } else {
                RankingWeights::trained()
            }
        } else {
            RankingWeights::trained()
        };

        Self {
            model_name: "pico-probabilistic-engine".to_string(),
            model_version: Arc::new(RwLock::new("pico-decision-v1.0.0".to_string())),
            classifier: IntentClassifier::new(),
            index: Arc::new(RwLock::new(ToolIndex::new())),
            ranker: Arc::new(RwLock::new(CrossEncoderRanker::new(initial_weights))),
            config: Arc::new(RwLock::new(config)),
            history_provider: Arc::new(RwLock::new(None)),
            memory_store: Arc::new(RwLock::new(None)),
        }
    }

    /// Connects a historical tool success provider for dynamic calibration.
    pub fn set_history_provider(&self, provider: Arc<dyn ToolHistoryProvider>) {
        let mut p = self.history_provider.write().unwrap();
        *p = Some(provider);
    }

    /// Connects conversational memory store for context-aware tool selection.
    pub fn set_memory_store(&self, store: Arc<dyn MemoryStore>) {
        let mut m = self.memory_store.write().unwrap();
        *m = Some(store);
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

        // Query memory context if available for context-aware tool selection
        let mut context_facts = Vec::new();
        let memory_store = {
            let guard = self.memory_store.read().unwrap();
            guard.clone()
        };
        if let Some(ref mem) = memory_store {
            if let Ok(facts) = mem.search_relevant_context(trimmed, 3).await {
                context_facts = facts;
            }
        }

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
        let mut candidate_actions = index.retrieve_candidates(
            trimmed,
            target_os,
            Some(classification.primary_intent.as_str()),
            15,
        );

        // Augment candidates with context facts from memory if pool is sparse
        if candidate_actions.len() < 5 && !context_facts.is_empty() {
            for fact in &context_facts {
                let extra = index.retrieve_candidates(
                    fact,
                    target_os,
                    Some(classification.primary_intent.as_str()),
                    5,
                );
                for c in extra {
                    if !candidate_actions
                        .iter()
                        .any(|existing| existing.name == c.name)
                    {
                        candidate_actions.push(c);
                    }
                }
            }
        }
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
        let mut resolution: ArgumentResolution =
            resolve_arguments(trimmed, &best.action_name, &schema);

        // Context-aware fallback: if required arguments are missing, check memory facts
        if !resolution.missing_required.is_empty() && !context_facts.is_empty() {
            for fact in &context_facts {
                let fact_res = resolve_arguments(fact, &best.action_name, &schema);
                if let (Some(res_obj), Some(fact_obj)) = (
                    resolution.resolved_args.as_object_mut(),
                    fact_res.resolved_args.as_object(),
                ) {
                    for (k, v) in fact_obj {
                        if !res_obj.contains_key(k) {
                            res_obj.insert(k.clone(), v.clone());
                            resolution.missing_required.retain(|m| m != k);
                        }
                    }
                }

                // Check key-value pattern (e.g. "param: value" or "param is value")
                let missing_clone = resolution.missing_required.clone();
                for req in missing_clone {
                    let prefix = format!("{}:", req);
                    if let Some(pos) = fact.to_lowercase().find(&prefix) {
                        let val = fact[pos + prefix.len()..]
                            .trim()
                            .trim_matches(|c| c == '"' || c == '\'');
                        if !val.is_empty() {
                            if let Some(res_obj) = resolution.resolved_args.as_object_mut() {
                                res_obj.insert(req.clone(), serde_json::json!(val));
                                resolution.missing_required.retain(|m| m != &req);
                            }
                        }
                    }
                }
            }
        }

        // If required arguments are still missing, request clarification
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

        // Dynamically compute historical tool success rate from experience store
        let history_scores: Vec<f32> = {
            let provider_opt = self.history_provider.read().unwrap().clone();
            if let Some(provider) = provider_opt {
                candidates
                    .iter()
                    .map(|c| provider.get_success_rate(&c.name))
                    .collect()
            } else {
                vec![0.8f32; candidates.len()]
            }
        };

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
