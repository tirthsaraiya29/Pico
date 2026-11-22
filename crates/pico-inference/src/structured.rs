use pico_core::session::ToolCall;
use regex::Regex;
use std::sync::OnceLock;
use tracing::debug;

static JSON_BLOCK_REGEX: OnceLock<Regex> = OnceLock::new();

/// Parser and validator for structured tool call decisions from LLM output.
pub struct StructuredDecisionParser;

impl StructuredDecisionParser {
    fn block_regex() -> &'static Regex {
        JSON_BLOCK_REGEX.get_or_init(|| Regex::new(r"```(?:json)?\s*([\s\S]*?)\s*```").unwrap())
    }

    /// Attempts to extract and parse one or more ToolCalls from text.
    pub fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
        let mut calls = Vec::new();

        // 1. First attempt: check markdown fenced code blocks
        for cap in Self::block_regex().captures_iter(text) {
            if let Some(matched) = cap.get(1) {
                if let Some(call) = Self::try_parse_single_json(matched.as_str()) {
                    calls.push(call);
                } else if let Some(mut batch) = Self::try_parse_array_json(matched.as_str()) {
                    calls.append(&mut batch);
                }
            }
        }

        if !calls.is_empty() {
            return calls;
        }

        // 2. Second attempt: search for direct JSON objects in the raw text
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                if end > start {
                    let candidate = &text[start..=end];
                    if let Some(call) = Self::try_parse_single_json(candidate) {
                        calls.push(call);
                    }
                }
            }
        }

        calls
    }

    fn try_parse_single_json(raw: &str) -> Option<ToolCall> {
        let trimmed = raw.trim();
        let val: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        Self::convert_value_to_call(&val)
    }

    fn try_parse_array_json(raw: &str) -> Option<Vec<ToolCall>> {
        let trimmed = raw.trim();
        let val: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        if let Some(arr) = val.as_array() {
            let parsed: Vec<ToolCall> =
                arr.iter().filter_map(Self::convert_value_to_call).collect();
            if !parsed.is_empty() {
                return Some(parsed);
            }
        }
        None
    }

    fn convert_value_to_call(val: &serde_json::Value) -> Option<ToolCall> {
        // Supported formats:
        // Format A: { "type": "tool_call", "tool": "...", "arguments": { ... }, "reason": "..." }
        // Format B: { "tool": "...", "arguments": { ... } }
        // Format C: { "name": "...", "parameters": { ... } }
        // Format D (OpenAI native): { "id": "...", "function": { "name": "...", "arguments": "..." } }

        if let Some(func) = val.get("function") {
            let name = func.get("name")?.as_str()?;
            let args_val = match func.get("arguments") {
                Some(serde_json::Value::String(s)) => {
                    serde_json::from_str(s).unwrap_or(serde_json::json!({}))
                }
                Some(obj) => obj.clone(),
                None => serde_json::json!({}),
            };
            let id = val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("call_generated")
                .to_string();
            return Some(ToolCall {
                id,
                name: name.to_string(),
                arguments: args_val,
                reason: None,
            });
        }

        let tool_name = val
            .get("tool")
            .or_else(|| val.get("name"))
            .and_then(|v| v.as_str())?;

        let arguments = val
            .get("arguments")
            .or_else(|| val.get("parameters"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let reason = val
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        debug!(tool = %tool_name, "Parsed structured tool call from model output");

        Some(ToolCall {
            id: format!("call_{}", uuid::Uuid::new_v4().simple()),
            name: tool_name.to_string(),
            arguments,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_structured_markdown_json() {
        let input = r#"
I will check the system info for you.
```json
{
  "type": "tool_call",
  "tool": "get_system_info",
  "arguments": {},
  "reason": "User requested hardware status"
}
```
"#;
        let calls = StructuredDecisionParser::parse_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_system_info");
        assert_eq!(
            calls[0].reason.as_deref(),
            Some("User requested hardware status")
        );
    }

    #[test]
    fn test_parse_raw_json_decision() {
        let input = r#"{"tool": "open_application", "arguments": {"application": "notepad"}}"#;
        let calls = StructuredDecisionParser::parse_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "open_application");
        assert_eq!(calls[0].arguments["application"], "notepad");
    }
}
