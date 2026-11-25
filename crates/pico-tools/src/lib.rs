pub mod audio_tools;
pub mod calculator_parser;
pub mod catalog;
pub mod command_tools;
pub mod display_tools;
pub mod filesystem_tools;
pub mod network_tools;
pub mod power_tools;
pub mod registry;
pub mod system_tools;
pub mod utility_tools;

pub use audio_tools::{GetMasterVolumeTool, MuteAudioTool, SetMasterVolumeTool};
pub use catalog::{
    get_host_os_catalog_with_sandbox, get_linux_catalog, get_massive_tool_catalog,
    get_windows_catalog,
};
pub use command_tools::ExecuteShellCommandTool;
pub use display_tools::{GetBrightnessTool, SetBrightnessTool};
pub use filesystem_tools::{ReadFileTool, SearchFilesTool, WriteFileTool};
pub use network_tools::PingHostTool;
pub use power_tools::{LockScreenTool, SleepSystemTool};
pub use registry::ToolRegistry;
pub use system_tools::{
    CloseApplicationTool, GetSystemInfoTool, KillProcessTool, ListProcessesTool,
    OpenApplicationTool,
};
pub use utility_tools::{CalculatorTool, ClipboardTool, FetchWebContentTool, ManageTimerTool};

use pico_security::{AuditLogger, PathSandbox, SecurityPolicy};
use std::sync::Arc;

/// Builds the default tool registry populated with universal built-in tools
/// plus only verified catalog tools targeting the current host OS.
pub fn build_default_registry(
    security_policy: SecurityPolicy,
    sandbox: Arc<PathSandbox>,
    audit_logger: Option<Arc<AuditLogger>>,
) -> ToolRegistry {
    let mut reg = ToolRegistry::new(security_policy, audit_logger);

    // System & Application Lifecycle
    reg.register(Arc::new(GetSystemInfoTool));
    reg.register(Arc::new(ListProcessesTool));
    reg.register(Arc::new(KillProcessTool));
    reg.register(Arc::new(OpenApplicationTool));
    reg.register(Arc::new(CloseApplicationTool));

    // Native Audio & Display Controls
    reg.register(Arc::new(GetMasterVolumeTool));
    reg.register(Arc::new(SetMasterVolumeTool));
    reg.register(Arc::new(MuteAudioTool));
    reg.register(Arc::new(GetBrightnessTool));
    reg.register(Arc::new(SetBrightnessTool));

    // Power Tools
    reg.register(Arc::new(LockScreenTool));
    reg.register(Arc::new(SleepSystemTool));

    // Network Tools
    reg.register(Arc::new(PingHostTool));

    // Sandboxed Filesystem Tools
    reg.register(Arc::new(ReadFileTool::new(sandbox.clone())));
    reg.register(Arc::new(WriteFileTool::new(sandbox.clone())));
    reg.register(Arc::new(SearchFilesTool::new(sandbox.clone())));

    // Shell Tool (ConfirmationRequired)
    reg.register(Arc::new(ExecuteShellCommandTool));

    // Utility Tools
    reg.register(Arc::new(CalculatorTool));
    reg.register(Arc::new(ManageTimerTool::default()));
    reg.register(Arc::new(ClipboardTool));
    reg.register(Arc::new(FetchWebContentTool::new()));

    // Register verified host-OS-specific catalog actions (avoids polluting retrieval with foreign OS tools)
    reg.register_batch(catalog::get_host_os_catalog_with_sandbox(sandbox));

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::traits::Tool;
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
    async fn test_registry_tool_definitions_host_os_filtered() {
        let sandbox = Arc::new(PathSandbox::new(vec![PathBuf::from(".")], vec![]));
        let policy = SecurityPolicy::new(false, false);
        let reg = build_default_registry(policy, sandbox, None);

        let defs = reg.get_tool_definitions();
        assert!(defs.len() >= 20);
        #[cfg(target_os = "windows")]
        assert!(defs
            .iter()
            .all(|d| !d["function"]["name"].as_str().unwrap().starts_with("linux.")));
        #[cfg(target_os = "linux")]
        assert!(defs
            .iter()
            .all(|d| !d["function"]["name"].as_str().unwrap().starts_with("windows.")));
    }
}
