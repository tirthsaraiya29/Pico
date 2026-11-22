//! Calibrated probability calculation and confidence thresholding for candidate actions.

/// Calibrates raw scores into valid probability distributions via Softmax with temperature scaling.
pub struct Calibrator;

impl Calibrator {
    /// Applies temperature-scaled Softmax over raw candidate scores to yield calibrated probabilities.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
