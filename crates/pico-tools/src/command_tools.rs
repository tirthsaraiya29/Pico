use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_security::InputSanitizer;
use serde_json::json;
use std::path::Path;

const MAX_COMMAND_OUTPUT_BYTES: usize = 256 * 1024;

fn is_shell_interpreter_wrapper(cmd: &str) -> bool {
    let base = Path::new(cmd)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd)
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "cmd" | "powershell" | "pwsh" | "sh" | "bash" | "zsh" | "fish" | "wsl"
    )
}

fn bounded_lossy_utf8(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_COMMAND_OUTPUT_BYTES {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        let slice = &bytes[..MAX_COMMAND_OUTPUT_BYTES];
        format!(
            "{}\n...[output truncated at {} bytes]",
            String::from_utf8_lossy(slice),
            MAX_COMMAND_OUTPUT_BYTES
        )
    }
}

/// Tool to execute approved non-shell executables directly without spawning arbitrary shell interpreters.
pub struct ExecuteShellCommandTool;

#[async_trait]
impl Tool for ExecuteShellCommandTool {
    fn name(&self) -> &str {
        "execute_shell_command"
    }

    fn description(&self) -> &str {
        "Executes a validated executable directly (no raw shell wrappers). Strictly requires user confirmation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command executable to invoke (e.g. 'git', 'cargo', 'ping')"
                },
                "arguments": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of command arguments"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let command = arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'command' argument".into(),
            })?;

        if !InputSanitizer::is_safe_shell_arg(command) || is_shell_interpreter_wrapper(command) {
            return Err(PicoError::Security(format!(
                "Command '{}' is rejected (dangerous characters or bare shell interpreter wrapper)",
                command
            )));
        }

        let mut args_list = Vec::new();
        if let Some(arr) = arguments.get("arguments").and_then(|v| v.as_array()) {
            for arg_val in arr {
                let Some(s) = arg_val.as_str() else {
                    return Err(PicoError::Security(
                        "Command arguments must be strings".into(),
                    ));
                };
                if !InputSanitizer::is_safe_shell_arg(s) {
                    return Err(PicoError::Security(format!(
                        "Command argument '{}' contains dangerous shell metacharacters",
                        s
                    )));
                }
                args_list.push(s.to_string());
            }
        }

        let mut cmd = tokio::process::Command::new(command);
        cmd.kill_on_drop(true);
        cmd.args(&args_list);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd.spawn().map_err(|e| PicoError::ToolExecution {
            tool: self.name().into(),
            message: format!("Failed to spawn command '{}': {}", command, e),
        })?;

        let child_id = child.id();
        let timeout = std::time::Duration::from_secs(30);

        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(res) => res.map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed to execute command '{}': {}", command, e),
            })?,
            Err(_) => {
                #[cfg(target_os = "windows")]
                if let Some(pid) = child_id {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &pid.to_string()])
                        .output();
                }
                #[cfg(not(target_os = "windows"))]
                if let Some(pid) = child_id {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .output();
                }
                return Err(PicoError::ToolExecution {
                    tool: self.name().into(),
                    message: format!("Command '{}' timed out after {:?}", command, timeout),
                });
            }
        };

        let stdout = bounded_lossy_utf8(&output.stdout);
        let stderr = bounded_lossy_utf8(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            return Err(PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!(
                    "Command '{}' failed with exit code {}: {}",
                    command,
                    exit_code,
                    if !stderr.is_empty() { &stderr } else { &stdout }
                ),
            });
        }

        Ok(json!({
            "command": command,
            "arguments": args_list,
            "exit_code": exit_code,
            "stdout": stdout.trim(),
            "stderr": stderr.trim(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shell_wrappers_blocked() {
        let tool = ExecuteShellCommandTool;
        for wrapper in ["powershell", "cmd.exe", "bash", "/bin/sh", "pwsh"] {
            assert!(
                tool.execute(json!({ "command": wrapper, "arguments": ["-c", "echo hi"] }))
                    .await
                    .is_err(),
                "Wrapper '{}' must be blocked",
                wrapper
            );
        }
    }
}
