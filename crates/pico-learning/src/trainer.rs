//! Continuous on-device ranking optimizer.
//! Updates decision model ranking weights using pairwise margin ranking loss on observed human interactions.

use crate::observation::ObservationRecord;
use pico_core::error::Result;
use pico_inference::ranking::{MultiFeatureRanker, RankingWeights};
use tracing::debug;

pub struct RankingTrainer {
    learning_rate: f32,
}

impl Default for RankingTrainer {
    fn default() -> Self {
        Self::new(0.05)
    }
}

impl RankingTrainer {
    pub fn new(learning_rate: f32) -> Self {
        Self { learning_rate }
    }

    /// Trains new candidate ranking weights using pairwise feature differences from observations.
    pub fn train_step(
        &self,
        current_weights: &RankingWeights,
        observations: &[ObservationRecord],
    ) -> Result<RankingWeights> {
        if observations.is_empty() {
            return Ok(current_weights.clone());
        }

        let ranker = MultiFeatureRanker::new(current_weights.clone());
        let mut grad_semantic = 0.0f32;
        let mut grad_lexical = 0.0f32;
        let mut grad_intent = 0.0f32;
        let mut grad_history = 0.0f32;

        for obs in observations {
            let weight = obs.quality.sample_weight();

            if let Some(target) = &obs.corrected_action {
                let (_, sem_target, lex_target) =
                    ranker.score_candidate(&obs.request_text, target, target, 0.5, 0.5);
                let (_, sem_chosen, lex_chosen) = ranker.score_candidate(
                    &obs.request_text,
                    &obs.selected_action,
                    &obs.selected_action,
                    0.5,
                    0.5,
                );

                let int_match_target = if target
                    .to_lowercase()
                    .contains(&obs.primary_intent.to_lowercase())
                {
                    1.0
                } else {
                    0.5
                };
                let int_match_chosen = if obs
                    .selected_action
                    .to_lowercase()
                    .contains(&obs.primary_intent.to_lowercase())
                {
                    1.0
                } else {
                    0.5
                };

                grad_semantic += weight * (sem_target - sem_chosen);
                grad_lexical += weight * (lex_target - lex_chosen);
                grad_intent += weight * (int_match_target - int_match_chosen);
                // Avoid over-weighting stale historical prior on explicit corrections
                grad_history -= weight * 0.05;
            } else if obs.user_feedback == Some(1) || obs.execution_status == "success" {
                let (_, sem, lex) = ranker.score_candidate(
                    &obs.request_text,
                    &obs.selected_action,
                    &obs.selected_action,
                    0.5,
                    0.5,
                );
                let margin = (sem + lex) * 0.5 - 0.3;
                if margin > 0.0 {
                    grad_semantic += weight * (sem - 0.5).max(0.0) * 0.1;
                    grad_lexical += weight * (lex - 0.3).max(0.0) * 0.1;
                    grad_history += weight * 0.02;
                }
            } else if obs.user_feedback == Some(-1) || obs.execution_status == "failure" {
                let (_, sem, lex) = ranker.score_candidate(
                    &obs.request_text,
                    &obs.selected_action,
                    &obs.selected_action,
                    0.5,
                    0.5,
                );
                grad_semantic -= weight * (sem - 0.4).max(0.0) * 0.1;
                grad_lexical -= weight * (lex - 0.3).max(0.0) * 0.1;
                grad_intent -= weight * 0.02;
            }
        }

        let n = observations.len() as f32;
        let lr = self.learning_rate;

        let mut new_w_semantic = (current_weights.w_semantic + lr * (grad_semantic / n)).max(0.05);
        let mut new_w_lexical = (current_weights.w_lexical + lr * (grad_lexical / n)).max(0.05);
        let mut new_w_intent = (current_weights.w_intent + lr * (grad_intent / n)).max(0.05);
        let mut new_w_history = (current_weights.w_history + lr * (grad_history / n)).max(0.05);

        // Normalize sum to 1.0
        let total = new_w_semantic + new_w_lexical + new_w_intent + new_w_history;
        if total > 0.0 {
            new_w_semantic /= total;
            new_w_lexical /= total;
            new_w_intent /= total;
            new_w_history /= total;
        }

        let updated = RankingWeights {
            w_semantic: new_w_semantic,
            w_lexical: new_w_lexical,
            w_intent: new_w_intent,
            w_history: new_w_history,
            bias: current_weights.bias,
        };

        debug!(
            w_sem = updated.w_semantic,
            w_lex = updated.w_lexical,
            w_int = updated.w_intent,
            w_hist = updated.w_history,
            "Calculated candidate weights from training step"
        );

        Ok(updated)
    }
}
