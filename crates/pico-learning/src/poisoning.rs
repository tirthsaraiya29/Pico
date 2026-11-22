//! Data poisoning defense, anomaly detection, and bounded weight updates.

use chrono::{DateTime, Duration, Utc};
use pico_core::error::{PicoError, Result};
use pico_inference::ranking::RankingWeights;
use std::collections::VecDeque;
use std::sync::Mutex;
use tracing::warn;

pub struct AntiPoisoningGuard {
    correction_history: Mutex<VecDeque<DateTime<Utc>>>,
    max_corrections_per_minute: usize,
    max_weight_delta: f32,
}

impl Default for AntiPoisoningGuard {
    fn default() -> Self {
        Self::new(5, 0.15)
    }
}

impl AntiPoisoningGuard {
    pub fn new(max_corrections_per_minute: usize, max_weight_delta: f32) -> Self {
        Self {
            correction_history: Mutex::new(VecDeque::new()),
            max_corrections_per_minute,
            max_weight_delta,
        }
    }

    /// Verifies that a user correction is not part of a rapid-fire poisoning burst.
    pub fn validate_correction_rate(&self) -> Result<()> {
        let mut history = self.correction_history.lock().unwrap();
        let now = Utc::now();
        let one_min_ago = now - Duration::minutes(1);

        // Evict older entries
        while let Some(front) = history.front() {
            if *front < one_min_ago {
                history.pop_front();
            } else {
                break;
            }
        }

        if history.len() >= self.max_corrections_per_minute {
            warn!(
                count = history.len(),
                "Rejected correction: rate limit exceeded (possible poisoning attempt)"
            );
            return Err(PicoError::Security(
                "Correction rate limit exceeded. Please wait a moment before correcting again."
                    .into(),
            ));
        }

        history.push_back(now);
        Ok(())
    }

    /// Clips and validates proposed weight deltas to prevent sudden catastrophic drift.
    pub fn clip_weights(
        &self,
        current: &RankingWeights,
        proposed: &RankingWeights,
    ) -> RankingWeights {
        let clip_delta = |curr: f32, prop: f32| -> f32 {
            let delta = prop - curr;
            let clipped_delta = delta.clamp(-self.max_weight_delta, self.max_weight_delta);
            (curr + clipped_delta).clamp(0.01, 1.0)
        };

        RankingWeights {
            w_semantic: clip_delta(current.w_semantic, proposed.w_semantic),
            w_lexical: clip_delta(current.w_lexical, proposed.w_lexical),
            w_intent: clip_delta(current.w_intent, proposed.w_intent),
            w_history: clip_delta(current.w_history, proposed.w_history),
            bias: clip_delta(current.bias, proposed.bias).clamp(-0.5, 0.5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiting() {
        let guard = AntiPoisoningGuard::new(2, 0.1);
        assert!(guard.validate_correction_rate().is_ok());
        assert!(guard.validate_correction_rate().is_ok());
        assert!(guard.validate_correction_rate().is_err());
    }

    #[test]
    fn test_weight_clipping() {
        let guard = AntiPoisoningGuard::new(5, 0.1);
        let curr = RankingWeights {
            w_semantic: 0.35,
            w_lexical: 0.30,
            w_intent: 0.20,
            w_history: 0.15,
            bias: 0.0,
        };
        let proposed = RankingWeights {
            w_semantic: 0.90, // Delta +0.55 > max 0.10
            w_lexical: 0.05,  // Delta -0.25 < -0.10
            w_intent: 0.22,   // Delta +0.02 <= 0.10
            w_history: 0.15,
            bias: 0.0,
        };
        let clipped = guard.clip_weights(&curr, &proposed);
        assert!((clipped.w_semantic - 0.45).abs() < 1e-4);
        assert!((clipped.w_lexical - 0.20).abs() < 1e-4);
        assert!((clipped.w_intent - 0.22).abs() < 1e-4);
    }
}
