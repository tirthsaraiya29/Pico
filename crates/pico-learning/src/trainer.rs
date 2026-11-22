//! Continuous on-device ranking optimizer.
//! Updates decision model ranking weights using pairwise margin ranking loss on observed human interactions.

use crate::observation::ObservationRecord;
use pico_core::error::Result;
use pico_inference::embeddings::SemanticEmbedder;
use pico_inference::ranking::RankingWeights;
use tracing::debug;

pub struct RankingTrainer {
    embedder: SemanticEmbedder,
    learning_rate: f32,
}

impl Default for RankingTrainer {
    fn default() -> Self {
        Self::new(0.05)
    }
}

impl RankingTrainer {
    pub fn new(learning_rate: f32) -> Self {
        Self {
            embedder: SemanticEmbedder::default(),
            learning_rate,
        }
    }

    /// Trains new candidate ranking weights from recorded experience observations.
    pub fn train_step(
        &self,
        current_weights: &RankingWeights,
        observations: &[ObservationRecord],
    ) -> Result<RankingWeights> {
        if observations.is_empty() {
            return Ok(current_weights.clone());
        }

        let mut grad_semantic = 0.0f32;
        let mut grad_lexical = 0.0f32;
        let mut grad_intent = 0.0f32;
        let mut grad_history = 0.0f32;

        for obs in observations {
            let weight = obs.quality.sample_weight();

            // 1. If user corrected the model (e.g. chosen was wrong, target was right)
            if let Some(target) = &obs.corrected_action {
                // Pairwise margin gradient: boost target features, penalize chosen features
                let q_emb = self.embedder.embed(&obs.request_text);
                let target_emb = self.embedder.embed(target);
                let chosen_emb = self.embedder.embed(&obs.selected_action);

                let sim_target = (q_emb.cosine_similarity(&target_emb) + 1.0) / 2.0;
                let sim_chosen = (q_emb.cosine_similarity(&chosen_emb) + 1.0) / 2.0;

                grad_semantic += weight * (sim_target - sim_chosen);
                grad_intent += weight * 0.2; // Intent affinity was likely higher on target
                grad_lexical += weight * 0.1;
                grad_history += weight * 0.05;
            } else if obs.user_feedback == Some(1) || obs.execution_status == "success" {
                // Positive reinforcement on successful choice
                grad_semantic += weight * 0.05;
                grad_intent += weight * 0.05;
                grad_lexical += weight * 0.02;
                grad_history += weight * 0.05;
            } else if obs.user_feedback == Some(-1) || obs.execution_status == "failure" {
                // Negative penalty
                grad_semantic -= weight * 0.05;
                grad_intent -= weight * 0.05;
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
