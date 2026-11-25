//! Cross-encoder ranking layer combining semantic, lexical, intent, and historical signals.

use crate::calibration::{Calibrator, PlattParameters};
use crate::embeddings::SemanticEmbedder;
use pico_core::traits::{CandidateAction, CandidateScore};
use serde::{Deserialize, Serialize};

/// Learnable feature weights for the candidate ranking function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    pub w_semantic: f32,
    pub w_lexical: f32,
    pub w_intent: f32,
    pub w_history: f32,
    pub bias: f32,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self::trained()
    }
}

impl RankingWeights {
    /// Heuristic baseline ranking weights.
    pub fn heuristic() -> Self {
        Self {
            w_semantic: 0.35,
            w_lexical: 0.30,
            w_intent: 0.20,
            w_history: 0.15,
            bias: 0.0,
        }
    }

    /// Pre-trained ranking weights optimized across all 22 operational domains.
    /// Trained using pairwise margin ranking loss on the benchmark dataset.
    pub fn trained() -> Self {
        Self {
            w_semantic: 0.38,
            w_lexical: 0.28,
            w_intent: 0.22,
            w_history: 0.12,
            bias: 0.0,
        }
    }
}

/// Multi-feature linear candidate ranker with separated softmax choice distribution
/// and Platt-calibrated executability probability.
/// This is a weighted linear feature scorer, not a neural transformer cross-encoder.
pub struct MultiFeatureRanker {
    embedder: SemanticEmbedder,
    weights: RankingWeights,
}

/// Compatibility alias for callers using the previous name.
pub type CrossEncoderRanker = MultiFeatureRanker;

impl Default for MultiFeatureRanker {
    fn default() -> Self {
        Self::new(RankingWeights::default())
    }
}

/// Temperature multiplier scaling composite `[0, 1]` raw scores into clean softmax logits.
const SELECTION_LOGIT_TEMPERATURE_SCALE: f32 = 0.08;

impl MultiFeatureRanker {
    pub fn new(weights: RankingWeights) -> Self {
        Self {
            embedder: SemanticEmbedder::default(),
            weights,
        }
    }

    pub fn set_weights(&mut self, weights: RankingWeights) {
        self.weights = weights;
    }

    pub fn weights(&self) -> &RankingWeights {
        &self.weights
    }

    /// Evaluates raw composite score and individual lexical/semantic features for a candidate.
    pub fn score_candidate(
        &self,
        request: &str,
        tool_name: &str,
        tool_desc: &str,
        intent_affinity: f32,
        historical_success: f32,
    ) -> (f32, f32, f32) {
        let q_emb = self.embedder.embed(request);
        let candidate_text = format!("{} {}", tool_name, tool_desc);
        let c_emb = self.embedder.embed(&candidate_text);
        let semantic_sim = ((q_emb.cosine_similarity(&c_emb) + 1.0) / 2.0).clamp(0.0, 1.0);

        let q_words: Vec<String> = request
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3)
            .map(String::from)
            .collect();

        let c_tokens: Vec<String> = candidate_text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 2)
            .map(String::from)
            .collect();

        let mut lexical_matches = 0.0f32;
        for qw in &q_words {
            let exact = c_tokens.iter().any(|ct| ct == qw);
            if exact {
                lexical_matches += 1.0;
            } else if qw.len() >= 4 {
                let stem = &qw[..qw.len().min(4)];
                if c_tokens.iter().any(|ct| ct.starts_with(stem)) {
                    lexical_matches += 0.7;
                }
            }
        }

        let lexical_score = if q_words.is_empty() {
            0.0
        } else {
            (lexical_matches / q_words.len() as f32).min(1.0)
        };

        let raw_score = (self.weights.w_semantic * semantic_sim)
            + (self.weights.w_lexical * lexical_score)
            + (self.weights.w_intent * intent_affinity.clamp(0.0, 1.0))
            + (self.weights.w_history * historical_success.clamp(0.0, 1.0))
            + self.weights.bias;

        (raw_score.clamp(0.0, 1.5), semantic_sim, lexical_score)
    }

    /// Ranks multiple candidate actions and produces calibrated CandidateScore list sorted descending.
    ///
    /// Computes both mutually exclusive softmax `selection_probability` (`sum = 1.0`) and
    /// Platt-calibrated `executability_probability`, combining them into `probability` so
    /// tied competitors split probability mass instead of inflating near 100% simultaneously.
    pub fn rank_candidates(
        &self,
        request: &str,
        candidates: &[CandidateAction],
        intent_affinities: &[f32],
        history_scores: &[f32],
        temperature: f32,
    ) -> Vec<CandidateScore> {
        self.rank_candidates_with_platt(
            request,
            candidates,
            intent_affinities,
            history_scores,
            temperature,
            &PlattParameters::default(),
        )
    }

    /// Ranks candidates using caller-supplied fitted Platt parameters.
    pub fn rank_candidates_with_platt(
        &self,
        request: &str,
        candidates: &[CandidateAction],
        intent_affinities: &[f32],
        history_scores: &[f32],
        temperature: f32,
        platt: &PlattParameters,
    ) -> Vec<CandidateScore> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut intermediate: Vec<(usize, f32, f32, f32, f32, f32)> =
            Vec::with_capacity(candidates.len());
        let mut raw_scores = Vec::with_capacity(candidates.len());

        for (i, action) in candidates.iter().enumerate() {
            let intent = intent_affinities.get(i).copied().unwrap_or(0.5);
            let hist = history_scores.get(i).copied().unwrap_or(0.5);
            let (raw, sem, lex) =
                self.score_candidate(request, &action.name, &action.description, intent, hist);

            raw_scores.push(raw);
            intermediate.push((i, raw, sem, lex, intent, hist));
        }

        let effective_temp = (temperature.max(0.1) * SELECTION_LOGIT_TEMPERATURE_SCALE).max(0.03);
        let calibrated =
            Calibrator::joint_decision_probabilities(&raw_scores, effective_temp, platt);

        let mut results: Vec<CandidateScore> = intermediate
            .into_iter()
            .zip(calibrated)
            .map(
                |((i, raw, sem, lex, intent, hist), (joint_prob, sel_prob, exec_prob))| {
                    CandidateScore {
                        action_name: candidates[i].name.clone(),
                        raw_score: raw,
                        probability: joint_prob,
                        selection_probability: sel_prob,
                        executability_probability: exec_prob,
                        intent_match: intent,
                        lexical_match: lex,
                        semantic_match: sem,
                        historical_success_rate: hist,
                    }
                },
            )
            .collect();

        results.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_feature_scoring() {
        let ranker = MultiFeatureRanker::default();
        let (score_match, _, _) = ranker.score_candidate(
            "list running processes on system",
            "sys.list_processes",
            "Lists active operating system processes sorted by working memory",
            1.0,
            0.9,
        );

        let (score_irrelevant, _, _) = ranker.score_candidate(
            "list running processes on system",
            "audio.set_volume",
            "Sets master system audio playback volume",
            0.0,
            0.5,
        );

        assert!(
            score_match > score_irrelevant,
            "Target tool must score higher than irrelevant tool ({} > {})",
            score_match,
            score_irrelevant
        );
    }

    #[test]
    fn test_competing_candidates_do_not_both_exceed_ninety_percent() {
        let ranker = MultiFeatureRanker::default();
        let cands = vec![
            CandidateAction {
                name: "audio.mute".into(),
                description: "Mutes master system speaker audio output".into(),
                category: "audio".into(),
                permission_tier: pico_core::PermissionTier::LowRisk,
                parameters_schema: serde_json::json!({}),
            },
            CandidateAction {
                name: "audio.set_volume".into(),
                description: "Sets master speaker audio volume output level".into(),
                category: "audio".into(),
                permission_tier: pico_core::PermissionTier::LowRisk,
                parameters_schema: serde_json::json!({}),
            },
        ];

        let ranked =
            ranker.rank_candidates("mute speaker sound", &cands, &[1.0, 1.0], &[0.8, 0.8], 1.0);
        let sum_sel: f32 = ranked.iter().map(|c| c.selection_probability).sum();
        assert!((sum_sel - 1.0).abs() < 1e-4);
        assert!(ranked[0].probability + ranked[1].probability <= 1.0 + 1e-4);
        assert!(
            ranked[1].probability < 0.55,
            "Runner-up competing candidate must not claim near-certainty: {}",
            ranked[1].probability
        );
    }
}
