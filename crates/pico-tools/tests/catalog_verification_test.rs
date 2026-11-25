//! Comprehensive Verification Suite for Pico's Curated OS Tool Catalog.
//! Enforces unique adapter mappings, valid parameter bindings, honest permission tiers,
//! sandbox containment, injection safety, and real host OS execution.

use pico_core::traits::PermissionTier;
use pico_security::PathSandbox;
use pico_tools::catalog::linux::LINUX_CATEGORIES;
use pico_tools::catalog::windows::WINDOWS_CATEGORIES;
use pico_tools::catalog::{get_linux_catalog, get_massive_tool_catalog, get_windows_catalog};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

fn extract_placeholders(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut text = template;
    while let Some(dollar_idx) = text.find('$') {
        let rest = &text[dollar_idx + 1..];
        if let Some(after_brace) = rest.strip_prefix('{') {
            if let Some(end_brace) = after_brace.find('}') {
                let param_name = &after_brace[..end_brace];
                if !param_name.is_empty() && !param_name.chars().all(|c| c.is_numeric()) {
                    names.push(param_name.to_string());
                }
                text = &after_brace[end_brace + 1..];
                continue;
            }
        }
        let param_len = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .count();
        if param_len > 0 {
            let param_name = &rest[..param_len];
            // Ignore PowerShell automatic runtime variables like $PSItem
            if param_name != "PSItem" && !param_name.chars().all(|c| c.is_numeric()) {
                names.push(param_name.to_string());
            }
            text = &rest[param_len..];
        } else {
            text = rest;
        }
    }
    names
}

#[test]
fn test_all_curated_catalogue_entries_structure_and_uniqueness() {
    let all_tools = get_massive_tool_catalog();
    let windows_tools = get_windows_catalog();
    let linux_tools = get_linux_catalog();

    assert_eq!(all_tools.len(), windows_tools.len() + linux_tools.len());
    assert!(windows_tools.len() >= 40);
    assert!(linux_tools.len() >= 30);

    let mut canonical_ids = HashSet::new();
    for tool in &all_tools {
        let name = tool.name();
        assert!(
            name.starts_with("windows.") || name.starts_with("linux."),
            "Unexpected canonical ID '{}'",
            name
        );
        assert!(canonical_ids.insert(name.to_string()), "Duplicate ID {}", name);

        let desc = tool.description();
        assert!(desc.len() >= 10, "Short description on {}", name);

        let schema = tool.parameters_schema();
        assert_eq!(schema.get("type").and_then(|t| t.as_str()), Some("object"));
        if let Some(req_arr) = schema.get("required").and_then(|r| r.as_array()) {
            let prop_map = schema.get("properties").and_then(|p| p.as_object()).unwrap();
            for req in req_arr {
                let req_key = req.as_str().unwrap();
                assert!(
                    prop_map.contains_key(req_key),
                    "Tool '{}' missing declared property '{}'",
                    name,
                    req_key
                );
            }
        }
    }
}

#[test]
fn test_all_windows_catalogue_adapters_and_param_contracts() {
    let mut seen_sigs = HashSet::new();

    for cat in WINDOWS_CATEGORIES {
        assert!(!cat.name.is_empty() && !cat.domain_label.is_empty());
        for action in cat.actions {
            let full_id = format!("windows.{}.{}", cat.name, action.subaction);
            assert!(
                seen_sigs.insert((action.adapter_cmd, action.adapter_args)),
                "Duplicate Windows command/args detected on {}",
                full_id
            );

            let placeholders = extract_placeholders(action.adapter_args);
            for param_name in &placeholders {
                assert!(
                    action.params.iter().any(|p| p.name == param_name),
                    "Action '{}' references undeclared parameter '${}'",
                    full_id,
                    param_name
                );
            }
            for declared in action.params {
                assert!(
                    placeholders.iter().any(|p| p == declared.name),
                    "Action '{}' declares unused parameter '{}'",
                    full_id,
                    declared.name
                );
            }

            let lower_sub = action.subaction.to_lowercase();
            let mutating = lower_sub.contains("delete")
                || lower_sub.contains("remove")
                || lower_sub.contains("copy")
                || lower_sub.contains("move")
                || lower_sub.contains("create")
                || lower_sub.contains("start")
                || lower_sub.contains("stop")
                || lower_sub.contains("flush");
            if mutating {
                assert_ne!(
                    action.tier,
                    PermissionTier::ReadOnly,
                    "Mutating action '{}' cannot be ReadOnly",
                    full_id
                );
            }
        }
    }
}

#[test]
fn test_all_linux_catalogue_adapters_and_param_contracts() {
    let mut seen_sigs = HashSet::new();

    for cat in LINUX_CATEGORIES {
        assert!(!cat.name.is_empty() && !cat.domain_label.is_empty());
        for action in cat.actions {
            let full_id = format!("linux.{}.{}", cat.name, action.subaction);
            assert!(
                seen_sigs.insert((action.adapter_cmd, action.adapter_args)),
                "Duplicate Linux command/args detected on {}",
                full_id
            );

            assert!(
                !action.adapter_cmd.contains("Get-")
                    && !action.adapter_cmd.contains("Set-")
                    && !action.adapter_cmd.ends_with(".exe"),
                "Invalid Linux command on '{}': {}",
                full_id,
                action.adapter_cmd
            );

            let mut placeholders = extract_placeholders(action.adapter_args);
            placeholders.extend(extract_placeholders(action.adapter_cmd));
            for param_name in &placeholders {
                assert!(
                    action.params.iter().any(|p| p.name == param_name),
                    "Action '{}' references undeclared parameter '${}'",
                    full_id,
                    param_name
                );
            }
            for declared in action.params {
                assert!(
                    placeholders.iter().any(|p| p == declared.name),
                    "Action '{}' declares unused parameter '{}'",
                    full_id,
                    declared.name
                );
            }

            let lower_sub = action.subaction.to_lowercase();
            let mutating = lower_sub.contains("delete")
                || lower_sub.contains("remove")
                || lower_sub.contains("copy")
                || lower_sub.contains("move")
                || lower_sub.contains("create")
                || lower_sub.contains("start")
                || lower_sub.contains("stop");
            if mutating {
                assert_ne!(
                    action.tier,
                    PermissionTier::ReadOnly,
                    "Mutating action '{}' cannot be ReadOnly",
                    full_id
                );
            }
        }
    }
}

#[tokio::test]
async fn test_live_host_os_catalog_execution_and_cross_os_fail_closed() {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let sandbox = Arc::new(PathSandbox::new(vec![root], vec![]));

    #[cfg(target_os = "windows")]
    {
        let windows_tools = pico_tools::catalog::get_windows_catalog_with_sandbox(sandbox.clone());
        let list_dir = windows_tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .unwrap();
        let res = list_dir.execute(serde_json::json!({ "path": "." })).await;
        assert!(res.is_ok(), "Live Windows execution failed: {:?}", res.err());

        let linux_tools = pico_tools::catalog::get_linux_catalog_with_sandbox(sandbox);
        let linux_list = linux_tools
            .iter()
            .find(|t| t.name() == "linux.filesystem.list_directory")
            .unwrap();
        let cross = linux_list.execute(serde_json::json!({ "path": "." })).await;
        assert!(cross.is_err(), "Linux tool must fail cleanly when invoked on Windows");
    }

    #[cfg(target_os = "linux")]
    {
        let linux_tools = pico_tools::catalog::get_linux_catalog_with_sandbox(sandbox.clone());
        let list_dir = linux_tools
            .iter()
            .find(|t| t.name() == "linux.filesystem.list_directory")
            .unwrap();
        let res = list_dir.execute(serde_json::json!({ "path": "." })).await;
        assert!(res.is_ok(), "Live Linux execution failed: {:?}", res.err());

        let windows_tools = pico_tools::catalog::get_windows_catalog_with_sandbox(sandbox);
        let win_list = windows_tools
            .iter()
            .find(|t| t.name() == "windows.filesystem.list_directory")
            .unwrap();
        let cross = win_list.execute(serde_json::json!({ "path": "." })).await;
        assert!(cross.is_err(), "Windows tool must fail cleanly when invoked on Linux");
    }
}

#[tokio::test]
async fn test_sandbox_and_injection_enforcement_across_catalogue() {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let sandbox = Arc::new(PathSandbox::new(
        vec![root],
        vec![PathBuf::from("C:\\Windows\\System32")],
    ));
    let all_tools = pico_tools::catalog::get_massive_tool_catalog_with_sandbox(sandbox);

    let path_tools: Vec<_> = all_tools
        .iter()
        .filter(|t| {
            t.parameters_schema()
                .get("properties")
                .and_then(|p| p.get("path"))
                .is_some()
        })
        .collect();

    assert!(path_tools.len() >= 15);
    for tool in &path_tools {
        let traversal = serde_json::json!({ "path": "../../../../../Windows/System32/cmd.exe" });
        let result = tool.execute(traversal).await;
        assert!(
            result.is_err(),
            "Tool '{}' allowed traversal escape!",
            tool.name()
        );
    }

    // PowerShell / shell injection payload test
    let win_find = all_tools
        .iter()
        .find(|t| t.name() == "windows.process.find_process_by_name")
        .unwrap();
    let inj = win_find
        .execute(serde_json::json!({ "name": "explorer; Start-Process calc" }))
        .await;
    assert!(inj.is_err(), "Catalog adapter must block shell metacharacters");
}
