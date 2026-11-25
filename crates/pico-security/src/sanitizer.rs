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
        Self::patterns().iter().any(|p| p.is_match(text))
    }

    /// Checks for dangerous shell metacharacters, NUL bytes, or subshells in command arguments.
    pub fn is_safe_shell_arg(arg: &str) -> bool {
        if arg.contains('\0') {
            return false;
        }
        let dangerous_tokens = [
            "|", ";", "&&", "||", "`", "$(", "${", "<(", ">(", ">", "<", "\n", "\r",
        ];
        for token in dangerous_tokens {
            if arg.contains(token) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_arg_blocks_nul_and_subshells() {
        assert!(InputSanitizer::is_safe_shell_arg("8.8.8.8"));
        assert!(!InputSanitizer::is_safe_shell_arg("foo\0bar"));
        assert!(!InputSanitizer::is_safe_shell_arg("${IFS}cat"));
        assert!(!InputSanitizer::is_safe_shell_arg("$(whoami)"));
    }
}
