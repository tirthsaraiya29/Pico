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

/// Evaluates candidates with multi-factor cross-scoring.
pub struct CrossEncoderRanker {
    embedder: SemanticEmbedder,
    weights: RankingWeights,
}

impl Default for CrossEncoderRanker {
    fn default() -> Self {
        Self::new(RankingWeights::default())
    }
}

impl CrossEncoderRanker {
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

    /// Evaluates a candidate action against a user request.
    pub fn score_candidate(
        &self,
        request: &str,
        tool_name: &str,
        tool_desc: &str,
        intent_affinity: f32,
        historical_success: f32,
    ) -> (f32, f32, f32) {
        // 1. Semantic embedding similarity
        let q_emb = self.embedder.embed(request);
        let candidate_text = format!("{} {}", tool_name, tool_desc);
        let c_emb = self.embedder.embed(&candidate_text);
        let semantic_sim = (q_emb.cosine_similarity(&c_emb) + 1.0) / 2.0; // normalize to [0, 1]

        // 2. Lexical word overlap with stem and substring matching
        let q_words: Vec<String> = request
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| w.len() >= 3)
            .collect();

        let mut lexical_matches = 0;
        let c_lower = candidate_text.to_lowercase();
        for w in &q_words {
            let stem_len = w.len().min(4);
            let stem = &w[..stem_len];
            if c_lower.contains(w) || (stem.len() >= 3 && c_lower.contains(stem)) {
                lexical_matches += 1;
            }
        }

        let lexical_score = if q_words.is_empty() {
            0.0
        } else {
            (lexical_matches as f32 / q_words.len() as f32).min(1.0)
        };

        // 3. Composite score
        let raw_score = (self.weights.w_semantic * semantic_sim)
            + (self.weights.w_lexical * lexical_score)
            + (self.weights.w_intent * intent_affinity.clamp(0.0, 1.0))
            + (self.weights.w_history * historical_success.clamp(0.0, 1.0))
            + self.weights.bias;

        (raw_score, semantic_sim, lexical_score)
    }

    /// Ranks multiple candidate actions and produces calibrated CandidateScore list sorted descending.
    pub fn rank_candidates(
        &self,
        request: &str,
        candidates: &[CandidateAction],
        intent_affinities: &[f32],
        history_scores: &[f32],
        temperature: f32,
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

        let platt = PlattParameters::default();

        let mut results: Vec<CandidateScore> = intermediate
            .into_iter()
            .map(|(i, raw, sem, lex, intent, hist)| {
                let calibrated_prob =
                    Calibrator::platt_calibrate_with_temperature(raw, &platt, temperature)
                        .clamp(0.0, 1.0);
                CandidateScore {
                    action_name: candidates[i].name.clone(),
                    raw_score: raw,
                    probability: calibrated_prob,
                    intent_match: intent,
                    lexical_match: lex,
                    semantic_match: sem,
                    historical_success_rate: hist,
                }
            })
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
    fn test_cross_encoder_scoring() {
        let ranker = CrossEncoderRanker::default();
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
}
