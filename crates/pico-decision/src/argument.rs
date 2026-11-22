//! Deterministic argument resolution and schema extraction without LLMs.
//! Extracts typed arguments (paths, expressions, PIDs, durations, URLs, flags) directly from user queries.

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

static RE_PATH_WIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:[a-zA-Z]:\\[^"<>|?*\r\n\t]+)|(?:[a-zA-Z]:/[^"<>|?*\r\n\t]+)"#).unwrap()
});

static RE_QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([^"']+)["']"#).unwrap());

static RE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s/$.?#].[^\s]*"#).unwrap());

static RE_PID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:pid|process\s+id|id)[:\s]*(\d+)"#).unwrap());

static RE_DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\d+)\s*(s|sec|seconds?|m|min|minutes?|h|hr|hours?)"#).unwrap());

static RE_PERCENTAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\d+)\s*%"#).unwrap());

/// Result of extracting arguments for a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentResolution {
    pub resolved_args: Value,
    pub missing_required: Vec<String>,
    pub confidence: f32,
}

/// Resolves typed tool arguments from a user request given the tool's parameter schema.
pub fn resolve_arguments(query: &str, tool_name: &str, schema: &Value) -> ArgumentResolution {
    let mut args = Map::new();
    let mut missing = Vec::new();
    let query_clean = query.trim();

    // Check required properties from JSON schema
    let required_fields: Vec<String> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let properties = schema.get("properties").and_then(|p| p.as_object());

    // Domain-specific extraction heuristics based on tool name and schema
    match tool_name {
        "calculator" | "math_eval" => {
            // Extract the mathematical expression
            let expr = extract_math_expression(query_clean);
            args.insert("expression".to_string(), json!(expr));
        }
        "file_read" | "file_write" | "file_delete" | "fs_stat" => {
            if let Some(path) = extract_path(query_clean) {
                args.insert("path".to_string(), json!(path));
            }
            if tool_name == "file_write" {
                if let Some(content) = extract_content_payload(query_clean) {
                    args.insert("content".to_string(), json!(content));
                }
            }
        }
        "kill_process" | "process_terminate" => {
            if let Some(pid) = extract_pid(query_clean) {
                args.insert("pid".to_string(), json!(pid));
            } else if let Some(proc_name) = extract_target_process_name(query_clean) {
                args.insert("name".to_string(), json!(proc_name));
            }
        }
        "set_volume" | "audio_volume" => {
            if let Some(pct) = extract_percentage_or_number(query_clean) {
                args.insert("level".to_string(), json!(pct));
            }
        }
        "set_timer" | "timer_create" => {
            if let Some(secs) = extract_duration_seconds(query_clean) {
                args.insert("seconds".to_string(), json!(secs));
            }
            let label = extract_timer_label(query_clean);
            args.insert("label".to_string(), json!(label));
        }
        "web_fetch" | "http_get" => {
            if let Some(url) = extract_url(query_clean) {
                args.insert("url".to_string(), json!(url));
            }
        }
        "open_application" | "app_launch" => {
            let app = extract_app_target(query_clean);
            args.insert("target".to_string(), json!(app));
        }
        "clipboard_set" => {
            let text = extract_quoted_or_trailing(query_clean, &["copy", "clipboard", "set"]);
            args.insert("text".to_string(), json!(text));
        }
        _ => {
            // General schema-driven extraction
            if let Some(props) = properties {
                for (prop_name, prop_def) in props {
                    let prop_type = prop_def
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("string");

                    match prop_type {
                        "integer" | "number" => {
                            if let Some(num) = extract_number(query_clean) {
                                args.insert(prop_name.clone(), json!(num));
                            }
                        }
                        "boolean" => {
                            let val = query_clean.contains("true")
                                || query_clean.contains("enable")
                                || query_clean.contains("on");
                            args.insert(prop_name.clone(), json!(val));
                        }
                        _ => {
                            if prop_name.contains("path") || prop_name.contains("file") {
                                if let Some(path) = extract_path(query_clean) {
                                    args.insert(prop_name.clone(), json!(path));
                                }
                            } else if prop_name.contains("url") {
                                if let Some(url) = extract_url(query_clean) {
                                    args.insert(prop_name.clone(), json!(url));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Verify all required fields are present
    for req in &required_fields {
        if !args.contains_key(req) {
            missing.push(req.clone());
        }
    }

    let confidence = if required_fields.is_empty() {
        1.0
    } else {
        let satisfied = required_fields.len().saturating_sub(missing.len());
        satisfied as f32 / required_fields.len() as f32
    };

    ArgumentResolution {
        resolved_args: Value::Object(args),
        missing_required: missing,
        confidence,
    }
}

/// Extracts a math expression by stripping non-math command phrases.
pub fn extract_math_expression(query: &str) -> String {
    let lower = query.to_lowercase();
    let prefixes = [
        "calculate",
        "what is",
        "what's",
        "compute",
        "solve",
        "evaluate",
        "calc",
        "please calculate",
    ];

    let mut expr = query.trim();
    for prefix in prefixes {
        if lower.starts_with(prefix) {
            expr = query[prefix.len()..].trim();
            break;
        }
    }

    expr.trim_end_matches('?').trim().to_string()
}

/// Extracts a filesystem path from quotes or regex.
pub fn extract_path(query: &str) -> Option<String> {
    if let Some(caps) = RE_QUOTED.captures(query) {
        let p = caps[1].trim();
        if p.contains('\\') || p.contains('/') || p.contains('.') {
            return Some(p.to_string());
        }
    }

    if let Some(mat) = RE_PATH_WIN.find(query) {
        return Some(mat.as_str().trim().to_string());
    }

    None
}

/// Extracts URL from query.
pub fn extract_url(query: &str) -> Option<String> {
    RE_URL.find(query).map(|m| m.as_str().to_string())
}

/// Extracts process ID (PID).
pub fn extract_pid(query: &str) -> Option<u32> {
    if let Some(caps) = RE_PID.captures(query) {
        return caps[1].parse::<u32>().ok();
    }
    None
}

/// Extracts target process name from termination commands.
pub fn extract_target_process_name(query: &str) -> Option<String> {
    let lower = query.to_lowercase();
    let prefixes = [
        "kill process",
        "kill",
        "terminate",
        "stop process",
        "end task",
    ];

    for p in prefixes {
        if let Some(pos) = lower.find(p) {
            let remainder = query[pos + p.len()..].trim();
            let name = remainder.split_whitespace().next().unwrap_or("");
            if !name.is_empty() && name.parse::<u32>().is_err() {
                return Some(name.trim_matches(|c| c == '"' || c == '\'').to_string());
            }
        }
    }
    None
}

/// Extracts percentage or integer value.
pub fn extract_percentage_or_number(query: &str) -> Option<u32> {
    if let Some(caps) = RE_PERCENTAGE.captures(query) {
        return caps[1].parse::<u32>().ok();
    }
    extract_number(query).map(|n| n as u32)
}

/// Extracts duration in seconds from human expressions.
pub fn extract_duration_seconds(query: &str) -> Option<u64> {
    if let Some(caps) = RE_DURATION.captures(query) {
        let num: u64 = caps[1].parse().ok()?;
        let unit = &caps[2];
        let mult = match unit {
            u if u.starts_with('m') => 60,
            u if u.starts_with('h') => 3600,
            _ => 1,
        };
        return Some(num * mult);
    }
    None
}

/// Extracts a label for a timer.
pub fn extract_timer_label(query: &str) -> String {
    if let Some(caps) = RE_QUOTED.captures(query) {
        return caps[1].to_string();
    }
    let lower = query.to_lowercase();
    if let Some(pos) = lower.find("for ") {
        let remainder = query[pos + 4..].trim();
        // Remove trailing duration
        let words: Vec<&str> = remainder.split_whitespace().collect();
        return words.join(" ");
    }
    "Timer".to_string()
}

/// Extracts application target name.
pub fn extract_app_target(query: &str) -> String {
    let lower = query.to_lowercase();
    let prefixes = ["open", "launch", "start", "run"];
    for p in prefixes {
        if lower.starts_with(p) {
            return query[p.len()..].trim().to_string();
        }
    }
    query.trim().to_string()
}

/// Extracts content payload (e.g. for write_file).
pub fn extract_content_payload(query: &str) -> Option<String> {
    if let Some(caps) = RE_QUOTED.captures(query) {
        return Some(caps[1].to_string());
    }
    None
}

/// Extracts first number found in string.
pub fn extract_number(query: &str) -> Option<i64> {
    let re = Regex::new(r#"-?\d+"#).unwrap();
    re.find(query).and_then(|m| m.as_str().parse::<i64>().ok())
}

/// Extracts quoted text or trailing words after trigger phrases.
pub fn extract_quoted_or_trailing(query: &str, triggers: &[&str]) -> String {
    if let Some(caps) = RE_QUOTED.captures(query) {
        return caps[1].to_string();
    }
    let lower = query.to_lowercase();
    for trigger in triggers {
        if let Some(pos) = lower.find(trigger) {
            let trailing = query[pos + trigger.len()..].trim();
            if !trailing.is_empty() {
                return trailing.to_string();
            }
        }
    }
    query.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_math() {
        assert_eq!(
            extract_math_expression("Calculate 2 + 2 * 10"),
            "2 + 2 * 10"
        );
        assert_eq!(extract_math_expression("What is sqrt(144)?"), "sqrt(144)");
    }

    #[test]
    fn test_extract_path() {
        let p = extract_path(r#"read file "C:\Users\test\document.txt""#).unwrap();
        assert_eq!(p, r#"C:\Users\test\document.txt"#);
    }

    #[test]
    fn test_extract_duration() {
        assert_eq!(
            extract_duration_seconds("Set a timer for 5 minutes"),
            Some(300)
        );
        assert_eq!(extract_duration_seconds("timer 30 sec"), Some(30));
        assert_eq!(extract_duration_seconds("remind me in 2 hours"), Some(7200));
    }

    #[test]
    fn test_extract_pid_and_process() {
        assert_eq!(extract_pid("kill process with pid 4582"), Some(4582));
        assert_eq!(
            extract_target_process_name("kill notepad.exe"),
            Some("notepad.exe".to_string())
        );
    }
}
