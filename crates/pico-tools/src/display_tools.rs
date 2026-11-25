//! Display brightness control tools across Windows and Linux.

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_platform::PlatformDisplay;
use serde_json::{json, Value};

/// Tool to query current primary display brightness percentage.
pub struct GetBrightnessTool;

#[async_trait]
impl Tool for GetBrightnessTool {
    fn name(&self) -> &str {
        "display.get_brightness"
    }

    fn description(&self) -> &str {
        "Gets the current screen display brightness percentage (0 to 100)."
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
        let level = PlatformDisplay::get_brightness()?;
        Ok(json!({
            "success": true,
            "level": level,
            "message": format!("Display brightness is {}%", level)
        }))
    }
}

/// Tool to adjust screen brightness level (absolute 0..=100 or relative delta step).
pub struct SetBrightnessTool;

#[async_trait]
impl Tool for SetBrightnessTool {
    fn name(&self) -> &str {
        "display.set_brightness"
    }

    fn description(&self) -> &str {
        "Increases, decreases, dims, or sets the screen display brightness level (0 to 100 percent)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "level": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 100,
                    "description": "Target brightness percentage (0 to 100)"
                },
                "delta": {
                    "type": "integer",
                    "minimum": -100,
                    "maximum": 100,
                    "description": "Relative brightness adjustment (e.g. +15 or -15)"
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: Value) -> Result<Value> {
        let level = if let Some(abs_lvl) = arguments.get("level").and_then(|v| v.as_u64()) {
            PlatformDisplay::set_brightness(abs_lvl.min(100) as u32)?
        } else if let Some(delta) = arguments.get("delta").and_then(|v| v.as_i64()) {
            PlatformDisplay::adjust_brightness(delta as i32)?
        } else {
            return Err(PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'level' or 'delta' argument for brightness".into(),
            });
        };

        Ok(json!({
            "success": true,
            "level": level,
            "message": format!("Display brightness set to {}%", level)
        }))
    }
}
