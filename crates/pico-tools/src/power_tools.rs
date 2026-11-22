//! Power management and workstation lock tools.

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use serde_json::{json, Value};
use std::process::Command;

/// Tool to lock the workstation session.
pub struct LockScreenTool;

#[async_trait]
impl Tool for LockScreenTool {
    fn name(&self) -> &str {
        "power.lock"
    }

    fn description(&self) -> &str {
        "Locks the workstation or current user desktop session immediately."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        #[cfg(target_os = "windows")]
        {
            let status = Command::new("rundll32.exe")
                .args(["user32.dll,LockWorkStation"])
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to lock workstation: {}", e)))?;

            if !status.success() {
                return Err(PicoError::Platform(
                    "Failed to execute lock workstation command".into(),
                ));
            }
        }

        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("loginctl").args(["lock-session"]).status();
        }

        Ok(json!({
            "success": true,
            "message": "Workstation locked successfully"
        }))
    }
}

/// Tool to put computer to sleep.
pub struct SleepSystemTool;

#[async_trait]
impl Tool for SleepSystemTool {
    fn name(&self) -> &str {
        "power.sleep"
    }

    fn description(&self) -> &str {
        "Puts the computer into sleep or suspend mode. Requires confirmation."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        #[cfg(target_os = "windows")]
        {
            let status = Command::new("rundll32.exe")
                .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                .status()
                .map_err(|e| PicoError::Platform(format!("Failed to suspend computer: {}", e)))?;

            if !status.success() {
                return Err(PicoError::Platform("Failed to suspend system".into()));
            }
        }

        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("systemctl").args(["suspend"]).status();
        }

        Ok(json!({
            "success": true,
            "message": "System suspend initiated"
        }))
    }
}
