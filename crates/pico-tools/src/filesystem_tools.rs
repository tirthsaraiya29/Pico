use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_security::PathSandbox;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

/// Tool to read file contents within sandbox boundaries.
pub struct ReadFileTool {
    sandbox: Arc<PathSandbox>,
}

impl ReadFileTool {
    pub fn new(sandbox: Arc<PathSandbox>) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Reads textual contents of a specified file within allowed workspace directories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (default 500)",
                    "minimum": 1
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let path_str = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'path' argument".into(),
            })?;

        let max_lines = arguments
            .get("max_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as usize;

        let safe_path = self.sandbox.validate_path(path_str)?;

        let content =
            std::fs::read_to_string(&safe_path).map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed to read file {}: {}", safe_path.display(), e),
            })?;

        let lines: Vec<&str> = content.lines().take(max_lines).collect();
        let total_lines = content.lines().count();
        let truncated = total_lines > max_lines;

        Ok(json!({
            "path": safe_path.display().to_string(),
            "content": lines.join("\n"),
            "lines_read": lines.len(),
            "total_lines": total_lines,
            "truncated": truncated
        }))
    }
}

/// Tool to write or append file contents within sandbox boundaries.
pub struct WriteFileTool {
    sandbox: Arc<PathSandbox>,
}

impl WriteFileTool {
    pub fn new(sandbox: Arc<PathSandbox>) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Writes content to a file in an allowed directory. Requires user confirmation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to create or overwrite"
                },
                "content": {
                    "type": "string",
                    "description": "Textual content to write"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ConfirmationRequired
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let path_str = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'path' argument".into(),
            })?;

        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'content' argument".into(),
            })?;

        let safe_path = self.sandbox.validate_path(path_str)?;

        if let Some(parent) = safe_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed to create parent directory: {}", e),
            })?;
        }

        std::fs::write(&safe_path, content).map_err(|e| PicoError::ToolExecution {
            tool: self.name().into(),
            message: format!("Failed to write file {}: {}", safe_path.display(), e),
        })?;

        Ok(json!({
            "path": safe_path.display().to_string(),
            "bytes_written": content.len(),
            "status": "success"
        }))
    }
}

/// Tool to search files matching pattern.
pub struct SearchFilesTool {
    sandbox: Arc<PathSandbox>,
}

impl SearchFilesTool {
    pub fn new(sandbox: Arc<PathSandbox>) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Searches for files matching a pattern in allowed directories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "File substring or extension to match (e.g. '.rs', 'config')"
                },
                "directory": {
                    "type": "string",
                    "description": "Directory to search within (defaults to current workspace)"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let pattern = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'pattern' argument".into(),
            })?;

        let dir_str = arguments
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let safe_dir = self.sandbox.validate_path(dir_str)?;

        let mut matches = Vec::new();
        let pattern_lower = pattern.to_lowercase();

        fn walk(dir: &Path, pattern: &str, matches: &mut Vec<String>, depth: usize) {
            if depth > 5 || matches.len() >= 50 {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name == "target" || name == ".git" || name == "node_modules" {
                        continue;
                    }
                    if name.to_lowercase().contains(pattern) {
                        matches.push(path.display().to_string());
                    }
                    if path.is_dir() {
                        walk(&path, pattern, matches, depth + 1);
                    }
                }
            }
        }

        walk(&safe_dir, &pattern_lower, &mut matches, 0);

        Ok(json!({
            "pattern": pattern,
            "root": safe_dir.display().to_string(),
            "matches": matches,
            "count": matches.len()
        }))
    }
}
