use regex::Regex;
use std::sync::OnceLock;

/// Sanitizer detecting prompt injection attempts and dangerous character patterns.
pub struct InputSanitizer;

static INJECTION_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

impl InputSanitizer {
    fn patterns() -> &'static Vec<Regex> {
        INJECTION_PATTERNS.get_or_init(|| {
            vec![
                Regex::new(r"(?i)ignore\s+(all\s+)?(previous|prior)\s+instructions").unwrap(),
                Regex::new(r"(?i)disregard\s+(all\s+)?(previous|prior)\s+system\s+prompts")
                    .unwrap(),
                Regex::new(r"(?i)you\s+are\s+now\s+in\s+developer\s+mode").unwrap(),
                Regex::new(r"(?i)bypass\s+all\s+security\s+checks").unwrap(),
                Regex::new(r"(?i)act\s+as\s+an\s+unrestricted\s+ai").unwrap(),
                Regex::new(r"(?i)system\s*:\s*override").unwrap(),
            ]
        })
    }

    /// Checks if a text string contains recognizable prompt injection patterns.
    pub fn contains_prompt_injection(text: &str) -> bool {
        for pattern in Self::patterns() {
            if pattern.is_match(text) {
                return true;
            }
        }
        false
    }

    /// Checks for dangerous shell metacharacters in command argument strings.
    pub fn is_safe_shell_arg(arg: &str) -> bool {
        // Disallow unescaped chained command separators, subshells, output redirectors
        let dangerous_tokens = ["|", ";", "&&", "||", "`", "$(", ">", "<", "\n", "\r"];
        for token in dangerous_tokens {
            if arg.contains(token) {
                return false;
            }
        }
        true
    }
}
