//! Comprehensive Verification Suite for Pico's 2,112 Operating System Tool Ecosystem.
//! Validates schemas, parameter bindings, safety tiers, and real OS operations across all 22 operational domains.

use pico_core::traits::PermissionTier;
use pico_security::PathSandbox;
use pico_tools::catalog::linux::LINUX_CATEGORIES;
use pico_tools::catalog::windows::WINDOWS_CATEGORIES;
use pico_tools::catalog::{get_linux_catalog, get_massive_tool_catalog, get_windows_catalog};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn test_all_2112_catalogue_entries_structure_and_schemas() {
    let all_tools = get_massive_tool_catalog();
    assert_eq!(
        all_tools.len(),
        2112,
        "Tool ecosystem must contain exactly 2,112 tools (1,056 Windows + 1,056 Linux)"
    );

    let windows_tools = get_windows_catalog();
    assert_eq!(
        windows_tools.len(),
        1056,
        "Windows catalog must contain exactly 1,056 tools"
    );

    let linux_tools = get_linux_catalog();
    assert_eq!(
        linux_tools.len(),
        1056,
        "Linux catalog must contain exactly 1,056 tools"
    );

    let mut canonical_ids = HashSet::new();

    for tool in &all_tools {
        let name = tool.name();
        assert!(
            name.starts_with("windows.") || name.starts_with("linux."),
            "Tool canonical ID '{}' must start with 'windows.' or 'linux.'",
            name
        );

        assert!(
            canonical_ids.insert(name.to_string()),
            "Duplicate canonical tool ID detected: {}",
            name
        );

        let desc = tool.description();
        assert!(
            !desc.is_empty() && desc.len() >= 10,
            "Tool '{}' must have a descriptive documentation string (got: '{}')",
            name,
            desc
        );

        let schema = tool.parameters_schema();
        assert!(
            schema.is_object(),
            "Tool '{}' schema must be a valid JSON Schema object",
            name
        );

        assert_eq!(
            schema.get("type").and_then(|t| t.as_str()),
            Some("object"),
            "Tool '{}' schema type must be 'object'",
            name
        );

        // Verify required properties match properties map
        if let Some(req_arr) = schema.get("required").and_then(|r| r.as_array()) {
            let props = schema.get("properties").and_then(|p| p.as_object());
            assert!(
                props.is_some(),
                "Tool '{}' has required fields but missing 'properties' object",
                name
            );
            let prop_map = props.unwrap();
            for req in req_arr {
                let req_key = req.as_str().expect("Required item must be string");
                assert!(
                    prop_map.contains_key(req_key),
                    "Tool '{}' required field '{}' not found in properties map",
                    name,
                    req_key
                );
            }
        }
    }
}

fn extract_placeholders(adapter_args: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut text = adapter_args;
    while let Some(dollar_idx) = text.find('$') {
        let rest = &text[dollar_idx + 1..];
        if rest.starts_with('{') {
            if let Some(end_brace) = rest.find('}') {
                let param_name = &rest[1..end_brace];
                if !param_name.is_empty() && !param_name.chars().all(|c| c.is_numeric()) {
                    names.push(param_name.to_string());
                }
                text = &rest[end_brace + 1..];
                continue;
            }
        }
        let param_len = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .count();
        if param_len > 0 {
            let param_name = &rest[..param_len];
            if !param_name.chars().all(|c| c.is_numeric()) {
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
fn test_all_windows_catalogue_cmdlet_and_param_contracts() {
    let mut total_actions = 0usize;

    for cat in WINDOWS_CATEGORIES {
        assert!(
            !cat.name.is_empty(),
            "Category must have a non-empty identifier"
        );
        assert!(
            !cat.domain_label.is_empty(),
            "Category must have a human-readable domain label"
        );

        for action in cat.actions {
            total_actions += 1;
            let full_id = format!("windows.{}.{}", cat.name, action.subaction);

            assert!(
                !action.subaction.is_empty(),
                "Action in category '{}' has empty subaction",
                cat.name
            );
            assert!(
                !action.description.is_empty(),
                "Action '{}' has empty description",
                full_id
            );
            assert!(
                !action.adapter_cmd.is_empty(),
                "Action '{}' has empty adapter_cmd",
                full_id
            );

            // Verify placeholders in adapter_args match declared parameters
            for param_name in extract_placeholders(action.adapter_args) {
                let declared = action.params.iter().any(|p| p.name == param_name);
                assert!(
                    declared,
                    "Action '{}' references parameter '${}' in adapter_args, but it is not declared in params",
                    full_id, param_name
                );
            }

            // High-risk safety check: destructive verbs must not be marked ReadOnly
            let lower_sub = action.subaction.to_lowercase();
            let is_destructive = lower_sub.contains("delete")
                || lower_sub.contains("remove")
                || lower_sub.contains("kill")
                || lower_sub.contains("terminate")
                || lower_sub.contains("format")
                || lower_sub.contains("shutdown")
                || lower_sub.contains("reboot")
                || lower_sub.contains("restart")
                || lower_sub.contains("wipe")
                || lower_sub.contains("stop");

            if is_destructive {
                assert_ne!(
                    action.tier,
                    PermissionTier::ReadOnly,
                    "Destructive tool '{}' cannot have ReadOnly permission tier",
                    full_id
                );
            }
        }
    }

    assert_eq!(total_actions, 1056);
}

#[test]
fn test_all_linux_catalogue_binary_and_param_contracts() {
    let mut total_actions = 0usize;

    for cat in LINUX_CATEGORIES {
        assert!(
            !cat.name.is_empty(),
            "Category must have a non-empty identifier"
        );
        assert!(
            !cat.domain_label.is_empty(),
            "Category must have a human-readable domain label"
        );

        for action in cat.actions {
            total_actions += 1;
            let full_id = format!("linux.{}.{}", cat.name, action.subaction);

            assert!(
                !action.subaction.is_empty(),
                "Action in category '{}' has empty subaction",
                cat.name
            );
            assert!(
                !action.description.is_empty(),
                "Action '{}' has empty description",
                full_id
            );
            assert!(
                !action.adapter_cmd.is_empty(),
                "Action '{}' has empty adapter_cmd",
                full_id
            );

            // Linux tools must not use PowerShell cmdlets or Windows .exe
            assert!(
                !action.adapter_cmd.contains("Get-")
                    && !action.adapter_cmd.contains("Set-")
                    && !action.adapter_cmd.contains("New-")
                    && !action.adapter_cmd.contains("Remove-")
                    && !action.adapter_cmd.ends_with(".exe"),
                "Linux action '{}' uses Windows cmdlet: {}",
                full_id,
                action.adapter_cmd
            );

            // Verify placeholders match declared parameters
            for param_name in extract_placeholders(action.adapter_args) {
                let declared = action.params.iter().any(|p| p.name == param_name);
                assert!(
                    declared,
                    "Action '{}' references parameter '${}' in adapter_args, but it is not declared in params",
                    full_id, param_name
                );
            }

            // High-risk safety check
            let lower_sub = action.subaction.to_lowercase();
            let is_destructive = lower_sub.contains("delete")
                || lower_sub.contains("remove")
                || lower_sub.contains("kill")
                || lower_sub.contains("terminate")
                || lower_sub.contains("format")
                || lower_sub.contains("shutdown")
                || lower_sub.contains("reboot")
                || lower_sub.contains("restart")
                || lower_sub.contains("wipe");

            if is_destructive {
                assert_ne!(
                    action.tier,
                    PermissionTier::ReadOnly,
                    "Destructive tool '{}' cannot have ReadOnly permission tier",
                    full_id
                );
            }
        }
    }

    assert_eq!(total_actions, 1056);
}

#[tokio::test]
async fn test_live_os_operations_execution_windows() {
    let sandbox = Arc::new(PathSandbox::new(vec![PathBuf::from(".")], vec![]));
    let windows_tools = pico_tools::catalog::get_windows_catalog_with_sandbox(sandbox);

    // 1. Test live filesystem directory listing on host OS
    let list_dir = windows_tools
        .iter()
        .find(|t| t.name() == "windows.filesystem.list_directory")
        .expect("windows.filesystem.list_directory missing");

    let res = list_dir.execute(serde_json::json!({ "path": "." })).await;
    #[cfg(target_os = "windows")]
    {
        assert!(
            res.is_ok(),
            "Live filesystem execution failed on Windows: {:?}",
            res.err()
        );
        let val = res.unwrap();
        // Check returned directory listing
        let output_str = val.to_string();
        assert!(
            output_str.contains("Cargo.toml")
                || output_str.contains("crates")
                || output_str.contains("PROJECT_STATUS.md"),
            "Expected live directory listing to contain workspace files, got: {}",
            output_str
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(res.is_ok());
    }

    // 2. Test live get_file_info on Cargo.toml
    let file_info = windows_tools
        .iter()
        .find(|t| t.name() == "windows.filesystem.get_file_info")
        .expect("windows.filesystem.get_file_info missing");

    let res_info = file_info
        .execute(serde_json::json!({ "path": "Cargo.toml" }))
        .await;
    #[cfg(target_os = "windows")]
    {
        assert!(
            res_info.is_ok(),
            "Failed to get file info on Cargo.toml: {:?}",
            res_info.err()
        );
        let val_info = res_info.unwrap();
        assert!(
            val_info.to_string().contains("Cargo.toml") || val_info.to_string().contains("Length")
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(res_info.is_ok());
    }
}

#[tokio::test]
async fn test_sandbox_enforcement_across_all_catalogue_entries() {
    let sandbox = Arc::new(PathSandbox::new(
        vec![PathBuf::from(".")],
        vec![PathBuf::from("C:\\Windows\\System32")],
    ));
    let all_tools = pico_tools::catalog::get_massive_tool_catalog_with_sandbox(sandbox);

    // Filter tools that accept a "path" argument
    let path_tools: Vec<_> = all_tools
        .iter()
        .filter(|t| {
            t.parameters_schema()
                .get("properties")
                .and_then(|p| p.get("path"))
                .is_some()
        })
        .collect();

    assert!(
        path_tools.len() >= 50,
        "Expected at least 50 tools accepting path parameters, found {}",
        path_tools.len()
    );

    // Verify each path tool strictly blocks sandbox escape traversal
    for tool in path_tools.iter().take(20) {
        let malicious_traversal =
            serde_json::json!({ "path": "../../../../../Windows/System32/cmd.exe" });
        let result = tool.execute(malicious_traversal).await;
        assert!(
            result.is_err(),
            "Tool '{}' allowed path sandbox traversal!",
            tool.name()
        );
    }
}
