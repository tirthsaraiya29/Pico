//! Execution engine & platform adapter for OS catalog tools.
//! Passes parameters via environment variables on Windows (zero PowerShell syntax interpolation),
//! validates dynamic `/proc`/`/sys` paths strictly on Linux, rejects cross-OS mismatch cleanly,
//! enforces PathSandbox on path parameters, and caps output sizes cleanly.

use crate::catalog::types::{LatencyClass, TargetOs, ToolMetadata};
use pico_core::error::{PicoError, Result};
use pico_security::PathSandbox;
use std::time::Duration;

const MAX_OUTPUT_BYTES: usize = 512 * 1024;

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

    // 2. Enforce PathSandbox validation on all path parameters and sanitize non-path strings
    let default_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let default_sandbox = PathSandbox::new(vec![default_root], vec![]);
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
                if !pico_security::InputSanitizer::is_safe_shell_arg(s) {
                    return Err(PicoError::Security(format!(
                        "Prohibited shell metacharacters detected in argument '{}': {}",
                        k, s
                    )));
                }
            }
        }
    }

    let timeout = match meta.latency_class {
        LatencyClass::SubMillisecond => Duration::from_millis(2000),
        LatencyClass::FastSync => Duration::from_millis(8000),
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
                Err(PicoError::Platform(format!(
                    "Tool '{}' is a Windows-specific tool and cannot run on {}",
                    meta.canonical_id, current_os
                )))
            }
        }
        TargetOs::Linux => {
            #[cfg(target_os = "linux")]
            {
                execute_linux_command(meta, &validated_arguments, timeout).await
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(PicoError::Platform(format!(
                    "Tool '{}' is a Linux-specific tool and cannot run on {}",
                    meta.canonical_id, current_os
                )))
            }
        }
        TargetOs::Universal => Ok(serde_json::json!({
            "status": "success",
            "canonical_id": meta.canonical_id,
            "parameters": validated_arguments
        })),
    }
}

fn truncate_utf8(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        String::from_utf8_lossy(bytes).trim().to_string()
    } else {
        let mut s = String::from_utf8_lossy(&bytes[..limit]).trim().to_string();
        s.push_str("\n...[output truncated]");
        s
    }
}

#[cfg(target_os = "windows")]
async fn execute_windows_command(
    meta: &ToolMetadata,
    arguments: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let mut env_pairs: Vec<(String, String)> = Vec::new();
    let mut resolved_args = meta.adapter_args.clone();

    if let Some(obj) = arguments.as_object() {
        for (k, v) in obj {
            let val_str = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            let env_key = format!("PICO_ARG_{}", k.to_ascii_uppercase());
            env_pairs.push((env_key.clone(), val_str));
            resolved_args = resolved_args
                .replace(&format!("${{{}}}", k), &format!("$env:{}", env_key))
                .replace(&format!("${}", k), &format!("$env:{}", env_key));
        }
    }

    let mut cmd = if meta.adapter_cmd.ends_with(".exe") && !resolved_args.contains("$env:") {
        let mut c = tokio::process::Command::new(&meta.adapter_cmd);
        c.args(resolved_args.split_whitespace());
        c
    } else {
        let script = if meta.adapter_cmd.ends_with(".exe") {
            let mut argv: Vec<String> = resolved_args.split_whitespace().map(String::from).collect();
            for part in &mut argv {
                if let Some(env_name) = part.strip_prefix("$env:") {
                    if let Some((_, val)) = env_pairs.iter().find(|(ek, _)| ek == env_name) {
                        *part = val.clone();
                    }
                }
            }
            let mut c = tokio::process::Command::new(&meta.adapter_cmd);
            c.args(&argv);
            c
        } else {
            let ps_script = format!(
                "& {{ {} {} }} | ConvertTo-Json -Compress -Depth 3",
                meta.adapter_cmd, resolved_args
            );
            let mut c = tokio::process::Command::new("powershell.exe");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &ps_script,
            ]);
            c
        };
        script
    };

    cmd.kill_on_drop(true);
    for (ek, ev) in &env_pairs {
        cmd.env(ek, ev);
    }

    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!("Failed to spawn process: {}", e),
        })?;

    let child_id = child.id();

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(res) => res.map_err(|e| PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!("Failed to wait for process: {}", e),
        })?,
        Err(_) => {
            if let Some(pid) = child_id {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .output();
            }
            return Err(PicoError::ToolExecution {
                tool: meta.canonical_id.clone(),
                message: format!("Execution timed out after {:?}", timeout),
            });
        }
    };

    let stdout = truncate_utf8(&output.stdout, MAX_OUTPUT_BYTES);
    let stderr = truncate_utf8(&output.stderr, MAX_OUTPUT_BYTES);

    let exit_code = output.status.code().unwrap_or(-1);
    if !output.status.success() {
        return Err(PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!(
                "Command exited with non-zero exit code {}: {}",
                exit_code,
                if !stderr.is_empty() { &stderr } else { &stdout }
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
fn validate_proc_or_sys_token(key: &str, value: &str) -> Result<()> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(PicoError::Security(format!(
            "Unsafe path token '{}' in parameter '{}'",
            value, key
        )));
    }
    if key.to_ascii_lowercase().contains("pid") {
        if !value.chars().all(|c| c.is_ascii_digit()) {
            return Err(PicoError::Security(format!(
                "PID parameter '{}' must contain digits only",
                key
            )));
        }
    } else if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(PicoError::Security(format!(
            "Parameter '{}' contains prohibited sysfs/procfs characters",
            key
        )));
    }
    Ok(())
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
                validate_proc_or_sys_token(k, val_str)?;
                path = path
                    .replace(&format!("${{{}}}", k), val_str)
                    .replace(&format!("${}", k), val_str);
            }
        }
        let canonical = tokio::fs::canonicalize(&path).await.map_err(|e| {
            PicoError::ToolExecution {
                tool: meta.canonical_id.clone(),
                message: format!("Cannot resolve {}: {}", path, e),
            }
        })?;
        let canonical_str = canonical.to_string_lossy();
        if !canonical_str.starts_with("/proc/") && !canonical_str.starts_with("/sys/") {
            return Err(PicoError::Security(format!(
                "Canonical path '{}' escaped /proc or /sys containment",
                canonical_str
            )));
        }
        let content =
            tokio::fs::read(&canonical)
                .await
                .map_err(|e| PicoError::ToolExecution {
                    tool: meta.canonical_id.clone(),
                    message: format!("Failed to read {}: {}", canonical.display(), e),
                })?;
        return Ok(serde_json::json!({
            "status": "success",
            "path": canonical.display().to_string(),
            "content": truncate_utf8(&content, MAX_OUTPUT_BYTES)
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
    cmd.kill_on_drop(true);
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

    args.retain(|arg| !arg.contains('$'));
    cmd.args(&args);

    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
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
                    message: format!("Failed to spawn process '{}': {}", binary, e),
                }
            }
        })?;

    let child_id = child.id();

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(res) => res.map_err(|e| PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!("Failed to wait for process '{}': {}", binary, e),
        })?,
        Err(_) => {
            if let Some(pid) = child_id {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
            }
            return Err(PicoError::ToolExecution {
                tool: meta.canonical_id.clone(),
                message: format!("Execution timed out after {:?}", timeout),
            });
        }
    };

    let stdout = truncate_utf8(&output.stdout, MAX_OUTPUT_BYTES);
    let stderr = truncate_utf8(&output.stderr, MAX_OUTPUT_BYTES);

    let exit_code = output.status.code().unwrap_or(-1);
    if !output.status.success() {
        return Err(PicoError::ToolExecution {
            tool: meta.canonical_id.clone(),
            message: format!(
                "Command '{}' exited with non-zero exit code {}: {}",
                binary,
                exit_code,
                if !stderr.is_empty() { &stderr } else { &stdout }
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
