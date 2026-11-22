//! Hardened Voice Authorization Engine.
//! Protects dangerous and privileged tool execution against accidental approval,
//! phonetic ambiguities, sub-string collisions, and adversarial affirmative phrasing.

use regex::Regex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Outcome of evaluating spoken user utterance for authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceAuthDecision {
    /// Explicit, unambiguous confirmation received without any negative context.
    Approved,
    /// User explicitly refused, cancelled, or negated the action.
    Declined { reason: String },
    /// Utterance is unclear, ambiguous, or contains neither clear confirmation nor denial.
    Ambiguous { prompt: String },
}

/// Hardened voice authorization evaluator.
pub struct VoiceAuthorizer;

impl VoiceAuthorizer {
    /// Evaluates user speech utterance for authorization of a dangerous command.
    ///
    /// Strict Evaluation Invariants:
    /// 1. Negation priority: If any negative word ("no", "don't", "stop", "abort", "cancel") appears,
    ///    the request is IMMEDIATELY declined regardless of whether affirmative words also appear
    ///    (e.g., "No, do not do it" or "Don't confirm" are strictly DECLINED).
    /// 2. Strict word boundaries: Substrings like "ok" inside "broken" or "sure" inside "unsure"
    ///    never trigger false-positive approvals.
    /// 3. Affirmation requirement: Requires explicit affirmative tokens ("confirm", "proceed", "approve", "yes").
    pub fn evaluate_utterance(utterance: &str, tool_name: &str) -> VoiceAuthDecision {
        let clean = utterance.trim().to_lowercase();
        if clean.is_empty() {
            return VoiceAuthDecision::Ambiguous {
                prompt: format!(
                    "I didn't hear a response. Please say 'confirm' to proceed with {}, or 'cancel' to abort.",
                    tool_name
                ),
            };
        }

        static NEGATION_RE: OnceLock<Regex> = OnceLock::new();
        let negation_re = NEGATION_RE.get_or_init(|| {
            Regex::new(r"(?i)\b(no|not|don't|dont|never|stop|abort|cancel|cancelled|cancelling|deny|denied|refuse|refused|wait|negative|disagree|unsure|nah|nope)\b")
                .expect("Valid negation regex")
        });

        // 1. NEGATION CHECK (HIGHEST PRIORITY):
        // If the utterance contains ANY refusal/negation, decline immediately.
        if negation_re.is_match(&clean) {
            return VoiceAuthDecision::Declined {
                reason: format!(
                    "User explicitly declined or cancelled action '{}'",
                    tool_name
                ),
            };
        }

        static AFFIRMATIVE_RE: OnceLock<Regex> = OnceLock::new();
        let affirmative_re = AFFIRMATIVE_RE.get_or_init(|| {
            Regex::new(r"(?i)\b(confirm|confirmed|proceed|approve|approved|authorize|authorized|yes|yeah|yep|affirmative)\b")
                .expect("Valid affirmative regex")
        });

        static AFFIRMATIVE_PHRASES: OnceLock<Regex> = OnceLock::new();
        let phrase_re = AFFIRMATIVE_PHRASES.get_or_init(|| {
            Regex::new(r"(?i)\b(go ahead|do it|i agree|please do)\b").expect("Valid phrase regex")
        });

        // 2. AFFIRMATIVE MATCH:
        if affirmative_re.is_match(&clean) || phrase_re.is_match(&clean) {
            return VoiceAuthDecision::Approved;
        }

        // 3. AMBIGUOUS / UNCLEAR:
        VoiceAuthDecision::Ambiguous {
            prompt: format!(
                "I heard '{}'. Please say 'confirm' to proceed with {}, or 'cancel' to abort.",
                utterance.trim(),
                tool_name
            ),
        }
    }
}

/// State-tracked authorization request with strict single-use and timeout expiration.
#[derive(Debug)]
pub struct VoiceAuthRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub created_at: Instant,
    pub timeout: Duration,
    pub resolved: bool,
}

impl VoiceAuthRequest {
    pub fn new(
        request_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        timeout: Duration,
    ) -> Self {
        Self {
            request_id,
            tool_name,
            arguments,
            created_at: Instant::now(),
            timeout,
            resolved: false,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affirmative_authorizations() {
        let valid_confirms = [
            "confirm",
            "yes",
            "proceed",
            "please proceed",
            "confirm execution",
            "yes please",
            "authorize",
            "approved",
            "go ahead",
            "do it",
            "affirmative",
        ];

        for phrase in valid_confirms {
            let res = VoiceAuthorizer::evaluate_utterance(phrase, "kill_process");
            assert_eq!(
                res,
                VoiceAuthDecision::Approved,
                "Phrase '{}' should be approved",
                phrase
            );
        }
    }

    #[test]
    fn test_negation_priority_and_rejections() {
        let rejections = [
            "no",
            "cancel",
            "abort",
            "stop",
            "don't do it",
            "do not do it",
            "no don't proceed",
            "never do that",
            "no way",
            "deny",
            "wait, stop",
            "i am unsure",
            "nope",
            "nah",
        ];

        for phrase in rejections {
            let res = VoiceAuthorizer::evaluate_utterance(phrase, "kill_process");
            match res {
                VoiceAuthDecision::Declined { .. } => {}
                other => panic!("Phrase '{}' must be declined, got {:?}", phrase, other),
            }
        }
    }

    #[test]
    fn test_adversarial_affirmative_substrings_blocked() {
        // These utterances contain substrings that naive .contains("do it") or .contains("ok") would match
        let dangerous_traps = [
            "don't do it",
            "please do not do it",
            "never do it",
            "do not proceed",
            "it is broken",           // contains "ok"
            "looking at it",          // contains "ok"
            "spoke to him",           // contains "ok"
            "i am unsure about this", // contains "sure"
            "under pressure",         // contains "sure"
        ];

        for trap in dangerous_traps {
            let res = VoiceAuthorizer::evaluate_utterance(trap, "dangerous_tool");
            assert_ne!(
                res,
                VoiceAuthDecision::Approved,
                "Dangerous trap phrase '{}' was falsely approved!",
                trap
            );
        }
    }

    #[test]
    fn test_ambiguous_utterance_triggers_clarification() {
        let ambiguous_phrases = [
            "what was that",
            "tell me a joke",
            "who are you",
            "hello",
            "maybe later",
        ];

        for phrase in ambiguous_phrases {
            let res = VoiceAuthorizer::evaluate_utterance(phrase, "format_drive");
            match res {
                VoiceAuthDecision::Ambiguous { prompt } => {
                    assert!(prompt.contains("format_drive"));
                }
                other => panic!("Phrase '{}' should be ambiguous, got {:?}", phrase, other),
            }
        }
    }
}
