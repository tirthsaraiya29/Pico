//! Data poisoning defense, anomaly detection, and bounded weight updates.

use chrono::{DateTime, Duration, Utc};
use pico_core::error::{PicoError, Result};
use pico_inference::ranking::RankingWeights;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::warn;

#[derive(Debug, Serialize, Deserialize)]
struct PersistedGuardState {
    timestamps_rfc3339: Vec<String>,
    checksum: u64,
}

pub struct AntiPoisoningGuard {
    correction_history: Mutex<VecDeque<DateTime<Utc>>>,
    max_corrections_per_minute: usize,
    max_weight_delta: f32,
    state_path: Option<PathBuf>,
}

impl Default for AntiPoisoningGuard {
    fn default() -> Self {
        Self::new(5, 0.15)
    }
}

impl AntiPoisoningGuard {
    pub fn new(max_corrections_per_minute: usize, max_weight_delta: f32) -> Self {
        Self::with_state_path(max_corrections_per_minute, max_weight_delta, None)
    }

    pub fn with_state_path(
        max_corrections_per_minute: usize,
        max_weight_delta: f32,
        state_path: Option<PathBuf>,
    ) -> Self {
        let initial_history = state_path
            .as_deref()
            .map(|p| Self::load_history(p, max_corrections_per_minute))
            .unwrap_or_default();
        Self {
            correction_history: Mutex::new(initial_history),
            max_corrections_per_minute,
            max_weight_delta,
            state_path,
        }
    }

    fn compute_checksum(timestamps: &[String]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for ts in timestamps {
            for b in ts.as_bytes() {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn load_history(path: &Path, max_per_min: usize) -> VecDeque<DateTime<Utc>> {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return VecDeque::new();
        };
        let Ok(parsed) = serde_json::from_str::<PersistedGuardState>(&raw) else {
            warn!(path = %path.display(), "Corrupt anti-poisoning state file; applying fail-safe cooldown");
            return Self::cooldown_window(max_per_min);
        };
        if parsed.checksum != Self::compute_checksum(&parsed.timestamps_rfc3339) {
            warn!(path = %path.display(), "Anti-poisoning state checksum mismatch; applying fail-safe cooldown");
            return Self::cooldown_window(max_per_min);
        }

        let now = Utc::now();
        let one_min_ago = now - Duration::minutes(1);
        let max_future = now + Duration::seconds(60);
        let mut queue = VecDeque::new();
        for ts_str in &parsed.timestamps_rfc3339 {
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts_str) {
                let utc = dt.with_timezone(&Utc);
                if utc >= one_min_ago && utc <= max_future {
                    queue.push_back(utc);
                }
            }
        }
        queue
    }

    fn cooldown_window(count: usize) -> VecDeque<DateTime<Utc>> {
        let now = Utc::now();
        let mut q = VecDeque::with_capacity(count);
        for _ in 0..count {
            q.push_back(now);
        }
        q
    }

    fn persist_history(path: &Path, history: &VecDeque<DateTime<Utc>>) {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let timestamps_rfc3339: Vec<String> = history.iter().map(|dt| dt.to_rfc3339()).collect();
        let checksum = Self::compute_checksum(&timestamps_rfc3339);
        let state = PersistedGuardState {
            timestamps_rfc3339,
            checksum,
        };
        if let Ok(payload) = serde_json::to_vec(&state) {
            let tmp = path.with_extension("guard.tmp");
            if std::fs::write(&tmp, payload).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(path);
                let _ = std::fs::rename(&tmp, path);
            }
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
        if let Some(ref path) = self.state_path {
            Self::persist_history(path, &history);
        }
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
    fn test_persisted_rate_limiting_across_restarts_and_tamper_detection() {
        let path = std::env::temp_dir().join(format!(
            "pico_guard_test_{}_{}.json",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        {
            let guard = AntiPoisoningGuard::with_state_path(2, 0.1, Some(path.clone()));
            assert!(guard.validate_correction_rate().is_ok());
            assert!(guard.validate_correction_rate().is_ok());
            assert!(guard.validate_correction_rate().is_err());
        }
        // New instance simulates process restart: rate limit must still be enforced.
        {
            let guard_restarted = AntiPoisoningGuard::with_state_path(2, 0.1, Some(path.clone()));
            assert!(guard_restarted.validate_correction_rate().is_err());
        }
        // Tampered file triggers fail-safe cooldown.
        std::fs::write(&path, r#"{"timestamps_rfc3339":[],"checksum":123}"#).unwrap();
        {
            let guard_tampered = AntiPoisoningGuard::with_state_path(2, 0.1, Some(path.clone()));
            assert!(guard_tampered.validate_correction_rate().is_err());
        }
        let _ = std::fs::remove_file(&path);
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
