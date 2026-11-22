use pico_core::session::Message;

pub struct ContextBudgeter;

impl ContextBudgeter {
    /// Rough token estimation (~4 characters per token in English).
    pub fn estimate_tokens(text: &str) -> usize {
        text.len().div_ceil(4)
    }

    /// Trims messages to fit within a given token budget, keeping most recent turns.
    pub fn budget_messages(messages: &[Message], budget_tokens: usize) -> Vec<Message> {
        if messages.is_empty() {
            return Vec::new();
        }

        let mut total_tokens = 0;
        let mut selected = Vec::new();

        // Traverse backwards from most recent to oldest
        for msg in messages.iter().rev() {
            let msg_tokens = msg
                .token_count
                .unwrap_or_else(|| Self::estimate_tokens(&msg.content));

            if total_tokens + msg_tokens > budget_tokens && !selected.is_empty() {
                break;
            }

            total_tokens += msg_tokens;
            selected.push(msg.clone());
        }

        selected.reverse();
        selected
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

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::session::SessionId;

    #[test]
    fn test_budget_messages() {
        let sid = SessionId::new();
        let msgs = vec![
            Message::new_user(sid, "Old message 1"),
            Message::new_assistant(sid, "Old reply 1", None),
            Message::new_user(sid, "Recent message 2"),
            Message::new_assistant(sid, "Recent reply 2", None),
        ];

        let budgeted = ContextBudgeter::budget_messages(&msgs, 15);
        assert!(!budgeted.is_empty());
        assert_eq!(budgeted.last().unwrap().content, "Recent reply 2");
    }
}
