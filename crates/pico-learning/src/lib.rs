//! `pico-learning`: Continuous On-Device Experience Learning Subsystem.
//! Collects structured interaction tuples in SQLite, evaluates training quality hierarchies,
//! executes lightweight ranking optimization, enforces anti-poisoning guards, and manages versioned checkpoints.

pub mod evaluator;
pub mod manager;
pub mod observation;
pub mod poisoning;
pub mod store;
pub mod trainer;

pub use evaluator::{EvaluationMetrics, ModelEvaluator};
pub use manager::{LearningManager, LearningStats};
pub use observation::{LabelQuality, ObservationRecord};
pub use poisoning::AntiPoisoningGuard;
pub use store::{ExperienceStore, ModelCheckpoint};
pub use trainer::RankingTrainer;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pico_core::config::LearningConfig;
    use pico_inference::ranking::RankingWeights;

    #[test]
    fn test_in_memory_experience_store() {
        let store = ExperienceStore::in_memory().unwrap();

        let record = ObservationRecord {
            id: "obs-123".to_string(),
            timestamp: Utc::now(),
            request_text: "What is 20 + 20?".to_string(),
            primary_intent: "arithmetic_computation".to_string(),
            candidate_names: vec!["calculator".to_string()],
            candidate_scores: vec![0.95],
            selected_action: "calculator".to_string(),
            confidence: 0.95,
            execution_status: "success".to_string(),
            latency_ms: 12,
            user_feedback: Some(1),
            corrected_action: None,
            quality: LabelQuality::ExplicitFeedback,
        };

        store.insert_observation(&record).unwrap();
        assert_eq!(store.count_observations().unwrap(), 1);

        let training = store
            .get_training_observations(LabelQuality::ExecutionOutcome, 10)
            .unwrap();
        assert_eq!(training.len(), 1);
        assert_eq!(training[0].selected_action, "calculator");
    }

    #[test]
    fn test_checkpoint_and_rollback() {
        let cfg = LearningConfig {
            min_observations_for_retraining: 1,
            ..Default::default()
        };
        let manager = LearningManager::in_memory(cfg).unwrap();

        let initial_stats = manager.stats().unwrap();
        assert_eq!(initial_stats.active_version, "pico-model-v1.0.0");

        // Save a second checkpoint
        let cp2 = ModelCheckpoint {
            version: "pico-model-v1.0.1".to_string(),
            weights: RankingWeights::default(),
            top1_accuracy: 0.95,
            top3_recall: 1.0,
            created_at: Utc::now(),
            is_active: true,
        };
        manager.store().save_checkpoint(&cp2).unwrap();

        let stats2 = manager.stats().unwrap();
        assert_eq!(stats2.active_version, "pico-model-v1.0.1");

        // Rollback to initial
        let rolled_back = manager.rollback(Some("pico-model-v1.0.0")).unwrap();
        assert_eq!(rolled_back.version, "pico-model-v1.0.0");

        let stats_after = manager.stats().unwrap();
        assert_eq!(stats_after.active_version, "pico-model-v1.0.0");
    }

    #[test]
    fn test_evaluator_across_all_22_domains() {
        let evaluator = ModelEvaluator::new();
        assert!(
            evaluator.sample_count() >= 44,
            "Benchmark must contain at least 44 test cases across all domains, got {}",
            evaluator.sample_count()
        );

        let weights = RankingWeights::default();
        let metrics = evaluator.evaluate(&weights, 0.70).unwrap();

        assert!(
            metrics.top1_accuracy >= 0.70,
            "Top-1 accuracy should meet target threshold, got {}",
            metrics.top1_accuracy
        );
        assert!(
            metrics.top3_recall >= 0.85,
            "Top-3 recall should be >= 85%, got {}",
            metrics.top3_recall
        );
        assert!(metrics.passed_promotion);
    }
}
