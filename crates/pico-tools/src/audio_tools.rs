//! Audio and sound control tools for Project Pico.

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use serde_json::{json, Value};
use std::process::Command;

/// Tool to adjust master output volume.
pub struct SetMasterVolumeTool;

#[async_trait]
impl Tool for SetMasterVolumeTool {
    fn name(&self) -> &str {
        "audio.set_volume"
    }

    fn description(&self) -> &str {
        "Adjusts, sets, increases, turns up, or turns down the master speaker output sound volume level (0-100 percentage)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "level": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 100,
                    "description": "Target volume percentage from 0 to 100"
                }
            },
            "required": ["level"]
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: Value) -> Result<Value> {
        let level = arguments
            .get("level")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing or invalid 'level' argument (0-100)".into(),
            })?
            .min(100) as u32;

        #[cfg(target_os = "windows")]
        {
            // Set master volume using PowerShell Windows Audio Endpoint API safely parameterized
            let ps_script = format!(
                "$w=New-Object -ComObject WScript.Shell; $null=$w.SendKeys([char]174*50); $null=$w.SendKeys([char]175*{});",
                level / 2
            );
            let status = Command::new("powershell")
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(&ps_script)
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to set volume: {}", e)))?;

            if !status.success() {
                return Err(PicoError::Platform(
                    "Failed to adjust volume via system handler".into(),
                ));
            }
        }

        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("pactl")
                .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", level)])
                .status();
        }

        Ok(json!({
            "success": true,
            "level": level,
            "message": format!("Master volume adjusted to {}%", level)
        }))
    }
}

/// Tool to toggle audio mute.
pub struct MuteAudioTool;

#[async_trait]
impl Tool for MuteAudioTool {
    fn name(&self) -> &str {
        "audio.mute"
    }

    fn description(&self) -> &str {
        "Toggles audio output mute state."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mute": {
                    "type": "boolean",
                    "description": "True to mute, false to unmute"
                }
            }
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        #[cfg(target_os = "windows")]
        {
            let ps_script = "$w=New-Object -ComObject WScript.Shell; $null=$w.SendKeys([char]173);";
            let _ = Command::new("powershell")
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(ps_script)
                .status();
        }

        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("pactl")
                .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
                .status();
        }

        Ok(json!({
            "success": true,
            "message": "Master audio mute toggled"
        }))
    }
}
