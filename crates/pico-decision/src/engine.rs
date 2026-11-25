//! Non-generative probabilistic Decision Engine.
//! Orchestrates intent classification, candidate tool retrieval, semantic ranking,
//! probability calibration, and structured argument resolution.

use crate::argument::{resolve_arguments, ArgumentResolution};
use crate::clarification::format_numbered_options;
use crate::index::ToolIndex;
use crate::intent::{IntentClassification, IntentClassifier, UserIntent};
use crate::synthesizer::ResponseSynthesizer;
use async_trait::async_trait;
use pico_core::config::DecisionConfig;
use pico_core::error::Result;
use pico_core::session::{ClarificationKind, ClarificationOption};
use pico_core::traits::{CandidateAction, CandidateScore, DecisionModel};
use pico_inference::ranking::{MultiFeatureRanker, RankingWeights};
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
        kind: ClarificationKind,
        prompt: String,
        options: Vec<ClarificationOption>,
        top_candidates: Vec<CandidateScore>,
    },
    /// Request is a direct conversational interaction or honest out-of-scope response.
    DirectConversation { response: String },
}

use pico_core::traits::{MemoryStore, ToolHistoryProvider};

/// The core probabilistic decision engine.
pub struct DecisionEngine {
    model_name: String,
    model_version: Arc<RwLock<String>>,
    classifier: IntentClassifier,
    index: Arc<RwLock<ToolIndex>>,
    ranker: Arc<RwLock<MultiFeatureRanker>>,
    config: Arc<RwLock<DecisionConfig>>,
    history_provider: Arc<RwLock<Option<Arc<dyn ToolHistoryProvider>>>>,
    memory_store: Arc<RwLock<Option<Arc<dyn MemoryStore>>>>,
}

impl DecisionEngine {
    pub fn new(config: DecisionConfig) -> Self {
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
            model_name: "pico-linear-calibrated-decision".to_string(),
            model_version: Arc::new(RwLock::new("pico-decision-v2.0.0".to_string())),
            classifier: IntentClassifier::new(),
            index: Arc::new(RwLock::new(ToolIndex::new())),
            ranker: Arc::new(RwLock::new(MultiFeatureRanker::new(initial_weights))),
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

        // Step 2: Retrieve candidate actions from compact ToolIndex (honoring config.max_candidates)
        let target_os = if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        };
        let max_cands = {
            let cfg = self.config.read().unwrap();
            cfg.max_candidates.max(1)
        };
        let (mut candidate_actions, index_entries) = {
            let index = self.index.read().unwrap();
            let cands = index.retrieve_candidates(
                trimmed,
                target_os,
                Some(classification.primary_intent.as_str()),
                max_cands,
            );
            let mut extra_cands = cands;
            if extra_cands.len() < 5 && !context_facts.is_empty() {
                for fact in &context_facts {
                    let extra = index.retrieve_candidates(
                        fact,
                        target_os,
                        Some(classification.primary_intent.as_str()),
                        5,
                    );
                    for c in extra {
                        if !extra_cands.iter().any(|existing| existing.name == c.name) {
                            extra_cands.push(c);
                        }
                    }
                }
            }
            let schemas: Vec<(String, String, serde_json::Value)> = extra_cands
                .iter()
                .map(|c| {
                    (
                        c.name.clone(),
                        c.description.clone(),
                        index
                            .get_entry(&c.name)
                            .map(|e| e.parameters_schema.clone())
                            .unwrap_or_else(|| c.parameters_schema.clone()),
                    )
                })
                .collect();
            (extra_cands, schemas)
        };
        if candidate_actions.is_empty() {
            let response =
                ResponseSynthesizer::format_direct_response(trimmed, classification.primary_intent);
            return Ok(DecisionResult::DirectConversation { response });
        }

        // Step 3: Score candidates via multi-feature ranker & calibrated probabilities
        let mut scored =
            self.score_candidates_internal(trimmed, &candidate_actions, &classification)?;

        if scored.is_empty() {
            let response =
                ResponseSynthesizer::format_direct_response(trimmed, classification.primary_intent);
            return Ok(DecisionResult::DirectConversation { response });
        }

        // Pre-resolve candidate schemas & arguments for the top candidates
        let build_option = |c: &CandidateScore| -> ClarificationOption {
            let (desc, schema) = index_entries
                .iter()
                .find(|(n, _, _)| n == &c.action_name)
                .map(|(_, d, s)| (d.clone(), s.clone()))
                .unwrap_or_else(|| (String::new(), serde_json::json!({})));
            let mut res = resolve_arguments(trimmed, &c.action_name, &schema);
            if !res.missing_required.is_empty() && !context_facts.is_empty() {
                merge_facts_into_resolution(&mut res, &context_facts, &c.action_name, &schema);
            }
            ClarificationOption {
                action_name: c.action_name.clone(),
                description: desc,
                probability: c.probability,
                selection_probability: c.selection_probability,
                parameters_schema: schema,
                pre_extracted_args: res.resolved_args,
                missing_required: res.missing_required,
            }
        };

        let cfg = self.config.read().unwrap().clone();
        let best = &scored[0];

        // Step 4: Out-of-scope / non-executable low-plausibility guard
        if classification.primary_intent == UserIntent::Unknown
            && best.executability_probability < 0.35
            && best.raw_score < 0.42
        {
            debug!(
                best_action = %best.action_name,
                exec_prob = best.executability_probability,
                raw_score = best.raw_score,
                "Input looks out of scope for available tools; declining to force an action"
            );
            return Ok(DecisionResult::DirectConversation {
                response: "I'm not sure which system action matches that request. Could you tell me specifically what tool or action you want?".to_string(),
            });
        }

        // Step 5: Check if best candidate is executable or genuinely ambiguous against top alternatives
        let margin = if scored.len() >= 2 {
            (best.selection_probability - scored[1].selection_probability).abs()
        } else {
            1.0
        };

        let top_options: Vec<ClarificationOption> =
            scored.iter().take(3).map(build_option).collect();
        let needs_choice = scored.len() >= 2
            && (margin < cfg.clarification_margin
                || (best.executability_probability < cfg.confidence_threshold
                    && (best.selection_probability - scored[1].selection_probability) < 0.15));

        if needs_choice {
            debug!(
                top1 = %best.action_name,
                top2 = %scored[1].action_name,
                sel1 = best.selection_probability,
                sel2 = scored[1].selection_probability,
                margin = margin,
                "Ambiguous candidates detected within clarification margin"
            );
            let choice_opts: Vec<ClarificationOption> = top_options.into_iter().take(2).collect();
            let prompt = format!(
                "I found two possible actions for '{}':\n{}\nReply with 'first', 'second', 'option 1', 'option 2', the action name, or 'cancel'.",
                trimmed,
                format_numbered_options(&choice_opts)
            );
            return Ok(DecisionResult::ClarificationRequired {
                kind: ClarificationKind::CandidateChoice,
                prompt,
                options: choice_opts,
                top_candidates: scored.iter().take(2).cloned().collect(),
            });
        }

        if best.probability < cfg.confidence_threshold
            && best.executability_probability < 0.55
            && scored.len() >= 2
        {
            let prompt = format!(
                "I am not certain ({:.0}% confidence) between:\n{}\nReply with an option number, the action name, or 'cancel'.",
                best.probability * 100.0,
                format_numbered_options(&top_options)
            );
            return Ok(DecisionResult::ClarificationRequired {
                kind: ClarificationKind::CandidateChoice,
                prompt,
                options: top_options,
                top_candidates: scored.iter().take(3).cloned().collect(),
            });
        }

        // Candidate list not further mutated beyond top_options; evaluate winner args cleanly
        candidate_actions.truncate(1);
        let chosen_opt = build_option(best);

        if !chosen_opt.missing_required.is_empty() {
            let prompt = ResponseSynthesizer::format_missing_args_prompt(
                &best.action_name,
                &chosen_opt.missing_required,
            );
            return Ok(DecisionResult::ClarificationRequired {
                kind: ClarificationKind::MissingArguments {
                    action_name: best.action_name.clone(),
                    parameters_schema: chosen_opt.parameters_schema.clone(),
                    partial_arguments: chosen_opt.pre_extracted_args.clone(),
                    missing_fields: chosen_opt.missing_required.clone(),
                },
                prompt,
                options: vec![chosen_opt],
                top_candidates: vec![best.clone()],
            });
        }

        // Keep top selection score clean without extra unused vectors
        let best_action_name = best.action_name.clone();
        let best_probability = best.probability;
        scored.truncate(5);

        Ok(DecisionResult::ActionSelected {
            action_name: best_action_name,
            arguments: chosen_opt.pre_extracted_args,
            confidence: best_probability,
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

fn merge_facts_into_resolution(
    resolution: &mut ArgumentResolution,
    context_facts: &[String],
    action_name: &str,
    schema: &serde_json::Value,
) {
    for fact in context_facts {
        let fact_res = resolve_arguments(fact, action_name, schema);
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
