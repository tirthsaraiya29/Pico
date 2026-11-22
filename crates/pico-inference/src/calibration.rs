//! Calibrated probability calculation, Platt scaling, and confidence thresholding for candidate actions.
//!
//! Replaces raw Softmax distributions with genuine probability calibration methods:
//! - Platt scaling (logistic sigmoid calibration) for candidate executability confidence
//! - Temperature-scaled Softmax for multi-class candidate distributions
//! - Expected Calibration Error (ECE) and Brier score evaluation

use serde::{Deserialize, Serialize};

/// Parameters for Platt scaling: P(Y=1|s) = 1 / (1 + exp(-(A * s + B)))
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlattParameters {
    pub a: f32,
    pub b: f32,
}

impl Default for PlattParameters {
    fn default() -> Self {
        // Calibrated empirical defaults for Pico composite score distributions [0.0, 1.0]
        Self { a: 10.0, b: -5.0 }
    }
}

/// Evaluation metrics for probability calibration quality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationMetrics {
    pub expected_calibration_error: f32,
    pub max_calibration_error: f32,
    pub brier_score: f32,
}

/// Calibrates raw scores into valid, calibrated probabilities.
pub struct Calibrator;

impl Calibrator {
    /// Applies Platt scaling (logistic sigmoid) to map a raw candidate score to a calibrated probability.
    /// P(executable | score) = 1 / (1 + exp(-(A * score + B)))
    pub fn platt_calibrate(score: f32, params: &PlattParameters) -> f32 {
        let logit = params.a * score + params.b;
        // Numerically stable sigmoid
        if logit >= 0.0 {
            1.0 / (1.0 + (-logit).exp())
        } else {
            let ez = logit.exp();
            ez / (1.0 + ez)
        }
    }

    /// Applies Platt scaling across a slice of candidate scores.
    pub fn platt_calibrate_slice(scores: &[f32], params: &PlattParameters) -> Vec<f32> {
        scores
            .iter()
            .map(|&s| Self::platt_calibrate(s, params))
            .collect()
    }

    /// Applies temperature-scaled Softmax over raw candidate scores to yield a multi-class distribution.
    pub fn softmax_calibrate(scores: &[f32], temperature: f32) -> Vec<f32> {
        if scores.is_empty() {
            return Vec::new();
        }

        let temp = if temperature <= 0.0 { 1.0 } else { temperature };
        let scaled: Vec<f32> = scores.iter().map(|&s| s / temp).collect();

        // Numerically stable softmax: subtract max before exponentiating
        let max_val = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = scaled.iter().map(|&s| (s - max_val).exp()).collect();
        let sum_exp: f32 = exps.iter().sum();

        if sum_exp < 1e-12 {
            vec![1.0 / (scores.len() as f32); scores.len()]
        } else {
            exps.iter()
                .map(|&e| (e / sum_exp).clamp(0.0, 1.0))
                .collect()
        }
    }

    /// Combines Platt calibrated individual executability probability with temperature-scaled
    /// multi-class softmax to compute final calibrated candidate probabilities.
    pub fn calibrated_probabilities(
        raw_scores: &[f32],
        temperature: f32,
        platt: &PlattParameters,
    ) -> Vec<f32> {
        if raw_scores.is_empty() {
            return Vec::new();
        }

        let softmax_probs = Self::softmax_calibrate(raw_scores, temperature);
        let platt_probs = Self::platt_calibrate_slice(raw_scores, platt);

        // Weighted blend of relative rank (softmax) and absolute confidence (platt)
        // normalized so the distribution remains well-calibrated
        let mut blended: Vec<f32> = softmax_probs
            .iter()
            .zip(platt_probs.iter())
            .map(|(&s, &p)| (0.6 * s + 0.4 * p).max(0.0))
            .collect();

        let sum: f32 = blended.iter().sum();
        if sum > 1e-12 {
            for v in &mut blended {
                *v = (*v / sum).clamp(0.0, 1.0);
            }
        }
        blended
    }

    /// Evaluates decision margin between the top two probabilities.
    pub fn compute_confidence_margin(probabilities: &[f32]) -> f32 {
        if probabilities.is_empty() {
            return 0.0;
        }
        if probabilities.len() == 1 {
            return probabilities[0];
        }

        let mut sorted = probabilities.to_vec();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        (sorted[0] - sorted[1]).clamp(0.0, 1.0)
    }

    /// Calculates Expected Calibration Error (ECE) and Brier score across M equal-frequency/width bins.
    /// `samples` consists of pairs of (predicted_probability, actual_binary_label).
    pub fn evaluate_calibration(samples: &[(f32, bool)], num_bins: usize) -> CalibrationMetrics {
        if samples.is_empty() || num_bins == 0 {
            return CalibrationMetrics {
                expected_calibration_error: 0.0,
                max_calibration_error: 0.0,
                brier_score: 0.0,
            };
        }

        let n = samples.len() as f32;
        let mut brier_sum = 0.0f32;

        for &(prob, label) in samples {
            let y = if label { 1.0f32 } else { 0.0f32 };
            let diff = prob - y;
            brier_sum += diff * diff;
        }
        let brier_score = brier_sum / n;

        let bin_width = 1.0 / num_bins as f32;
        let mut ece = 0.0f32;
        let mut mce = 0.0f32;

        for b in 0..num_bins {
            let bin_lower = b as f32 * bin_width;
            let bin_upper = (b + 1) as f32 * bin_width;

            let in_bin: Vec<(f32, bool)> = samples
                .iter()
                .cloned()
                .filter(|&(p, _)| {
                    if b == num_bins - 1 {
                        p >= bin_lower && p <= bin_upper
                    } else {
                        p >= bin_lower && p < bin_upper
                    }
                })
                .collect();

            if in_bin.is_empty() {
                continue;
            }

            let bin_size = in_bin.len() as f32;
            let avg_conf: f32 = in_bin.iter().map(|&(p, _)| p).sum::<f32>() / bin_size;
            let true_pos: f32 = in_bin.iter().filter(|&(_, y)| *y).count() as f32;
            let accuracy = true_pos / bin_size;

            let abs_diff = (accuracy - avg_conf).abs();
            ece += (bin_size / n) * abs_diff;
            if abs_diff > mce {
                mce = abs_diff;
            }
        }

        CalibrationMetrics {
            expected_calibration_error: ece,
            max_calibration_error: mce,
            brier_score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platt_calibration() {
        let params = PlattParameters::default();
        let prob_low = Calibrator::platt_calibrate(0.1, &params);
        let prob_mid = Calibrator::platt_calibrate(0.5, &params);
        let prob_high = Calibrator::platt_calibrate(0.9, &params);

        assert!(prob_low < prob_mid);
        assert!(prob_mid < prob_high);
        assert!((0.0..=1.0).contains(&prob_low));
        assert!((0.0..=1.0).contains(&prob_high));
    }

    #[test]
    fn test_softmax_calibration() {
        let scores = vec![4.0, 2.0, 1.0];
        let probs = Calibrator::softmax_calibrate(&scores, 1.0);

        assert_eq!(probs.len(), 3);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(probs[0] > probs[1]);
        assert!(probs[1] > probs[2]);

        let margin = Calibrator::compute_confidence_margin(&probs);
        assert!(margin > 0.0);
    }

    #[test]
    fn test_calibrated_probabilities_blended() {
        let scores = vec![0.85, 0.40, 0.15];
        let params = PlattParameters::default();
        let probs = Calibrator::calibrated_probabilities(&scores, 1.0, &params);

        assert_eq!(probs.len(), 3);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(probs[0] > probs[1]);
        assert!(probs[1] > probs[2]);
    }

    #[test]
    fn test_calibration_evaluation_ece() {
        let samples = vec![
            (0.9, true),
            (0.85, true),
            (0.8, true),
            (0.2, false),
            (0.1, false),
            (0.15, false),
        ];

        let metrics = Calibrator::evaluate_calibration(&samples, 5);
        assert!(metrics.expected_calibration_error < 0.25);
        assert!(metrics.brier_score < 0.10);
    }
}
