//! Deterministic argument resolution and schema extraction without LLMs.
//! Extracts typed arguments (paths, expressions, PIDs, durations, URLs, flags) directly from user queries.

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

static RE_PATH_WIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:[a-zA-Z]:\\[^"<>|?*\r\n\t]+)|(?:[a-zA-Z]:/[^"<>|?*\r\n\t]+)"#).unwrap()
});

static RE_PATH_POSIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:/[a-zA-Z0-9_\.\-]+)+"#).unwrap());

static RE_PATH_REL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:\.{1,2}[\\/][a-zA-Z0-9_\.\-\\/]+)"#).unwrap());

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
    if tool_name == "calculator" || tool_name == "math_eval" || tool_name.contains("math") {
        let expr = extract_math_expression(query_clean);
        args.insert("expression".to_string(), json!(expr));
    } else if tool_name.contains("filesystem")
        || tool_name == "read_file"
        || tool_name == "write_file"
        || tool_name == "list_directory"
        || tool_name == "file_read"
        || tool_name == "file_write"
        || tool_name == "file_delete"
        || tool_name == "fs_stat"
    {
        // Source and destination for copy/move operations
        if schema
            .get("properties")
            .and_then(|p| p.get("source"))
            .is_some()
            && schema
                .get("properties")
                .and_then(|p| p.get("destination"))
                .is_some()
        {
            let (src, dst) = extract_source_destination(query_clean);
            if let Some(s) = src {
                args.insert("source".to_string(), json!(s));
            }
            if let Some(d) = dst {
                args.insert("destination".to_string(), json!(d));
            }
        } else if let Some(path) = extract_path(query_clean) {
            args.insert("path".to_string(), json!(path));
        } else if query_clean.contains("current directory")
            || query_clean.contains("current folder")
            || query_clean.contains("here")
            || query_clean.contains(" .")
            || query_clean.ends_with('.')
        {
            args.insert("path".to_string(), json!("."));
        }

        if tool_name.contains("write") || tool_name.contains("append") {
            if let Some(content) = extract_content_payload(query_clean) {
                args.insert("content".to_string(), json!(content));
            }
        }

        if tool_name.contains("head") || tool_name.contains("tail") || tool_name.contains("read") {
            if let Some(lines) = extract_number(query_clean) {
                if lines > 0 {
                    args.insert("lines".to_string(), json!(lines));
                    args.insert("max_lines".to_string(), json!(lines));
                }
            }
        }
    } else if tool_name.contains("process")
        || tool_name == "kill_process"
        || tool_name == "process_terminate"
    {
        if let Some(pid) = extract_pid(query_clean) {
            args.insert("pid".to_string(), json!(pid));
        } else if let Some(proc_name) = extract_target_process_name(query_clean) {
            args.insert("name".to_string(), json!(proc_name));
        }
    } else if tool_name.contains("audio")
        || tool_name == "set_volume"
        || tool_name == "audio_volume"
    {
        if let Some(pct) = extract_percentage_or_number(query_clean) {
            args.insert("level".to_string(), json!(pct));
        }
    } else if tool_name.contains("timer") || tool_name == "set_timer" || tool_name == "timer_create"
    {
        if let Some(secs) = extract_duration_seconds(query_clean) {
            args.insert("seconds".to_string(), json!(secs));
        }
        let label = extract_timer_label(query_clean);
        args.insert("label".to_string(), json!(label));
    } else if tool_name.contains("network") || tool_name == "web_fetch" || tool_name == "http_get" {
        if let Some(url) = extract_url(query_clean) {
            args.insert("url".to_string(), json!(url));
        }
    } else if tool_name == "open_application" || tool_name == "app_launch" {
        let app = extract_app_target(query_clean);
        args.insert("target".to_string(), json!(app));
    } else if tool_name == "clipboard" || tool_name == "clipboard_set" {
        if query_clean.contains("paste")
            || query_clean.contains("read")
            || query_clean.contains("get")
            || query_clean.contains("from")
        {
            args.insert("action".to_string(), json!("get"));
        } else {
            args.insert("action".to_string(), json!("set"));
            let text = extract_quoted_or_trailing(query_clean, &["copy", "clipboard", "set"]);
            args.insert("text".to_string(), json!(text));
        }
    } else if tool_name == "network.ping" || tool_name.contains("ping") {
        if let Some(host) = extract_host_or_ip(query_clean) {
            args.insert("host".to_string(), json!(host));
        } else {
            args.insert("host".to_string(), json!("8.8.8.8"));
        }
    }

    // General schema-driven fallback / augmentation
    if let Some(props) = properties {
        for (prop_name, prop_def) in props {
            if args.contains_key(prop_name) {
                continue;
            }

            let prop_type = prop_def
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");

            match prop_type {
                "integer" | "number" => {
                    if prop_name.contains("pid") {
                        if let Some(pid) = extract_pid(query_clean) {
                            args.insert(prop_name.clone(), json!(pid));
                        }
                    } else if prop_name.contains("second") || prop_name.contains("duration") {
                        if let Some(secs) = extract_duration_seconds(query_clean) {
                            args.insert(prop_name.clone(), json!(secs));
                        }
                    } else if let Some(num) = extract_number(query_clean) {
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
                    if prop_name.contains("path")
                        || prop_name.contains("file")
                        || prop_name.contains("dir")
                    {
                        if let Some(path) = extract_path(query_clean) {
                            args.insert(prop_name.clone(), json!(path));
                        }
                    } else if prop_name.contains("url") {
                        if let Some(url) = extract_url(query_clean) {
                            args.insert(prop_name.clone(), json!(url));
                        }
                    } else if prop_name.contains("expression") {
                        let expr = extract_math_expression(query_clean);
                        args.insert(prop_name.clone(), json!(expr));
                    } else if prop_name.contains("name") {
                        if let Some(proc_name) = extract_target_process_name(query_clean) {
                            args.insert(prop_name.clone(), json!(proc_name));
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

/// Extracts a filesystem path from quotes, platform regex, or token analysis.
pub fn extract_path(query: &str) -> Option<String> {
    // 1. Quoted paths or file names
    if let Some(caps) = RE_QUOTED.captures(query) {
        let p = caps[1].trim();
        if p.contains('\\') || p.contains('/') || p.contains('.') {
            return Some(p.to_string());
        }
    }

    // 2. Windows absolute paths: C:\dir\file.txt or D:/dir/file.txt
    if let Some(mat) = RE_PATH_WIN.find(query) {
        return Some(mat.as_str().trim().to_string());
    }

    // 3. Relative paths starting with ./ or ../
    if let Some(mat) = RE_PATH_REL.find(query) {
        return Some(mat.as_str().trim().to_string());
    }

    // 4. POSIX absolute paths: /etc/hosts, /home/user/...
    if let Some(mat) = RE_PATH_POSIX.find(query) {
        let p = mat.as_str().trim();
        let start = mat.start();
        if (start == 0 || query.as_bytes()[start - 1] != b'.')
            && p.len() > 1
            && !p.starts_with("//")
        {
            return Some(p.to_string());
        }
    }

    // 5. Look for filename tokens after contextual keywords
    let lower = query.to_lowercase();
    let keywords = [
        "file ",
        "path ",
        "folder ",
        "read ",
        "cat ",
        "delete ",
        "remove ",
        "open ",
        "view ",
        "show ",
        "write to ",
        "append to ",
        "in ",
        "from ",
    ];

    for kw in keywords {
        if let Some(pos) = lower.find(kw) {
            let remainder = query[pos + kw.len()..].trim();
            let first_word = remainder.split_whitespace().next().unwrap_or("");
            let clean_word = first_word.trim_matches(|c: char| {
                c == '"' || c == '\'' || c == ',' || c == '?' || c == ';' || c == '(' || c == ')'
            });
            if (clean_word.contains('.') || clean_word.contains('/') || clean_word.contains('\\'))
                && !clean_word.starts_with("http://")
                && !clean_word.starts_with("https://")
                && clean_word.len() > 1
            {
                return Some(clean_word.to_string());
            }
        }
    }

    // 6. Any standalone token ending with a common file extension
    for word in query.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            c == '"' || c == '\'' || c == ',' || c == '?' || c == ';' || c == '(' || c == ')'
        });
        let lower_clean = clean.to_lowercase();
        let common_exts = [
            ".txt", ".json", ".rs", ".py", ".md", ".log", ".toml", ".csv", ".yaml", ".yml", ".sh",
            ".ps1", ".html", ".css", ".js", ".pdf",
        ];
        if common_exts.iter().any(|ext| lower_clean.ends_with(ext)) {
            return Some(clean.to_string());
        }
    }

    None
}

/// Extracts source and destination paths from copy/move commands.
pub fn extract_source_destination(query: &str) -> (Option<String>, Option<String>) {
    let lower = query.to_lowercase();
    for kw in &[" to ", " into "] {
        if let Some(pos) = lower.find(kw) {
            let left = &query[..pos];
            let right = &query[pos + kw.len()..];
            let src = extract_path(left);
            let dst = extract_path(right);
            if src.is_some() && dst.is_some() {
                return (src, dst);
            }
        }
    }
    (extract_path(query), None)
}

/// Extracts host or IP address from query.
pub fn extract_host_or_ip(query: &str) -> Option<String> {
    for word in query.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-');
        if clean.contains('.') && clean.len() >= 3 && !clean.ends_with('.') {
            return Some(clean.to_string());
        }
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
    let lower = query.to_lowercase();
    for kw in &["content:", "content is", "with content", "text:"] {
        if let Some(pos) = lower.find(kw) {
            let rem = query[pos + kw.len()..]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            if !rem.is_empty() {
                return Some(rem.to_string());
            }
        }
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
        let p_win = extract_path(r#"read file "C:\Users\test\document.txt""#).unwrap();
        assert_eq!(p_win, r#"C:\Users\test\document.txt"#);

        let p_posix = extract_path("cat /var/log/syslog").unwrap();
        assert_eq!(p_posix, "/var/log/syslog");

        let p_rel = extract_path("inspect ./src/main.rs").unwrap();
        assert_eq!(p_rel, "./src/main.rs");

        let p_ext = extract_path("view Cargo.toml please").unwrap();
        assert_eq!(p_ext, "Cargo.toml");
    }

    #[test]
    fn test_extract_source_destination() {
        let (src, dst) = extract_source_destination("copy file.txt to backup.txt");
        assert_eq!(src, Some("file.txt".to_string()));
        assert_eq!(dst, Some("backup.txt".to_string()));
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
