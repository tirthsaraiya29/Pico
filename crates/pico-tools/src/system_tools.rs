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

/// Tool to terminate a running process by PID or process name.
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
                },
                "name": {
                    "type": "string",
                    "description": "Process executable name to terminate"
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        if let Some(pid) = arguments.get("pid").and_then(|v| v.as_u64()) {
            PlatformApps::kill_process(pid as u32)?;
            return Ok(json!({ "terminated_pid": pid, "status": "success" }));
        }
        if let Some(name) = arguments.get("name").and_then(|v| v.as_str()) {
            let count = PlatformApps::close_application(name)?;
            return Ok(json!({
                "terminated_name": name,
                "terminated_count": count,
                "status": "success"
            }));
        }
        Err(PicoError::ToolExecution {
            tool: self.name().into(),
            message: "Missing 'pid' or 'name' argument".into(),
        })
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
        "Opens and launches an installed desktop application, browser, program, or website (e.g. open chrome, brave, edge, notepad, calc, code, terminal, explorer, settings)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "application": {
                    "type": "string",
                    "description": "Application name, program, or executable path (e.g. 'notepad', 'calc', 'chrome')"
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
            .or_else(|| arguments.get("target"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message:
                    "Missing 'application' argument. Which application would you like to open?"
                        .into(),
            })?;

        PlatformApps::open_application(app)?;
        Ok(json!({ "opened": app, "status": "success" }))
    }
}

/// Tool to cleanly close or stop a running user desktop application by name.
pub struct CloseApplicationTool;

#[async_trait]
impl Tool for CloseApplicationTool {
    fn name(&self) -> &str {
        "close_application"
    }

    fn description(&self) -> &str {
        "Closes, quits, exits, or terminates a running user desktop application or browser (e.g. close brave, quit chrome, close notepad)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "application": {
                    "type": "string",
                    "description": "Name of the running application to close (e.g. 'brave', 'chrome', 'notepad')"
                }
            },
            "required": ["application"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let app = arguments
            .get("application")
            .or_else(|| arguments.get("name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'application' argument specifying what to close".into(),
            })?;

        let count = PlatformApps::close_application(app)?;
        Ok(json!({
            "closed": app,
            "terminated_count": count,
            "status": "success",
            "message": format!("Closed {} ({} process(es) terminated).", app, count)
        }))
    }
}
