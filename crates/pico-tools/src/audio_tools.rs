//! Audio and sound control tools using native OS audio endpoints (WASAPI / wpctl / pactl).

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_platform::PlatformAudio;
use serde_json::{json, Value};

/// Tool to query current master output volume and mute status.
pub struct GetMasterVolumeTool;

#[async_trait]
impl Tool for GetMasterVolumeTool {
    fn name(&self) -> &str {
        "audio.get_volume"
    }

    fn description(&self) -> &str {
        "Gets the current master system audio output volume percentage and mute status."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        let state = PlatformAudio::get_state()?;
        Ok(json!({
            "success": true,
            "level": state.volume_percent,
            "muted": state.muted,
            "backend": state.backend
        }))
    }
}

/// Tool to adjust master output volume (absolute percentage 0..=100 or relative delta).
pub struct SetMasterVolumeTool;

#[async_trait]
impl Tool for SetMasterVolumeTool {
    fn name(&self) -> &str {
        "audio.set_volume"
    }

    fn description(&self) -> &str {
        "Sets, increases, turns up, or turns down the master speaker output volume level (0 to 100 percent)."
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
                },
                "delta": {
                    "type": "integer",
                    "minimum": -100,
                    "maximum": 100,
                    "description": "Relative volume change percentage when no absolute level is specified"
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: Value) -> Result<Value> {
        let target = if let Some(lvl) = arguments.get("level").and_then(|v| v.as_u64()) {
            lvl.min(100) as u32
        } else if let Some(delta) = arguments.get("delta").and_then(|v| v.as_i64()) {
            let current = PlatformAudio::get_state()?.volume_percent as i64;
            (current + delta).clamp(0, 100) as u32
        } else {
            return Err(PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'level' (0-100) or 'delta' argument".into(),
            });
        };

        let state = PlatformAudio::set_volume(target)?;
        Ok(json!({
            "success": true,
            "level": state.volume_percent,
            "muted": state.muted,
            "backend": state.backend,
            "message": format!("Master volume set to {}%", state.volume_percent)
        }))
    }
}

/// Tool to mute or unmute system audio output cleanly.
pub struct MuteAudioTool;

#[async_trait]
impl Tool for MuteAudioTool {
    fn name(&self) -> &str {
        "audio.mute"
    }

    fn description(&self) -> &str {
        "Mutes or unmutes the system master audio output. Pass mute=true to mute, mute=false to unmute."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mute": {
                    "type": "boolean",
                    "description": "True to mute audio, false to unmute audio"
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: Value) -> Result<Value> {
        let state = if let Some(explicit_mute) = arguments.get("mute").and_then(|v| v.as_bool()) {
            PlatformAudio::set_mute(explicit_mute)?
        } else {
            PlatformAudio::toggle_mute()?
        };

        Ok(json!({
            "success": true,
            "muted": state.muted,
            "level": state.volume_percent,
            "backend": state.backend,
            "message": if state.muted {
                "Audio output muted"
            } else {
                "Audio output unmuted"
            }
        }))
    }
}
