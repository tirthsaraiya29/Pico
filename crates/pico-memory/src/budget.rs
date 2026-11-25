use pico_core::session::{Message, Role};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct BudgetedHistory {
    pub messages: Vec<Message>,
    pub total_tokens: usize,
    pub truncated_count: usize,
    pub dropped_tokens: usize,
    pub was_truncated: bool,
}

pub struct ContextBudgeter;

impl ContextBudgeter {
    /// Estimates BPE-like token count across ASCII prose, Unicode/CJK, emoji, and technical text.
    ///
    /// Heuristics:
    /// - CJK/Hangul/Kana/Thai/Arabic characters typically map to ~1.0–1.5 tokens each.
    /// - Astral-plane emoji/symbols map to ~1.5–2 tokens each.
    /// - Dense technical symbols/punctuation (`{}[]()=<>:;/\\|`) and camelCase/snake_case
    ///   splits add subword boundaries (~1 token per 3–4 chars or per identifier segment).
    /// - Standard ASCII prose averages ~1.3 tokens per whitespace-delimited word.
    pub fn estimate_tokens(text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        let mut units: f64 = 0.0;
        let mut ascii_word_chars: usize = 0;
        let mut prev_lower = false;

        for ch in text.chars() {
            if ch.is_whitespace() {
                if ascii_word_chars > 0 {
                    units += (ascii_word_chars as f64 / 4.0).max(1.0);
                    ascii_word_chars = 0;
                }
                prev_lower = false;
                continue;
            }

            if is_cjk_or_complex_script(ch) {
                if ascii_word_chars > 0 {
                    units += (ascii_word_chars as f64 / 4.0).max(1.0);
                    ascii_word_chars = 0;
                }
                units += 1.25;
                prev_lower = false;
            } else if ch as u32 > 0xFFFF {
                if ascii_word_chars > 0 {
                    units += (ascii_word_chars as f64 / 4.0).max(1.0);
                    ascii_word_chars = 0;
                }
                units += 1.8;
                prev_lower = false;
            } else if ch.is_ascii_alphanumeric() {
                // Count camelCase transitions as subword boundaries.
                if prev_lower && ch.is_ascii_uppercase() && ascii_word_chars > 0 {
                    units += 0.45;
                }
                ascii_word_chars += 1;
                prev_lower = ch.is_ascii_lowercase();
            } else if ch.is_ascii_punctuation() {
                if ascii_word_chars > 0 {
                    units += (ascii_word_chars as f64 / 4.0).max(1.0);
                    ascii_word_chars = 0;
                }
                units += 0.75;
                prev_lower = false;
            } else {
                // Accented Latin / Cyrillic / Greek etc.
                ascii_word_chars += 1;
                prev_lower = ch.is_lowercase();
            }
        }

        if ascii_word_chars > 0 {
            units += (ascii_word_chars as f64 / 4.0).max(1.0);
        }

        (units.ceil() as usize).max(1)
    }

    /// Trims messages to fit within `budget_tokens`, preserving important system instructions
    /// first and filling the remaining budget with the most recent conversation turns.
    pub fn budget_messages(messages: &[Message], budget_tokens: usize) -> Vec<Message> {
        Self::budget_messages_with_report(messages, budget_tokens).messages
    }

    /// Budgets messages while explicitly reporting how many messages/tokens were truncated.
    pub fn budget_messages_with_report(
        messages: &[Message],
        budget_tokens: usize,
    ) -> BudgetedHistory {
        if messages.is_empty() {
            return BudgetedHistory {
                messages: Vec::new(),
                total_tokens: 0,
                truncated_count: 0,
                dropped_tokens: 0,
                was_truncated: false,
            };
        }

        let token_costs: Vec<usize> = messages
            .iter()
            .map(|m| {
                m.token_count
                    .unwrap_or_else(|| Self::estimate_tokens(&m.content))
            })
            .collect();

        let mut keep_indices = HashSet::new();
        let mut used_tokens = 0usize;

        // 1. Reserve budget for system instructions first (in chronological order).
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == Role::System {
                let cost = token_costs[idx];
                if used_tokens + cost <= budget_tokens || keep_indices.is_empty() {
                    keep_indices.insert(idx);
                    used_tokens += cost;
                }
            }
        }

        // 2. Fill remaining budget from most recent non-system turn backwards.
        let mut added_recent = false;
        for idx in (0..messages.len()).rev() {
            if messages[idx].role == Role::System {
                continue;
            }
            let cost = token_costs[idx];
            if used_tokens + cost > budget_tokens && (added_recent || !keep_indices.is_empty()) {
                break;
            }
            keep_indices.insert(idx);
            used_tokens += cost;
            added_recent = true;
        }

        let mut selected = Vec::with_capacity(keep_indices.len());
        let mut dropped_tokens = 0usize;
        for (idx, msg) in messages.iter().enumerate() {
            if keep_indices.contains(&idx) {
                selected.push(msg.clone());
            } else {
                dropped_tokens += token_costs[idx];
            }
        }

        let truncated_count = messages.len().saturating_sub(selected.len());
        BudgetedHistory {
            messages: selected,
            total_tokens: used_tokens,
            truncated_count,
            dropped_tokens,
            was_truncated: truncated_count > 0,
        }
    }

    /// Formats retrieved long-term memory facts into a system context snippet.
    pub fn format_memory_context(facts: &[String]) -> Option<String> {
        if facts.is_empty() {
            return None;
        }

        let mut block = String::from("\n[Relevant Long-term User Context]:\n");
        for fact in facts {
            block.push_str(&format!("- {}\n", fact));
        }
        Some(block)
    }
}

fn is_cjk_or_complex_script(ch: char) -> bool {
    let cp = ch as u32;
    matches!(
        cp,
        0x3040..=0x30FF // Hiragana & Katakana
            | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
            | 0x4E00..=0x9FFF // CJK Unified Ideographs
            | 0xAC00..=0xD7AF // Hangul Syllables
            | 0x0600..=0x06FF // Arabic
            | 0x0E00..=0x0E7F // Thai
            | 0xF900..=0xFAFF // CJK Compatibility Ideographs
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::session::SessionId;

    #[test]
    fn test_token_estimation_unicode_cjk_emoji_and_technical() {
        assert_eq!(ContextBudgeter::estimate_tokens(""), 0);
        // CJK: 6 characters should estimate >= 6 tokens (not 18 bytes / 4 = 5)
        let cjk = ContextBudgeter::estimate_tokens("こんにちは世界");
        assert!(cjk >= 7, "expected >= 7 tokens for CJK, got {cjk}");

        // Emoji: 4 emoji should estimate >= 4 tokens
        let emoji = ContextBudgeter::estimate_tokens("🦀🚀🔥🎉");
        assert!(emoji >= 6, "expected >= 6 tokens for emoji, got {emoji}");

        // Technical text with camelCase and punctuation
        let tech =
            ContextBudgeter::estimate_tokens("fn fooBarBaz(x: &str) -> Result<Vec<usize>, Error>");
        assert!(tech >= 12, "expected >= 12 tokens for code, got {tech}");
    }

    #[test]
    fn test_budget_messages_preserves_system_and_reports_truncation() {
        let sid = SessionId::new();
        let msgs = vec![
            Message::new_system(sid, "You are Pico, a helpful assistant."),
            Message::new_user(sid, "Old message 1 with extra words to consume budget"),
            Message::new_assistant(sid, "Old reply 1 with extra words to consume budget", None),
            Message::new_user(sid, "Recent message 2"),
            Message::new_assistant(sid, "Recent reply 2", None),
        ];

        let report = ContextBudgeter::budget_messages_with_report(&msgs, 22);
        assert!(report.was_truncated);
        assert!(report.truncated_count >= 1);
        assert!(report.dropped_tokens > 0);
        assert_eq!(report.messages[0].role, Role::System);
        assert_eq!(report.messages.last().unwrap().content, "Recent reply 2");
    }
}
