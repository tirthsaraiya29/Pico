use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_security::InputSanitizer;
use serde_json::json;
use std::process::Command;

/// Tool to execute approved system shell commands.
pub struct ExecuteShellCommandTool;

#[async_trait]
impl Tool for ExecuteShellCommandTool {
    fn name(&self) -> &str {
        "execute_shell_command"
    }

    fn description(&self) -> &str {
        "Executes a validated shell command on the host operating system. Strictly requires user confirmation."
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

        // Sanitize command executable
        if !InputSanitizer::is_safe_shell_arg(command) {
            return Err(PicoError::Security(format!(
                "Command name '{}' contains invalid or dangerous characters",
                command
            )));
        }

        let mut args_list = Vec::new();
        if let Some(arr) = arguments.get("arguments").and_then(|v| v.as_array()) {
            for arg_val in arr {
                if let Some(s) = arg_val.as_str() {
                    if !InputSanitizer::is_safe_shell_arg(s) {
                        return Err(PicoError::Security(format!(
                            "Command argument '{}' contains dangerous shell metacharacters",
                            s
                        )));
                    }
                    args_list.push(s.to_string());
                }
            }
        }

        let output = Command::new(command)
            .args(&args_list)
            .output()
            .map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed to execute command '{}': {}", command, e),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(json!({
            "command": command,
            "args": args_list,
            "exit_code": exit_code,
            "success": output.status.success(),
            "stdout": stdout,
            "stderr": stderr
        }))
    }
}
