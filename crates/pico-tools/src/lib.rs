pub mod audio_tools;
pub mod calculator_parser;
pub mod command_tools;
pub mod filesystem_tools;
pub mod network_tools;
pub mod power_tools;
pub mod registry;
pub mod system_tools;
pub mod utility_tools;

pub use audio_tools::{MuteAudioTool, SetMasterVolumeTool};
pub use command_tools::ExecuteShellCommandTool;
pub use filesystem_tools::{ReadFileTool, SearchFilesTool, WriteFileTool};
pub use network_tools::PingHostTool;
pub use power_tools::{LockScreenTool, SleepSystemTool};
pub use registry::ToolRegistry;
pub use system_tools::{
    GetSystemInfoTool, KillProcessTool, ListProcessesTool, OpenApplicationTool,
};
pub use utility_tools::{CalculatorTool, ClipboardTool, FetchWebContentTool, ManageTimerTool};

use pico_security::{AuditLogger, PathSandbox, SecurityPolicy};
use std::sync::Arc;

/// Builds the default standard tool registry populated with all core OS and utility tools.
pub fn build_default_registry(
    security_policy: SecurityPolicy,
    sandbox: Arc<PathSandbox>,
    audit_logger: Option<Arc<AuditLogger>>,
) -> ToolRegistry {
    let mut reg = ToolRegistry::new(security_policy, audit_logger);

    // System tools
    reg.register(Arc::new(GetSystemInfoTool));
    reg.register(Arc::new(ListProcessesTool));
    reg.register(Arc::new(KillProcessTool));
    reg.register(Arc::new(OpenApplicationTool));

    // Audio tools
    reg.register(Arc::new(SetMasterVolumeTool));
    reg.register(Arc::new(MuteAudioTool));

    // Power tools
    reg.register(Arc::new(LockScreenTool));
    reg.register(Arc::new(SleepSystemTool));

    // Network tools
    reg.register(Arc::new(PingHostTool));

    // File tools
    reg.register(Arc::new(ReadFileTool::new(sandbox.clone())));
    reg.register(Arc::new(WriteFileTool::new(sandbox.clone())));
    reg.register(Arc::new(SearchFilesTool::new(sandbox)));

    // Command tool
    reg.register(Arc::new(ExecuteShellCommandTool));

    // Utility tools
    reg.register(Arc::new(CalculatorTool));
    reg.register(Arc::new(ManageTimerTool));
    reg.register(Arc::new(ClipboardTool));
    reg.register(Arc::new(FetchWebContentTool::new()));

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::Tool;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_calculator_tool() {
        let tool = CalculatorTool;
        let res = tool
            .execute(serde_json::json!({ "expression": "15 * 4 + 10" }))
            .await;
        assert!(res.is_ok());
        let val = res.unwrap();
        assert_eq!(val["result"].as_str().unwrap(), "70");
    }

    #[tokio::test]
    async fn test_system_info_tool() {
        let tool = GetSystemInfoTool;
        let res = tool.execute(serde_json::json!({})).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_registry_tool_definitions() {
        let sandbox = Arc::new(PathSandbox::new(vec![PathBuf::from(".")], vec![]));
        let policy = SecurityPolicy::new(false, false);
        let registry = build_default_registry(policy, sandbox, None);

        let defs = registry.get_tool_definitions();
        assert!(defs.len() >= 10);
        let has_calc = defs.iter().any(|d| {
            d.pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                == "calculator"
        });
        assert!(has_calc);
    }
}
