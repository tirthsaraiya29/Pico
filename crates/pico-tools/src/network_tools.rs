//! Network diagnostics and connectivity tools.

use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use serde_json::{json, Value};
use std::process::Command;

/// Tool to test network latency via ping.
pub struct PingHostTool;

#[async_trait]
impl Tool for PingHostTool {
    fn name(&self) -> &str {
        "network.ping"
    }

    fn description(&self) -> &str {
        "Tests network latency and round-trip time to a specified host or IP address."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "host": {
                    "type": "string",
                    "description": "Host name or IP address (e.g. 1.1.1.1, google.com)"
                }
            },
            "required": ["host"]
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: Value) -> Result<Value> {
        let host = arguments
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("1.1.1.1");

        // Validate host does not contain shell injection characters
        if host.contains('&')
            || host.contains('|')
            || host.contains(';')
            || host.contains('`')
            || host.contains('$')
            || host.contains(' ')
        {
            return Err(PicoError::Security(
                "Invalid characters in host string".into(),
            ));
        }

        #[cfg(target_os = "windows")]
        let output = Command::new("ping")
            .args(["-n", "2", host])
            .output()
            .map_err(|e| PicoError::Platform(format!("Failed to execute ping: {}", e)))?;

        #[cfg(not(target_os = "windows"))]
        let output = Command::new("ping")
            .args(["-c", "2", host])
            .output()
            .map_err(|e| PicoError::Platform(format!("Failed to execute ping: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let success = output.status.success();

        Ok(json!({
            "host": host,
            "success": success,
            "output": stdout.trim()
        }))
    }
}
