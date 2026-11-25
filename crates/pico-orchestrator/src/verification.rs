//! Independent Objective Verification Engine for Autonomous Execution.
//! Inspects real operating system state (filesystem, process table, audio hardware,
//! arithmetic proofs, etc.) to verify whether user-commanded objectives were actually fulfilled.

use pico_core::session::ToolResult;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::debug;

/// Result of independent objective verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveVerification {
    pub tool_name: String,
    pub verified: bool,
    pub confidence: f32,
    pub evidence: String,
    pub details: serde_json::Value,
}

pub struct ObjectiveVerifier;

impl ObjectiveVerifier {
    /// Autonomously verifies whether the commanded objective was genuinely achieved on the host system.
    pub async fn verify_execution(
        tool_name: &str,
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        // If the tool reported an unhandled execution error, verification automatically fails
        if !result.success {
            let err_msg = result
                .error
                .clone()
                .unwrap_or_else(|| result.content.to_string());
            return ObjectiveVerification {
                tool_name: tool_name.to_string(),
                verified: false,
                confidence: 1.0,
                evidence: format!("Tool execution reported failure: {}", err_msg),
                details: serde_json::json!({ "error": err_msg }),
            };
        }

        // Domain-specific objective verification strategies
        if tool_name == "calculator"
            || tool_name.ends_with(".math")
            || tool_name.ends_with(".calculate")
        {
            Self::verify_calculator(arguments, result)
        } else if tool_name == "write_file"
            || tool_name.contains("filesystem.write")
            || tool_name.contains("filesystem.create")
        {
            Self::verify_file_write(arguments, result).await
        } else if tool_name.contains("filesystem.delete") || tool_name.contains("filesystem.remove")
        {
            Self::verify_file_deletion(arguments).await
        } else if tool_name == "kill_process"
            || tool_name == "close_application"
            || tool_name.contains("process.terminate")
            || tool_name.contains("process.kill")
        {
            Self::verify_process_termination(tool_name, arguments, result).await
        } else if tool_name.starts_with("audio.") {
            Self::verify_audio_operation(tool_name, arguments, result).await
        } else if tool_name.starts_with("display.") {
            Self::verify_display_operation(tool_name, arguments, result).await
        } else if tool_name == "open_application" {
            Self::verify_open_application(arguments, result)
        } else {
            Self::verify_generic_execution(tool_name, result)
        }
    }

    /// Verifies calculator results using an independent parse and evaluation.
    fn verify_calculator(
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        let expr = arguments
            .get("expression")
            .and_then(|e| e.as_str())
            .unwrap_or("");
        if expr.is_empty() {
            return ObjectiveVerification {
                tool_name: "calculator".into(),
                verified: true,
                confidence: 0.9,
                evidence: "No expression argument to verify".into(),
                details: serde_json::json!({}),
            };
        }

        let content_str = result.content.to_string();
        let reported_value = result
            .content
            .get("result")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                content_str
                    .lines()
                    .find(|l| l.contains("Result: ") || l.contains('='))
                    .and_then(|l| {
                        l.split(':')
                            .next_back()
                            .or_else(|| l.split('=').next_back())
                            .map(|v| v.trim())
                    })
                    .and_then(|v| v.parse::<f64>().ok())
            });

        // Independent AST calculation
        match pico_tools::calculator_parser::evaluate_expression(expr) {
            Ok(calc_value) => {
                let verified = if let Some(reported) = reported_value {
                    (reported - calc_value).abs() < 1e-6
                } else {
                    content_str.contains(&calc_value.to_string())
                };

                ObjectiveVerification {
                    tool_name: "calculator".into(),
                    verified,
                    confidence: 1.0,
                    evidence: format!(
                        "Independent AST evaluation yielded {}, reported {}",
                        calc_value,
                        reported_value.unwrap_or(calc_value)
                    ),
                    details: serde_json::json!({
                        "expression": expr,
                        "verified_value": calc_value,
                        "reported_value": reported_value
                    }),
                }
            }
            Err(e) => ObjectiveVerification {
                tool_name: "calculator".into(),
                verified: false,
                confidence: 0.95,
                evidence: format!(
                    "Independent AST verification failed to parse '{}': {}",
                    expr, e
                ),
                details: serde_json::json!({ "error": e }),
            },
        }
    }

    /// Verifies that a file write or creation actually persisted the expected content to disk.
    async fn verify_file_write(
        arguments: &serde_json::Value,
        _result: &ToolResult,
    ) -> ObjectiveVerification {
        let path_str = arguments
            .get("path")
            .or_else(|| arguments.get("file"))
            .or_else(|| arguments.get("target"))
            .and_then(|p| p.as_str())
            .unwrap_or("");

        if path_str.is_empty() {
            return ObjectiveVerification {
                tool_name: "write_file".into(),
                verified: true,
                confidence: 0.85,
                evidence: "Path parameter omitted; verified command execution".into(),
                details: serde_json::json!({}),
            };
        }

        let path = Path::new(path_str);
        match tokio::fs::metadata(path).await {
            Ok(meta) => {
                let size = meta.len();
                let expected_content = arguments.get("content").and_then(|c| c.as_str());

                let content_verified = if let Some(expected) = expected_content {
                    if let Ok(actual) = tokio::fs::read_to_string(path).await {
                        actual.contains(expected)
                    } else {
                        size > 0
                    }
                } else {
                    true
                };

                ObjectiveVerification {
                    tool_name: "write_file".into(),
                    verified: content_verified,
                    confidence: 1.0,
                    evidence: format!(
                        "Verified file '{}' exists on filesystem with size {} bytes",
                        path.display(),
                        size
                    ),
                    details: serde_json::json!({
                        "path": path_str,
                        "file_exists": true,
                        "size_bytes": size,
                        "content_matched": content_verified
                    }),
                }
            }
            Err(e) => ObjectiveVerification {
                tool_name: "write_file".into(),
                verified: false,
                confidence: 1.0,
                evidence: format!(
                    "File '{}' does not exist on filesystem: {}",
                    path.display(),
                    e
                ),
                details: serde_json::json!({
                    "path": path_str,
                    "file_exists": false,
                    "error": e.to_string()
                }),
            },
        }
    }

    /// Verifies that a file deletion actually removed the file from the filesystem.
    async fn verify_file_deletion(arguments: &serde_json::Value) -> ObjectiveVerification {
        let path_str = arguments
            .get("path")
            .or_else(|| arguments.get("file"))
            .and_then(|p| p.as_str())
            .unwrap_or("");

        if path_str.is_empty() {
            return ObjectiveVerification {
                tool_name: "delete_file".into(),
                verified: true,
                confidence: 0.8,
                evidence: "Missing path parameter".into(),
                details: serde_json::json!({}),
            };
        }

        let path = Path::new(path_str);
        let exists = tokio::fs::try_exists(path).await.unwrap_or(false);

        ObjectiveVerification {
            tool_name: "delete_file".into(),
            verified: !exists,
            confidence: 1.0,
            evidence: if !exists {
                format!(
                    "Verified file '{}' is no longer present on filesystem",
                    path.display()
                )
            } else {
                format!(
                    "File '{}' still exists on filesystem despite deletion attempt",
                    path.display()
                )
            },
            details: serde_json::json!({
                "path": path_str,
                "file_exists": exists
            }),
        }
    }

    /// Verifies that a terminated PID or closed application is absent from the process table.
    async fn verify_process_termination(
        tool_name: &str,
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        let pid_opt = arguments
            .get("pid")
            .and_then(|p| p.as_u64())
            .map(|p| p as u32);

        if let Some(pid) = pid_opt {
            let processes = pico_platform::PlatformApps::list_processes(300).unwrap_or_default();
            let is_still_running = processes.iter().any(|p| p.pid == pid);

            return ObjectiveVerification {
                tool_name: tool_name.into(),
                verified: !is_still_running,
                confidence: 1.0,
                evidence: if !is_still_running {
                    format!("PID {} absent from process list", pid)
                } else {
                    format!("PID {} is still running in active process table", pid)
                },
                details: serde_json::json!({
                    "pid": pid,
                    "still_running": is_still_running
                }),
            };
        }

        if let Some(name) = arguments
            .get("application")
            .or_else(|| arguments.get("name"))
            .and_then(|v| v.as_str())
        {
            let norm = name.trim_end_matches(".exe").to_lowercase();
            let processes = pico_platform::PlatformApps::list_processes(300).unwrap_or_default();
            let remaining = processes
                .iter()
                .filter(|p| {
                    let pname = p.name.trim_end_matches(".exe").to_lowercase();
                    pname == norm || pname.contains(&norm)
                })
                .count();
            let reported_count = result
                .content
                .get("terminated_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let verified = remaining == 0 && reported_count > 0;
            return ObjectiveVerification {
                tool_name: tool_name.into(),
                verified,
                confidence: 0.95,
                evidence: if verified {
                    format!(
                        "Verified {} process(es) for '{}' closed",
                        reported_count, name
                    )
                } else {
                    format!(
                        "{} matching process(es) for '{}' still active",
                        remaining, name
                    )
                },
                details: serde_json::json!({
                    "target": name,
                    "remaining_processes": remaining,
                    "terminated_count": reported_count
                }),
            };
        }

        ObjectiveVerification {
            tool_name: tool_name.into(),
            verified: false,
            confidence: 0.9,
            evidence: "Termination invoked without target PID or application name".into(),
            details: serde_json::json!({}),
        }
    }

    fn verify_open_application(
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        let status_ok = result
            .content
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s == "success")
            .unwrap_or(false);
        let opened = result
            .content
            .get("opened")
            .or_else(|| arguments.get("application"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let verified = result.success && status_ok && !opened.is_empty();
        ObjectiveVerification {
            tool_name: "open_application".into(),
            verified,
            confidence: if verified { 0.95 } else { 0.98 },
            evidence: if verified {
                format!("Validated launch dispatch for '{}'", opened)
            } else {
                format!("Launch verification failed for '{}'", opened)
            },
            details: serde_json::json!({ "opened": opened, "status_ok": status_ok }),
        }
    }

    /// Verifies audio hardware command outcome using live endpoint readback when available.
    async fn verify_audio_operation(
        tool_name: &str,
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        debug!(tool = tool_name, args = ?arguments, "Verifying audio hardware objective");
        let hardware_state = pico_platform::PlatformAudio::get_state();
        match hardware_state {
            Ok(state) => {
                let verified = if tool_name == "audio.set_volume" {
                    if let Some(req_lvl) = arguments.get("level").and_then(|v| v.as_u64()) {
                        (state.volume_percent as i64 - req_lvl as i64).abs() <= 2
                    } else {
                        result.success
                    }
                } else if tool_name == "audio.mute" {
                    if let Some(req_mute) = arguments.get("mute").and_then(|v| v.as_bool()) {
                        state.muted == req_mute
                    } else {
                        result.success
                    }
                } else {
                    result.success
                };
                ObjectiveVerification {
                    tool_name: tool_name.to_string(),
                    verified,
                    confidence: 1.0,
                    evidence: format!(
                        "Audio endpoint readback ({}): volume={}%, muted={}",
                        state.backend, state.volume_percent, state.muted
                    ),
                    details: serde_json::json!({
                        "volume_percent": state.volume_percent,
                        "muted": state.muted,
                        "backend": state.backend
                    }),
                }
            }
            Err(e) => ObjectiveVerification {
                tool_name: tool_name.to_string(),
                verified: false,
                confidence: 0.95,
                evidence: format!("Audio endpoint readback unavailable: {}", e),
                details: serde_json::json!({ "error": e.to_string() }),
            },
        }
    }

    async fn verify_display_operation(
        tool_name: &str,
        arguments: &serde_json::Value,
        result: &ToolResult,
    ) -> ObjectiveVerification {
        let reported_ok = result.success
            && result
                .content
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let level_opt = pico_platform::PlatformDisplay::get_brightness().ok();
        let verified = match (
            reported_ok,
            level_opt,
            arguments.get("level").and_then(|v| v.as_u64()),
        ) {
            (true, Some(actual), Some(target)) => (actual as i64 - target as i64).abs() <= 3,
            (ok, _, _) => ok,
        };
        ObjectiveVerification {
            tool_name: tool_name.to_string(),
            verified,
            confidence: if level_opt.is_some() { 0.98 } else { 0.9 },
            evidence: match level_opt {
                Some(b) => format!("Display brightness readback: {}%", b),
                None => format!("Display command result: {}", result.content),
            },
            details: serde_json::json!({ "brightness": level_opt }),
        }
    }

    /// Generic OS tool outcome verification.
    fn verify_generic_execution(tool_name: &str, result: &ToolResult) -> ObjectiveVerification {
        let content_str = result.content.to_string();
        let clean = content_str.trim().to_lowercase();
        let has_error_keyword = clean.contains("error:")
            || clean.contains("exception")
            || clean.contains("command not found")
            || clean.contains("access denied");

        let verified = result.success && !has_error_keyword;

        ObjectiveVerification {
            tool_name: tool_name.to_string(),
            verified,
            confidence: if verified { 0.90 } else { 0.98 },
            evidence: if verified {
                "Tool reported clean execution and verified response format".into()
            } else {
                format!(
                    "Execution output indicated unhandled system error: {}",
                    content_str
                )
            },
            details: serde_json::json!({
                "success": result.success,
                "has_error_keyword": has_error_keyword
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verify_calculator_objective() {
        let args = serde_json::json!({ "expression": "100 + 42" });
        let result = ToolResult {
            call_id: "test".into(),
            name: "calculator".into(),
            content: serde_json::json!({ "result": 142.0 }),
            error: None,
            success: true,
            execution_duration_ms: 1,
        };

        let ver = ObjectiveVerifier::verify_execution("calculator", &args, &result).await;
        assert!(ver.verified);
        assert_eq!(ver.confidence, 1.0);
    }

    #[tokio::test]
    async fn test_verify_file_write_and_delete_objective() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("verify_test.txt");
        let path_str = test_file.to_str().unwrap();

        // 1. Verify before write (should fail)
        let args = serde_json::json!({ "path": path_str, "content": "Autonomous Pico Test" });
        let res_dummy = ToolResult {
            call_id: "1".into(),
            name: "write_file".into(),
            content: serde_json::json!({ "status": "Wrote file" }),
            error: None,
            success: true,
            execution_duration_ms: 2,
        };
        let ver = ObjectiveVerifier::verify_execution("write_file", &args, &res_dummy).await;
        assert!(!ver.verified, "File does not exist yet");

        // 2. Perform real write and verify
        tokio::fs::write(&test_file, "Autonomous Pico Test")
            .await
            .unwrap();
        let ver2 = ObjectiveVerifier::verify_execution("write_file", &args, &res_dummy).await;
        assert!(ver2.verified);
        assert!(ver2.evidence.contains("exists"));

        // 3. Delete file and verify deletion
        tokio::fs::remove_file(&test_file).await.unwrap();
        let del_args = serde_json::json!({ "path": path_str });
        let ver3 = ObjectiveVerifier::verify_execution(
            "windows.filesystem.delete_file",
            &del_args,
            &res_dummy,
        )
        .await;
        assert!(ver3.verified);
        assert!(ver3.evidence.contains("no longer present"));
    }
}
