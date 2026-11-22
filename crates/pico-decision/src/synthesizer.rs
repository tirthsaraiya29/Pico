//! Deterministic natural language synthesis for tool execution results, direct queries, and clarifications.
//! Generates articulate, human-like voice responses locally without generative LLMs.

use crate::intent::UserIntent;
use serde_json::Value;

pub struct ResponseSynthesizer;

impl ResponseSynthesizer {
    /// Formats the final user-facing response from a tool execution result.
    pub fn format_tool_result(tool_name: &str, arguments: &Value, result: &Value) -> String {
        match tool_name {
            "calculator" | "math_eval" => {
                let expr = arguments
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(res) = result.get("result") {
                    format!("The result of {} is {}.", expr, res)
                } else if let Some(val) = result.as_str() {
                    format!("The result is {}.", val)
                } else {
                    format!("Calculated: {}", result)
                }
            }
            "system_info" | "sys_diagnostic" => {
                let os = result
                    .get("os")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Windows");
                let cpu = result
                    .get("cpu_usage_pct")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let mem = result
                    .get("memory_used_mb")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let mem_total = result
                    .get("memory_total_mb")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if mem_total > 0 {
                    format!(
                        "System is running {}. CPU utilization is {:.1}%, and memory usage is {} MB of {} MB.",
                        os, cpu, mem, mem_total
                    )
                } else {
                    format!("System report: {}", result)
                }
            }
            "list_processes" | "process_list" => {
                if let Some(arr) = result.as_array() {
                    let count = arr.len();
                    format!(
                        "Found {} active processes currently running on the system.",
                        count
                    )
                } else if let Some(count) = result.get("count").and_then(|v| v.as_u64()) {
                    format!("There are {} active processes running.", count)
                } else {
                    "Retrieved active process list.".to_string()
                }
            }
            "kill_process" | "process_terminate" => {
                let pid = arguments
                    .get("pid")
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                let name = arguments.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if !name.is_empty() {
                    format!("Successfully terminated process '{}'.", name)
                } else {
                    format!("Successfully terminated process with PID {}.", pid)
                }
            }
            "set_volume" | "audio_volume" => {
                let level = arguments
                    .get("level")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "desired".into());
                format!("Master audio volume set to {}%.", level)
            }
            "set_timer" | "timer_create" => {
                let secs = arguments
                    .get("seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let label = arguments
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Timer");
                let mins = secs / 60;
                let rem_secs = secs % 60;
                if mins > 0 && rem_secs > 0 {
                    format!(
                        "Set a timer for {} minutes and {} seconds for '{}'.",
                        mins, rem_secs, label
                    )
                } else if mins > 0 {
                    format!("Set a timer for {} minutes for '{}'.", mins, label)
                } else {
                    format!("Set a timer for {} seconds for '{}'.", secs, label)
                }
            }
            "file_read" => {
                if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
                    let snippet: String = content.chars().take(200).collect();
                    if content.len() > 200 {
                        format!("File contents: {}...", snippet)
                    } else {
                        format!("File contents: {}", snippet)
                    }
                } else {
                    "Read file successfully.".to_string()
                }
            }
            "file_write" => "File written successfully.".to_string(),
            "open_application" => {
                let target = arguments
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application");
                format!("Opened {}.", target)
            }
            "clipboard_set" => "Text copied to clipboard.".to_string(),
            "clipboard_get" => {
                if let Some(text) = result.get("text").and_then(|v| v.as_str()) {
                    format!("Clipboard contains: '{}'", text)
                } else {
                    "Retrieved clipboard content.".to_string()
                }
            }
            _ => {
                if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else if let Some(s) = result.as_str() {
                    s.to_string()
                } else {
                    format!("Action {} completed successfully.", tool_name)
                }
            }
        }
    }

    /// Formats a conversational direct response without calling any tool.
    pub fn format_direct_response(query: &str, intent: UserIntent) -> String {
        let lower = query.to_lowercase();
        if lower.contains("who are you")
            || lower.contains("what is your name")
            || lower.contains("what are you")
        {
            return "I am Pico, a real-time, locally operating probabilistic AI assistant running on your hardware.".to_string();
        }
        if lower.contains("hello") || lower.contains("hi") || lower.contains("hey") {
            return "Hello! How can I assist you with your system today?".to_string();
        }
        if lower.contains("thank") {
            return "You're welcome! Let me know if you need anything else.".to_string();
        }
        if lower.contains("goodbye") || lower.contains("bye") {
            return "Goodbye! Have a productive session.".to_string();
        }
        if lower.contains("how are you") {
            return "All local subsystems are operational and performing optimally. How can I help?".to_string();
        }
        if lower.contains("what can you do") || lower.contains("help") {
            return "I can monitor system diagnostics, manage processes, control audio, inspect files, evaluate mathematical expressions, manage timers, launch applications, and continuously learn from your feedback—all locally.".to_string();
        }

        match intent {
            UserIntent::DirectConversation => {
                "I understand. How else may I assist you?".to_string()
            }
            _ => {
                "I am ready to help. Please tell me which action or diagnostic you'd like to perform.".to_string()
            }
        }
    }

    /// Generates a clarification request when candidate actions are ambiguous.
    pub fn format_clarification_prompt(
        candidate_a: &str,
        score_a: f32,
        candidate_b: &str,
        score_b: f32,
    ) -> String {
        format!(
            "I found two likely actions: '{}' ({:.0}% confidence) and '{}' ({:.0}% confidence). Which one would you like me to execute?",
            candidate_a,
            score_a * 100.0,
            candidate_b,
            score_b * 100.0
        )
    }

    /// Generates a clarification prompt when required tool parameters are missing.
    pub fn format_missing_args_prompt(tool_name: &str, missing_args: &[String]) -> String {
        let args_str = missing_args.join(", ");
        format!(
            "To execute {}, please specify the following required information: {}.",
            tool_name, args_str
        )
    }
}
