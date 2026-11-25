//! Curated Verified Operating System Tool Catalog for Project Pico.
//! Every registered entry is verified for unique command adapters, valid parameter substitution,
//! honest permission tiers, and cross-platform isolation.

pub mod adapter;
pub mod linux;
pub mod types;
pub mod windows;

pub use adapter::execute_tool_adapter;
pub use linux::{get_linux_catalog, get_linux_catalog_with_sandbox};
pub use types::{
    ActionDef, CategoryDef, EcosystemTool, LatencyClass, ParamDef, Reversibility, TargetOs,
    ToolMetadata,
};
pub use windows::{get_windows_catalog, get_windows_catalog_with_sandbox};

use pico_core::traits::Tool;
use pico_security::PathSandbox;
use std::sync::Arc;

/// Returns all curated Windows and Linux catalog tools with a default sandbox.
pub fn get_massive_tool_catalog() -> Vec<Arc<dyn Tool>> {
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    get_massive_tool_catalog_with_sandbox(Arc::new(PathSandbox::new(vec![root], vec![])))
}

/// Returns all curated Windows and Linux catalog tools initialized with a shared PathSandbox.
pub fn get_massive_tool_catalog_with_sandbox(sandbox: Arc<PathSandbox>) -> Vec<Arc<dyn Tool>> {
    let mut catalog = Vec::with_capacity(128);
    catalog.extend(get_windows_catalog_with_sandbox(sandbox.clone()));
    catalog.extend(get_linux_catalog_with_sandbox(sandbox));
    catalog
}

/// Returns only verified catalog tools applicable to the running host operating system.
pub fn get_host_os_catalog_with_sandbox(sandbox: Arc<PathSandbox>) -> Vec<Arc<dyn Tool>> {
    #[cfg(target_os = "windows")]
    {
        get_windows_catalog_with_sandbox(sandbox)
    }
    #[cfg(target_os = "linux")]
    {
        get_linux_catalog_with_sandbox(sandbox)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = sandbox;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_windows_catalog_uniqueness_and_non_duplication() {
        let tools = get_windows_catalog();
        assert!(tools.len() >= 40, "Expected >= 40 verified Windows tools");

        let mut names = HashSet::new();
        for t in &tools {
            assert!(t.name().starts_with("windows."));
            assert!(names.insert(t.name().to_string()));
            assert!(!t.description().is_empty());
            assert!(t.parameters_schema().is_object());
        }

        let mut cmds = HashSet::new();
        for cat in crate::catalog::windows::WINDOWS_CATEGORIES {
            for action in cat.actions {
                let sig = (action.adapter_cmd, action.adapter_args);
                assert!(
                    cmds.insert(sig),
                    "Duplicate Windows adapter signature detected on {}.{}: {:?}",
                    cat.name,
                    action.subaction,
                    sig
                );
            }
        }
    }

    #[test]
    fn test_linux_catalog_uniqueness_and_non_duplication() {
        let tools = get_linux_catalog();
        assert!(tools.len() >= 30, "Expected >= 30 verified Linux tools");

        let mut names = HashSet::new();
        for t in &tools {
            assert!(t.name().starts_with("linux."));
            assert!(names.insert(t.name().to_string()));
            assert!(!t.description().is_empty());
            assert!(t.parameters_schema().is_object());
        }

        let mut cmds = HashSet::new();
        for cat in crate::catalog::linux::LINUX_CATEGORIES {
            for action in cat.actions {
                assert!(
                    !action.adapter_cmd.contains("Get-")
                        && !action.adapter_cmd.contains("Set-")
                        && !action.adapter_cmd.ends_with(".exe"),
                    "Linux action {}.{} uses Windows command",
                    cat.name,
                    action.subaction
                );
                let sig = (action.adapter_cmd, action.adapter_args);
                assert!(
                    cmds.insert(sig),
                    "Duplicate Linux adapter signature detected on {}.{}: {:?}",
                    cat.name,
                    action.subaction,
                    sig
                );
            }
        }
    }

    #[tokio::test]
    async fn test_sample_tool_execution_or_cross_os_rejection() {
        let tools = get_windows_catalog();
        let list_dir = tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .expect("list_directory tool missing");

        let res = list_dir.execute(serde_json::json!({ "path": "." })).await;
        #[cfg(target_os = "windows")]
        assert!(res.is_ok(), "Execution failed on Windows: {:?}", res.err());
        #[cfg(not(target_os = "windows"))]
        assert!(res.is_err(), "Cross-OS Windows tool must fail on non-Windows hosts");
    }

    #[tokio::test]
    async fn test_ecosystem_filesystem_tool_enforces_sandbox() {
        let tools = get_windows_catalog();
        let list_dir = tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .expect("list_directory tool missing");

        let res = list_dir
            .execute(serde_json::json!({ "path": "../../Windows/System32" }))
            .await;
        assert!(res.is_err(), "Ecosystem tool must reject traversal");
    }
}
