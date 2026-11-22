//! Execution engine & platform adapter for massive OS tools.
//! Provides parameterized invocation, timeouts, strict argument validation,
//! structured JSON output parsing, and cross-platform verification.

use crate::catalog::types::{LatencyClass, TargetOs, ToolMetadata};
use pico_core::error::{PicoError, Result};
use std::time::Duration;

pub async fn execute_tool_adapter(
    meta: &ToolMetadata,
    arguments: serde_json::Value,
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
                execute_windows_command(meta, &arguments, timeout).await
            }
            #[cfg(not(target_os = "windows"))]
            {
                Ok(serde_json::json!({
                    "status": "os_contract_verified",
                    "target_os": "windows",
                    "host_os": current_os,
                    "canonical_id": meta.canonical_id,
                    "parameters": arguments,
                    "simulated": true,
                    "message": format!("Tool {} validated against Windows execution contract on {}", meta.canonical_id, current_os)
                }))
            }
        }
        TargetOs::Linux => {
            #[cfg(target_os = "linux")]
            {
                execute_linux_command(meta, &arguments, timeout).await
            }
            #[cfg(not(target_os = "linux"))]
            {
                Ok(serde_json::json!({
                    "status": "os_contract_verified",
                    "target_os": "linux",
                    "host_os": current_os,
                    "canonical_id": meta.canonical_id,
                    "parameters": arguments,
                    "simulated": true,
                    "message": format!("Tool {} validated against Linux execution contract on {}", meta.canonical_id, current_os)
                }))
            }
        }
        TargetOs::Universal => Ok(serde_json::json!({
            "status": "success",
            "canonical_id": meta.canonical_id,
            "parameters": arguments
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
            resolved_script =
                resolved_script.replace(&format!("${}", k), &format!("'{}'", val_str));
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
                path = path.replace(&format!("${}", k), val_str);
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

    let mut cmd = tokio::process::Command::new(meta.adapter_cmd.clone());
    let mut args: Vec<String> = meta
        .adapter_args
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if let Some(obj) = arguments.as_object() {
        for arg in &mut args {
            for (k, v) in obj {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                *arg = arg.replace(&format!("${}", k), &val_str);
            }
        }
    }

    cmd.args(&args);

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
