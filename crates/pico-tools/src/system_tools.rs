use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_platform::{PlatformApps, PlatformSysInfo};
use serde_json::json;

/// Tool to retrieve current system hardware, memory, and OS state.
pub struct GetSystemInfoTool;

#[async_trait]
impl Tool for GetSystemInfoTool {
    fn name(&self) -> &str {
        "get_system_info"
    }

    fn description(&self) -> &str {
        "Retrieves system information, computer hardware specs, operating system details, CPU cores, RAM and memory usage, and battery status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, _arguments: serde_json::Value) -> Result<serde_json::Value> {
        let stats = PlatformSysInfo::get_system_stats()?;
        Ok(json!(stats))
    }
}

/// Tool to list running processes on the system.
pub struct ListProcessesTool;

#[async_trait]
impl Tool for ListProcessesTool {
    fn name(&self) -> &str {
        "list_processes"
    }

    fn description(&self) -> &str {
        "Lists active running processes, applications, tasks, memory usage, and process identifiers sorted by memory consumption."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of processes to return (default 10)",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let procs = PlatformApps::list_processes(limit)?;
        Ok(json!({ "processes": procs }))
    }
}

/// Tool to terminate a running process.
pub struct KillProcessTool;

#[async_trait]
impl Tool for KillProcessTool {
    fn name(&self) -> &str {
        "kill_process"
    }

    fn description(&self) -> &str {
        "Terminates, stops, kills, or ends an unresponsive process or task by PID or process name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "integer",
                    "description": "Process ID to terminate"
                }
            },
            "required": ["pid"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let pid = arguments
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'pid' argument".into(),
            })? as u32;

        PlatformApps::kill_process(pid)?;
        Ok(json!({ "terminated_pid": pid, "status": "success" }))
    }
}

/// Tool to launch desktop applications.
pub struct OpenApplicationTool;

#[async_trait]
impl Tool for OpenApplicationTool {
    fn name(&self) -> &str {
        "open_application"
    }

    fn description(&self) -> &str {
        "Opens an installed application or file (e.g. 'notepad', 'calc', 'msedge')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "application": {
                    "type": "string",
                    "description": "Application name or executable path (e.g. 'notepad', 'calc')"
                }
            },
            "required": ["application"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let app = arguments
            .get("application")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'application' argument".into(),
            })?;

        PlatformApps::open_application(app)?;
        Ok(json!({ "opened": app, "status": "success" }))
    }
}
