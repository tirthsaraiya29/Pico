//! Central coordinator for continuous learning, evaluations, and checkpoint management.

use crate::evaluator::{EvaluationMetrics, ModelEvaluator};
use crate::observation::{LabelQuality, ObservationRecord};
use crate::poisoning::AntiPoisoningGuard;
use crate::store::{ExperienceStore, ModelCheckpoint};
use crate::trainer::RankingTrainer;
use chrono::Utc;
use pico_core::config::LearningConfig;
use pico_core::error::Result;
use pico_inference::ranking::RankingWeights;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningStats {
    pub total_observations: usize,
    pub active_version: String,
    pub top1_accuracy: f32,
    pub top3_recall: f32,
    pub active_weights: RankingWeights,
    pub available_checkpoints: usize,
}

pub struct LearningManager {
    store: Arc<ExperienceStore>,
    guard: AntiPoisoningGuard,
    evaluator: ModelEvaluator,
    trainer: RankingTrainer,
    config: LearningConfig,
}

impl LearningManager {
    pub fn new(config: LearningConfig) -> Result<Self> {
        let store = Arc::new(ExperienceStore::new(&config.database_path)?);
        let guard = AntiPoisoningGuard::default();
        let evaluator = ModelEvaluator::new();
        let trainer = RankingTrainer::new(config.learning_rate);

        let manager = Self {
            store,
            guard,
            evaluator,
            trainer,
            config,
        };

        // Initialize baseline checkpoint if none exists
        if manager.store.get_active_checkpoint()?.is_none() {
            let initial = ModelCheckpoint {
                version: "pico-model-v1.0.0".to_string(),
                weights: RankingWeights::default(),
                top1_accuracy: 1.0,
                top3_recall: 1.0,
                created_at: Utc::now(),
                is_active: true,
            };
            manager.store.save_checkpoint(&initial)?;
        }

        Ok(manager)
    }

    /// For in-memory testing.
    pub fn in_memory(config: LearningConfig) -> Result<Self> {
        let store = Arc::new(ExperienceStore::in_memory()?);
        let guard = AntiPoisoningGuard::default();
        let evaluator = ModelEvaluator::new();
        let trainer = RankingTrainer::new(config.learning_rate);

        let manager = Self {
            store,
            guard,
            evaluator,
            trainer,
            config,
        };

        let initial = ModelCheckpoint {
            version: "pico-model-v1.0.0".to_string(),
            weights: RankingWeights::default(),
            top1_accuracy: 1.0,
            top3_recall: 1.0,
            created_at: Utc::now(),
            is_active: true,
        };
        manager.store.save_checkpoint(&initial)?;

        Ok(manager)
    }

    /// Records an interaction observation into experience memory.
    pub fn record_observation(&self, obs: ObservationRecord) -> Result<()> {
        self.store.insert_observation(&obs)
    }

    /// Records explicit thumbs up (+1) or thumbs down (-1) feedback.
    pub fn record_feedback(&self, observation_id: &str, feedback: i32) -> Result<()> {
        self.store.update_feedback(observation_id, feedback)
    }

    /// Records a user correction against a decision observation.
    pub fn record_correction(&self, observation_id: &str, target_action: &str) -> Result<()> {
        self.guard.validate_correction_rate()?;
        self.store.record_correction(observation_id, target_action)
    }

    /// Runs a training and validation step over accumulated experience.
    /// Returns new weights, version string, and metrics if candidate passes promotion gate.
    pub fn train_and_evaluate(
        &self,
        current_weights: &RankingWeights,
    ) -> Result<Option<(RankingWeights, String, EvaluationMetrics)>> {
        let observations = self
            .store
            .get_training_observations(LabelQuality::ExecutionOutcome, 200)?;

        if observations.len() < self.config.min_observations_for_retraining {
            debug_info(format!(
                "Insufficient observations for training: {} < {}",
                observations.len(),
                self.config.min_observations_for_retraining
            ));
            return Ok(None);
        }

        // 1. Train candidate weights
        let candidate_raw = self.trainer.train_step(current_weights, &observations)?;

        // 2. Apply anti-poisoning bounded clipping
        let candidate_clipped = self.guard.clip_weights(current_weights, &candidate_raw);

        // 3. Evaluate candidate against held-out benchmark
        let active = self.store.get_active_checkpoint()?;
        let baseline_acc = active.as_ref().map(|a| a.top1_accuracy).unwrap_or(0.8);
        let metrics = self.evaluator.evaluate(&candidate_clipped, baseline_acc)?;

        if !metrics.passed_promotion {
            warn!(
                acc = metrics.top1_accuracy,
                recall = metrics.top3_recall,
                "Candidate model failed promotion gate, rejecting update"
            );
            return Ok(None);
        }

        // 4. Candidate passed promotion! Create versioned checkpoint
        let new_version = format!("pico-model-v1.{}", Utc::now().timestamp());
        let checkpoint = ModelCheckpoint {
            version: new_version.clone(),
            weights: candidate_clipped.clone(),
            top1_accuracy: metrics.top1_accuracy,
            top3_recall: metrics.top3_recall,
            created_at: Utc::now(),
            is_active: true,
        };

        self.store.save_checkpoint(&checkpoint)?;
        info!(
            version = %new_version,
            acc = metrics.top1_accuracy,
            "Promoted new model checkpoint successfully"
        );

        Ok(Some((candidate_clipped, new_version, metrics)))
    }

    /// Rolls back active model to target version or previous checkpoint.
    pub fn rollback(&self, target_version: Option<&str>) -> Result<ModelCheckpoint> {
        self.store.rollback_checkpoint(target_version)
    }

    /// Returns high-level statistics and active checkpoint metadata.
    pub fn stats(&self) -> Result<LearningStats> {
        let total_observations = self.store.count_observations()?;
        let checkpoints = self.store.list_checkpoints()?;
        let active = self
            .store
            .get_active_checkpoint()?
            .unwrap_or_else(|| ModelCheckpoint {
                version: "none".into(),
                weights: RankingWeights::default(),
                top1_accuracy: 0.0,
                top3_recall: 0.0,
                created_at: Utc::now(),
                is_active: false,
            });

        Ok(LearningStats {
            total_observations,
            active_version: active.version,
            top1_accuracy: active.top1_accuracy,
            top3_recall: active.top3_recall,
            active_weights: active.weights,
            available_checkpoints: checkpoints.len(),
        })
    }

    /// Access underlying store for maintenance.
    pub fn store(&self) -> &Arc<ExperienceStore> {
        &self.store
    }
}

fn debug_info(msg: String) {
    tracing::debug!("{}", msg);
}
