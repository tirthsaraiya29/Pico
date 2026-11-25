//! Power management and workstation lock tools.

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

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
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        #[cfg(target_os = "windows")]
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("rundll32.exe")
                .args(["user32.dll,LockWorkStation"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| PicoError::Platform("Workstation lock command timed out".into()))?
        .map_err(|e| PicoError::Platform(format!("Failed to lock workstation: {}", e)))?;

        #[cfg(target_os = "linux")]
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("loginctl")
                .args(["lock-session"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| PicoError::Platform("Workstation lock command timed out".into()))?
        .map_err(|e| PicoError::Platform(format!("Failed to run loginctl: {}", e)))?;

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if !status.success() {
            return Err(PicoError::Platform(format!(
                "Lock workstation command failed with status {}",
                status
            )));
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
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, _arguments: Value) -> Result<Value> {
        #[cfg(target_os = "windows")]
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("rundll32.exe")
                .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| PicoError::Platform("System suspend command timed out".into()))?
        .map_err(|e| PicoError::Platform(format!("Failed to suspend computer: {}", e)))?;

        #[cfg(target_os = "linux")]
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("systemctl")
                .args(["suspend"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| PicoError::Platform("System suspend command timed out".into()))?
        .map_err(|e| PicoError::Platform(format!("Failed to suspend system: {}", e)))?;

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if !status.success() {
            return Err(PicoError::Platform(format!(
                "System suspend command failed with status {}",
                status
            )));
        }

        Ok(json!({
            "success": true,
            "message": "System suspend initiated"
        }))
    }
}
