use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_security::PathSandbox;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_READ_LINES: usize = 2000;
const MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SEARCH_VISITED: usize = 5000;
const MAX_SEARCH_MATCHES: usize = 50;

/// Tool to read file contents within sandbox boundaries with streaming byte/line caps.
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
                    "description": "Maximum number of lines to read (default 500, max 2000)",
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

        let max_lines = (arguments
            .get("max_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as usize)
            .clamp(1, MAX_READ_LINES);

        let safe_path = self.sandbox.validate_path(path_str)?;
        if std::fs::symlink_metadata(&safe_path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(PicoError::Security(format!(
                "Refusing to read symbolic link '{}'",
                safe_path.display()
            )));
        }

        let file = std::fs::File::open(&safe_path).map_err(|e| PicoError::ToolExecution {
            tool: self.name().into(),
            message: format!("Failed to open file {}: {}", safe_path.display(), e),
        })?;
        let mut reader = BufReader::new(file);
        let mut collected_lines = Vec::new();
        let mut bytes_read = 0usize;
        let mut truncated = false;
        let mut line_buf = String::new();

        loop {
            if collected_lines.len() >= max_lines {
                // Probe if additional data exists beyond limit.
                line_buf.clear();
                if reader.read_line(&mut line_buf).unwrap_or(0) > 0 {
                    truncated = true;
                }
                break;
            }
            line_buf.clear();
            let n = reader
                .read_line(&mut line_buf)
                .map_err(|e| PicoError::ToolExecution {
                    tool: self.name().into(),
                    message: format!("Failed reading file {}: {}", safe_path.display(), e),
                })?;
            if n == 0 {
                break;
            }
            if bytes_read + n > MAX_READ_BYTES {
                truncated = true;
                break;
            }
            bytes_read += n;
            let trimmed_line = line_buf.trim_end_matches(['\r', '\n']).to_string();
            collected_lines.push(trimmed_line);
        }

        Ok(json!({
            "path": safe_path.display().to_string(),
            "content": collected_lines.join("\n"),
            "lines_read": collected_lines.len(),
            "bytes_read": bytes_read,
            "truncated": truncated
        }))
    }
}

/// Tool to write file contents within sandbox boundaries with size and symlink TOCTOU guards.
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

        if content.len() > MAX_WRITE_BYTES {
            return Err(PicoError::Security(format!(
                "Write payload ({} bytes) exceeds 2 MB safety limit",
                content.len()
            )));
        }

        let safe_path = self.sandbox.validate_path(path_str)?;

        if let Some(parent) = safe_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed to create parent directory: {}", e),
            })?;
        }

        // Re-validate after parent directory creation to prevent symlink swap races.
        let verified_path = self.sandbox.validate_path(&safe_path)?;
        if std::fs::symlink_metadata(&verified_path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(PicoError::Security(format!(
                "Refusing to overwrite symbolic link '{}'",
                verified_path.display()
            )));
        }

        std::fs::write(&verified_path, content).map_err(|e| PicoError::ToolExecution {
            tool: self.name().into(),
            message: format!("Failed to write file {}: {}", verified_path.display(), e),
        })?;

        Ok(json!({
            "path": verified_path.display().to_string(),
            "bytes_written": content.len(),
            "status": "success"
        }))
    }
}

/// Tool to search files matching pattern safely without traversing symlinks/junctions.
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
        let mut visited = 0usize;
        let pattern_lower = pattern.to_lowercase();

        fn walk(
            sandbox: &PathSandbox,
            dir: &Path,
            pattern: &str,
            matches: &mut Vec<String>,
            visited: &mut usize,
            depth: usize,
        ) {
            if depth > 5 || matches.len() >= MAX_SEARCH_MATCHES || *visited >= MAX_SEARCH_VISITED {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if *visited >= MAX_SEARCH_VISITED || matches.len() >= MAX_SEARCH_MATCHES {
                        break;
                    }
                    *visited += 1;
                    // Check lstat/reparse attributes before ever following directories.
                    let Ok(ft) = entry.file_type() else {
                        continue;
                    };
                    if ft.is_symlink() {
                        continue;
                    }
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name == "target" || name == ".git" || name == "node_modules" {
                        continue;
                    }
                    if name.to_lowercase().contains(pattern) {
                        if let Ok(valid) = sandbox.validate_path(&path) {
                            matches.push(valid.display().to_string());
                        }
                    }
                    if ft.is_dir() && sandbox.validate_path(&path).is_ok() {
                        walk(sandbox, &path, pattern, matches, visited, depth + 1);
                    }
                }
            }
        }

        walk(
            &self.sandbox,
            &safe_dir,
            &pattern_lower,
            &mut matches,
            &mut visited,
            0,
        );

        Ok(json!({
            "pattern": pattern,
            "root": safe_dir.display().to_string(),
            "matches": matches,
            "count": matches.len(),
            "visited_entries": visited
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_file_streaming_respects_max_lines_and_blocks_sensitive() {
        let root =
            std::env::temp_dir().join(format!("pico_fs_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).unwrap();
        let big = root.join("large.txt");
        let lines: String = (0..100).map(|i| format!("line-{}\n", i)).collect();
        std::fs::write(&big, lines).unwrap();

        let sandbox = Arc::new(PathSandbox::new(vec![root.clone()], vec![]));
        let reader = ReadFileTool::new(sandbox.clone());
        let res = reader
            .execute(json!({ "path": big.to_str().unwrap(), "max_lines": 5 }))
            .await
            .unwrap();
        assert_eq!(res["lines_read"], 5);
        assert_eq!(res["truncated"], true);

        let env_path = root.join(".env");
        assert!(reader
            .execute(json!({ "path": env_path.to_str().unwrap() }))
            .await
            .is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}
