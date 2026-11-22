//! Execution engine & platform adapter for massive OS tools.
//! Provides parameterized invocation, timeouts, strict argument validation,
//! structured JSON output parsing, and cross-platform verification.

use crate::catalog::types::{LatencyClass, TargetOs, ToolMetadata};
use pico_core::error::{PicoError, Result};
use pico_security::PathSandbox;
use std::time::Duration;

pub async fn execute_tool_adapter(
    meta: &ToolMetadata,
    arguments: serde_json::Value,
    sandbox: Option<&PathSandbox>,
) -> Result<serde_json::Value> {
    // 1. Validate required parameters
    if let Some(req_array) = meta
        .parameters_schema
        .get("required")
        .and_then(|r| r.as_array())
    {
        for req in req_array {
            if let Some(key) = req.as_str() {
                if !arguments.is_object() || arguments.get(key).is_none() {
                    return Err(PicoError::ToolExecution {
                        tool: meta.canonical_id.clone(),
                        message: format!("Missing required parameter '{}'", key),
                    });
                }
            }
        }
    }

    // 2. Enforce PathSandbox validation on all path parameters and sanitize arguments
    let default_sandbox = PathSandbox::new(vec![std::path::PathBuf::from(".")], vec![]);
    let effective_sandbox = sandbox.unwrap_or(&default_sandbox);

    let mut validated_arguments = arguments.clone();
    if let Some(obj) = validated_arguments.as_object_mut() {
        for (k, v) in obj.iter_mut() {
            let k_lower = k.to_ascii_lowercase();
            let is_path_param = k_lower.contains("path")
                || k_lower.contains("file")
                || k_lower.contains("dir")
                || k_lower == "source"
                || k_lower == "destination"
                || k_lower == "target";

            if is_path_param {
                if let Some(s) = v.as_str() {
                    let safe_path = effective_sandbox.validate_path(s).map_err(|e| {
                        PicoError::Security(format!(
                            "Sandbox violation in parameter '{}' for tool {}: {}",
                            k, meta.canonical_id, e
                        ))
                    })?;
                    *v = serde_json::Value::String(safe_path.to_string_lossy().to_string());
                }
            } else if let Some(s) = v.as_str() {
                // Ensure shell argument does not contain arbitrary shell metacharacters
                if !pico_security::InputSanitizer::is_safe_shell_arg(s) {
                    return Err(PicoError::Security(format!(
                        "Prohibited shell metacharacters detected in argument '{}': {}",
                        k, s
                    )));
                }
            }
        }
    }

    // Determine execution timeout based on LatencyClass
    let timeout = match meta.latency_class {
        LatencyClass::SubMillisecond => Duration::from_millis(1000),
        LatencyClass::FastSync => Duration::from_millis(5000),
        LatencyClass::AsyncLongRunning => Duration::from_millis(30000),
    };

    let current_os = std::env::consts::OS;

    match meta.target_os {
        TargetOs::Windows => {
            #[cfg(target_os = "windows")]
            {
                execute_windows_command(meta, &validated_arguments, timeout).await
            }
            #[cfg(not(target_os = "windows"))]
            {
                Ok(serde_json::json!({
                    "status": "os_contract_verified",
                    "target_os": "windows",
                    "host_os": current_os,
                    "canonical_id": meta.canonical_id,
                    "parameters": validated_arguments,
                    "simulated": true,
                    "message": format!("Tool {} validated against Windows execution contract on {}", meta.canonical_id, current_os)
                }))
            }
        }
        TargetOs::Linux => {
            #[cfg(target_os = "linux")]
            {
                execute_linux_command(meta, &validated_arguments, timeout).await
            }
            #[cfg(not(target_os = "linux"))]
            {
                Ok(serde_json::json!({
                    "status": "os_contract_verified",
                    "target_os": "linux",
                    "host_os": current_os,
                    "canonical_id": meta.canonical_id,
                    "parameters": validated_arguments,
                    "simulated": true,
                    "message": format!("Tool {} validated against Linux execution contract on {}", meta.canonical_id, current_os)
                }))
            }
        }
        TargetOs::Universal => Ok(serde_json::json!({
            "status": "success",
            "canonical_id": meta.canonical_id,
            "parameters": validated_arguments
        })),
    }
}

#[cfg(target_os = "windows")]
async fn execute_windows_command(
    meta: &ToolMetadata,
    arguments: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let script = if meta.adapter_cmd.ends_with(".exe") {
        format!("{} {}", meta.adapter_cmd, meta.adapter_args)
    } else {
        format!(
            "{} {} | ConvertTo-Json -Compress -Depth 2",
            meta.adapter_cmd, meta.adapter_args
        )
    };

    let mut resolved_script = script;
    if let Some(obj) = arguments.as_object() {
        for (k, v) in obj {
            let val_str = match v {
                serde_json::Value::String(s) => s.replace('\'', "''").replace('`', "``"),
                other => other.to_string(),
            };
            resolved_script = resolved_script
                .replace(&format!("${{{}}}", k), &format!("'{}'", val_str))
                .replace(&format!("${}", k), &format!("'{}'", val_str));
        }
    }

    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &resolved_script,
    ]);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(res) => res.map_err(|e| PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!("Failed to execute process: {}", e),
        })?,
        Err(_) => {
            return Err(PicoError::ToolExecution {
                tool: meta.canonical_id.clone(),
                message: format!("Execution timed out after {:?}", timeout),
            })
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!(
                "Command exited with code {:?}: {}",
                output.status.code(),
                stderr
            ),
        });
    }

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(parsed)
    } else {
        Ok(serde_json::json!({
            "status": "success",
            "output": stdout,
            "stderr": stderr
        }))
    }
}

#[cfg(target_os = "linux")]
async fn execute_linux_command(
    meta: &ToolMetadata,
    arguments: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    if meta.adapter_cmd.starts_with("/proc") || meta.adapter_cmd.starts_with("/sys") {
        let mut path = meta.adapter_cmd.clone();
        if let Some(obj) = arguments.as_object() {
            for (k, v) in obj {
                let val_str = v.as_str().unwrap_or("");
                path = path
                    .replace(&format!("${{{}}}", k), val_str)
                    .replace(&format!("${}", k), val_str);
            }
        }
        let content =
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| PicoError::ToolExecution {
                    tool: meta.canonical_id.clone(),
                    message: format!("Failed to read {}: {}", path, e),
                })?;
        return Ok(serde_json::json!({
            "status": "success",
            "path": path,
            "content": content.trim()
        }));
    }

    let (binary, extra_cmd_args) = if let Some((cmd, rest)) = meta.adapter_cmd.split_once(' ') {
        (
            cmd,
            rest.split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>(),
        )
    } else {
        (meta.adapter_cmd.as_str(), Vec::new())
    };

    let mut cmd = tokio::process::Command::new(binary);
    let mut args: Vec<String> = extra_cmd_args;
    args.extend(meta.adapter_args.split_whitespace().map(|s| s.to_string()));

    if let Some(obj) = arguments.as_object() {
        for arg in &mut args {
            for (k, v) in obj {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                *arg = arg
                    .replace(&format!("${{{}}}", k), &val_str)
                    .replace(&format!("${}", k), &val_str);
            }
        }
    }

    // Drop optional placeholders that were not provided in arguments
    args.retain(|arg| !arg.contains('$'));

    cmd.args(&args);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(res) => res.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PicoError::ToolExecution {
                    tool: meta.canonical_id.clone(),
                    message: format!(
                        "Tool dependency '{}' is not installed or not in PATH on this system",
                        binary
                    ),
                }
            } else {
                PicoError::ToolExecution {
                    tool: meta.canonical_id.clone(),
                    message: format!("Failed to execute process '{}': {}", binary, e),
                }
            }
        })?,
        Err(_) => {
            return Err(PicoError::ToolExecution {
                tool: meta.canonical_id.clone(),
                message: format!("Execution timed out after {:?}", timeout),
            })
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!(
                "Command '{}' exited with code {:?}: {}",
                binary,
                output.status.code(),
                stderr
            ),
        });
    }

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(parsed)
    } else {
        Ok(serde_json::json!({
            "status": "success",
            "output": stdout,
            "stderr": stderr
        }))
    }
}
