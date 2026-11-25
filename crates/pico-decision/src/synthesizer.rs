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
            "get_system_info" | "system_info" | "sys_diagnostic" => {
                let os = result
                    .get("os")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown OS");
                let cores = result
                    .get("cpu_cores")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let avail_mb = result
                    .get("available_memory_mb")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total_mb = result
                    .get("total_memory_mb")
                    .or_else(|| result.get("memory_total_mb"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let battery = result.get("battery_percent").and_then(|v| v.as_u64());

                if total_mb > 0 {
                    let bat_str = match battery {
                        Some(b) => format!(" Battery is at {}%.", b),
                        None => String::new(),
                    };
                    format!(
                        "System is running {} ({} CPU cores), with {} MB free of {} MB RAM.{}",
                        os, cores, avail_mb, total_mb, bat_str
                    )
                } else {
                    format!("System report: {}", result)
                }
            }
            "list_processes" | "process_list" => {
                if let Some(arr) = result.get("processes").and_then(|v| v.as_array()) {
                    let top: Vec<String> = arr
                        .iter()
                        .take(5)
                        .filter_map(|p| {
                            Some(format!(
                                "{} ({} MB)",
                                p.get("name")?.as_str()?,
                                p.get("memory_mb").and_then(|m| m.as_u64()).unwrap_or(0)
                            ))
                        })
                        .collect();
                    if top.is_empty() {
                        format!("Found {} active processes.", arr.len())
                    } else {
                        format!(
                            "Listed {} processes. Top items: {}.",
                            arr.len(),
                            top.join(", ")
                        )
                    }
                } else if let Some(arr) = result.as_array() {
                    format!("Found {} active processes.", arr.len())
                } else {
                    "Retrieved active process list.".to_string()
                }
            }
            "kill_process" | "process_terminate" => {
                let pid = arguments
                    .get("pid")
                    .or_else(|| result.get("terminated_pid"))
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                let name = arguments.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if !name.is_empty() {
                    format!("Terminated process '{}'.", name)
                } else {
                    format!("Terminated process with PID {}.", pid)
                }
            }
            "close_application" => {
                if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else {
                    let target = arguments
                        .get("application")
                        .and_then(|v| v.as_str())
                        .unwrap_or("application");
                    format!("Closed {}.", target)
                }
            }
            "audio.set_volume" | "set_volume" | "audio_volume" => {
                let level = result
                    .get("level")
                    .or_else(|| arguments.get("level"))
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "requested".into());
                format!("Master audio volume set to {}%.", level)
            }
            "audio.get_volume" => {
                let level = result
                    .get("level")
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let muted = result
                    .get("muted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                format!(
                    "Master volume is {}% ({}).",
                    level,
                    if muted { "muted" } else { "unmuted" }
                )
            }
            "audio.mute" => {
                if let Some(muted) = result.get("muted").and_then(|v| v.as_bool()) {
                    if muted {
                        "Audio output muted.".to_string()
                    } else {
                        "Audio output unmuted.".to_string()
                    }
                } else if let Some(msg) = result.get("message").and_then(|v| v.as_str()) {
                    msg.to_string()
                } else {
                    "Updated audio mute state.".to_string()
                }
            }
            "display.set_brightness" | "display.get_brightness" => {
                if let Some(level) = result.get("level").and_then(|v| v.as_u64()) {
                    format!("Display brightness is {}%.", level)
                } else if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else {
                    "Display brightness updated.".to_string()
                }
            }
            "manage_timer" | "set_timer" | "timer_create" => {
                if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    return msg.to_string();
                }
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
            "read_file" | "file_read" => {
                if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
                    let normalized_snippet: String = content
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(200)
                        .collect();
                    format!("File contents: {}", normalized_snippet)
                } else {
                    "Read file successfully.".to_string()
                }
            }
            "write_file" | "file_write" => "File written successfully.".to_string(),
            "search_files" => {
                let count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                format!("Found {} matching files.", count)
            }
            "open_application" => {
                let target = arguments
                    .get("application")
                    .or_else(|| arguments.get("target"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("application");
                format!("Opened {}.", target)
            }
            "clipboard" | "clipboard_set" | "clipboard_get" => {
                if let Some(text) = result.get("text").and_then(|v| v.as_str()) {
                    format!("Clipboard contains: '{}'", text)
                } else if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else {
                    "Clipboard updated.".to_string()
                }
            }
            _ => {
                if let Some(err) = result.get("error").and_then(|e| e.as_str()) {
                    format!("Action {} failed: {}", tool_name, err)
                } else if let Some(msg) = result.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else if let Some(s) = result.as_str() {
                    s.to_string()
                } else {
                    format!("Action {} completed.", tool_name)
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
            return "All local subsystems are ready. How can I help?".to_string();
        }
        if lower.contains("what can you do") || lower.contains("help") {
            return "I can check system specs, list or stop processes, manage volume and display brightness, search or read files, calculate math expressions, manage timers, and open or close applications, all locally.".to_string();
        }

        if lower.contains("not sure") || lower.contains("don't know") {
            return "No problem! Let me know whenever you would like to run a tool, check system specs, calculate math, or launch an app.".to_string();
        }
        if lower == "cancel" || lower == "stop" {
            return "Operation cancelled.".to_string();
        }
        if lower == "first" || lower == "second" || lower == "neither" || lower == "none" {
            return "Understood. Please let me know what command you would like me to execute."
                .to_string();
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

    /// Formats an explicit, honest failure report when a tool or its verification fails.
    pub fn format_failure(tool_name: &str, reason: &str) -> String {
        format!("Action '{}' did not succeed: {}", tool_name, reason)
    }

    /// Generates a numbered clarification request when candidate actions are ambiguous.
    pub fn format_clarification_prompt(
        candidate_a: &str,
        score_a: f32,
        candidate_b: &str,
        score_b: f32,
    ) -> String {
        format!(
            "I found two possible actions:\n1. {} ({:.0}% confidence)\n2. {} ({:.0}% confidence)\nReply with 'first', 'second', 'option 1', 'option 2', the action name, or 'cancel'.",
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
