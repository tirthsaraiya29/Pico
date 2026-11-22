//! Massive Operating System Tool Ecosystem for Project Pico.
//! Exposes 1,000+ Windows tools and 1,000+ Linux tools across 22 operational domains.

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

/// Returns the complete combined ecosystem catalog (> 2,000 distinct tools) with default sandbox.
pub fn get_massive_tool_catalog() -> Vec<Arc<dyn Tool>> {
    get_massive_tool_catalog_with_sandbox(Arc::new(PathSandbox::new(
        vec![std::path::PathBuf::from(".")],
        vec![],
    )))
}

/// Returns the complete combined ecosystem catalog initialized with a shared PathSandbox.
pub fn get_massive_tool_catalog_with_sandbox(sandbox: Arc<PathSandbox>) -> Vec<Arc<dyn Tool>> {
    let mut catalog = Vec::with_capacity(2200);
    catalog.extend(get_windows_catalog_with_sandbox(sandbox.clone()));
    catalog.extend(get_linux_catalog_with_sandbox(sandbox));
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_windows_catalog_size_and_uniqueness() {
        let tools = get_windows_catalog();
        assert!(
            tools.len() >= 1000,
            "Windows tools must exceed 1000, got {}",
            tools.len()
        );

        let mut names = HashSet::new();
        for t in &tools {
            assert!(
                t.name().starts_with("windows."),
                "Tool name must start with 'windows.': {}",
                t.name()
            );
            assert!(
                names.insert(t.name().to_string()),
                "Duplicate tool name found: {}",
                t.name()
            );
            assert!(!t.description().is_empty());
            assert!(t.parameters_schema().is_object());
        }
    }

    #[test]
    fn test_linux_catalog_size_and_uniqueness() {
        let tools = get_linux_catalog();
        assert!(
            tools.len() >= 1000,
            "Linux tools must exceed 1000, got {}",
            tools.len()
        );

        let mut names = HashSet::new();
        for t in &tools {
            assert!(
                t.name().starts_with("linux."),
                "Tool name must start with 'linux.': {}",
                t.name()
            );
            assert!(
                names.insert(t.name().to_string()),
                "Duplicate tool name found: {}",
                t.name()
            );
            assert!(!t.description().is_empty());
            assert!(t.parameters_schema().is_object());
        }
    }

    #[test]
    fn test_combined_massive_catalog() {
        let catalog = get_massive_tool_catalog();
        assert!(
            catalog.len() >= 2000,
            "Combined catalog must exceed 2000, got {}",
            catalog.len()
        );
    }

    #[tokio::test]
    async fn test_sample_tool_execution() {
        let tools = get_windows_catalog();
        let list_dir = tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .expect("list_directory tool missing");

        // Execute list_directory with parameter
        let res = list_dir.execute(serde_json::json!({ "path": "." })).await;
        assert!(res.is_ok(), "Execution failed: {:?}", res.err());
    }

    #[tokio::test]
    async fn test_ecosystem_filesystem_tool_enforces_sandbox() {
        let tools = get_windows_catalog();
        let list_dir = tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .expect("list_directory tool missing");

        // Attempt directory traversal out of sandbox
        let res = list_dir
            .execute(serde_json::json!({ "path": "../../Windows/System32" }))
            .await;
        assert!(res.is_err(), "Ecosystem tool must reject traversal");
    }
}
