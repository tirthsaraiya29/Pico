//! Held-out benchmark evaluation suite and model promotion gate.

use pico_core::error::Result;
use pico_core::traits::{CandidateAction, PermissionTier};
use pico_inference::ranking::{CrossEncoderRanker, RankingWeights};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct BenchmarkItem {
    pub query: &'static str,
    pub expected_action: &'static str,
    pub candidate_actions: Vec<CandidateAction>,
    pub intent_affinities: Vec<f32>,
    pub historical_scores: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub top1_accuracy: f32,
    pub top3_recall: f32,
    pub total_samples: usize,
    pub passed_promotion: bool,
}

pub struct ModelEvaluator {
    benchmark_dataset: Vec<BenchmarkItem>,
}

impl Default for ModelEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelEvaluator {
    pub fn new() -> Self {
        Self {
            benchmark_dataset: build_standard_benchmark(),
        }
    }

    /// Evaluates candidate weights against the held-out benchmark suite.
    pub fn evaluate(
        &self,
        weights: &RankingWeights,
        baseline_top1: f32,
    ) -> Result<EvaluationMetrics> {
        let ranker = CrossEncoderRanker::new(weights.clone());
        let mut top1_correct = 0usize;
        let mut top3_correct = 0usize;
        let total = self.benchmark_dataset.len();

        for item in &self.benchmark_dataset {
            let ranked = ranker.rank_candidates(
                item.query,
                &item.candidate_actions,
                &item.intent_affinities,
                &item.historical_scores,
                1.0,
            );

            if let Some(first) = ranked.first() {
                if first.action_name == item.expected_action {
                    top1_correct += 1;
                }
            }

            let top3_names: Vec<&str> = ranked
                .iter()
                .take(3)
                .map(|c| c.action_name.as_str())
                .collect();
            if top3_names.contains(&item.expected_action) {
                top3_correct += 1;
            }
        }

        let top1_accuracy = if total > 0 {
            top1_correct as f32 / total as f32
        } else {
            0.0
        };

        let top3_recall = if total > 0 {
            top3_correct as f32 / total as f32
        } else {
            0.0
        };

        // Promotion criterion: No severe regression from baseline (< -2%) and Top-1 >= 70%
        let passed_promotion = top1_accuracy >= (baseline_top1 - 0.02) && top1_accuracy >= 0.70;

        Ok(EvaluationMetrics {
            top1_accuracy,
            top3_recall,
            total_samples: total,
            passed_promotion,
        })
    }
}

fn build_standard_benchmark() -> Vec<BenchmarkItem> {
    let make_action = |name: &'static str, desc: &'static str, cat: &'static str| CandidateAction {
        name: name.to_string(),
        description: desc.to_string(),
        category: cat.to_string(),
        permission_tier: PermissionTier::ReadOnly,
        parameters_schema: serde_json::json!({}),
    };

    vec![
        BenchmarkItem {
            query: "What is 15 * 12?",
            expected_action: "calculator",
            candidate_actions: vec![
                make_action(
                    "calculator",
                    "Evaluates mathematical arithmetic expressions",
                    "calculator",
                ),
                make_action(
                    "sys.diagnostic",
                    "Inspect hardware and system utilization",
                    "system",
                ),
                make_action(
                    "fs.read_file",
                    "Read text file from filesystem",
                    "filesystem",
                ),
            ],
            intent_affinities: vec![1.0, 0.1, 0.05],
            historical_scores: vec![0.9, 0.5, 0.5],
        },
        BenchmarkItem {
            query: "How much memory is used?",
            expected_action: "sys.diagnostic",
            candidate_actions: vec![
                make_action(
                    "sys.diagnostic",
                    "Inspect CPU and RAM memory utilization",
                    "system",
                ),
                make_action(
                    "calculator",
                    "Evaluates mathematical arithmetic expressions",
                    "calculator",
                ),
                make_action("media.volume", "Set master audio volume level", "audio"),
            ],
            intent_affinities: vec![1.0, 0.05, 0.1],
            historical_scores: vec![0.9, 0.5, 0.5],
        },
        BenchmarkItem {
            query: "Kill unresponsive chrome process",
            expected_action: "process.kill",
            candidate_actions: vec![
                make_action(
                    "process.kill",
                    "Terminate a running process by name or PID",
                    "process",
                ),
                make_action(
                    "sys.diagnostic",
                    "Inspect CPU and RAM memory utilization",
                    "system",
                ),
                make_action(
                    "fs.read_file",
                    "Read text file from filesystem",
                    "filesystem",
                ),
            ],
            intent_affinities: vec![1.0, 0.4, 0.1],
            historical_scores: vec![0.85, 0.5, 0.5],
        },
        BenchmarkItem {
            query: "Set volume to 50 percent",
            expected_action: "media.volume",
            candidate_actions: vec![
                make_action("media.volume", "Set master audio volume level", "audio"),
                make_action(
                    "calculator",
                    "Evaluates mathematical arithmetic expressions",
                    "calculator",
                ),
                make_action("process.kill", "Terminate a running process", "process"),
            ],
            intent_affinities: vec![1.0, 0.1, 0.05],
            historical_scores: vec![0.9, 0.5, 0.5],
        },
        BenchmarkItem {
            query: "Read document at C:\\notes.txt",
            expected_action: "fs.read_file",
            candidate_actions: vec![
                make_action(
                    "fs.read_file",
                    "Read text file from filesystem",
                    "filesystem",
                ),
                make_action("media.volume", "Set master audio volume level", "audio"),
                make_action(
                    "calculator",
                    "Evaluates mathematical arithmetic expressions",
                    "calculator",
                ),
            ],
            intent_affinities: vec![1.0, 0.05, 0.05],
            historical_scores: vec![0.9, 0.5, 0.5],
        },
    ]
}
