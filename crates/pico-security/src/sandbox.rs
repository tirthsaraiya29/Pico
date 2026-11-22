use pico_core::error::{PicoError, Result};
use std::path::{Path, PathBuf};

/// Sandboxed file system path boundary enforcer.
#[derive(Debug, Clone)]
pub struct PathSandbox {
    allowed_roots: Vec<PathBuf>,
    blocked_roots: Vec<PathBuf>,
}

impl PathSandbox {
    pub fn new(allowed_roots: Vec<PathBuf>, blocked_roots: Vec<PathBuf>) -> Self {
        // Canonicalize paths where possible
        let canonical_allowed: Vec<PathBuf> = allowed_roots
            .into_iter()
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .collect();

        let canonical_blocked: Vec<PathBuf> = blocked_roots
            .into_iter()
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .collect();

        Self {
            allowed_roots: canonical_allowed,
            blocked_roots: canonical_blocked,
        }
    }

    /// Verifies whether the specified path is permitted for access.
    pub fn validate_path(&self, target: impl AsRef<Path>) -> Result<PathBuf> {
        let path = target.as_ref();

        // Check for basic path traversal markers
        let path_str = path.to_string_lossy();
        if path_str.contains("../") || path_str.contains("..\\") {
            return Err(PicoError::Security(format!(
                "Directory traversal detected in path: {}",
                path.display()
            )));
        }

        // Canonicalize if exists, or normalize parent
        let normalized = if path.exists() {
            std::fs::canonicalize(path).map_err(|e| {
                PicoError::Security(format!(
                    "Failed to canonicalize path {}: {}",
                    path.display(),
                    e
                ))
            })?
        } else if let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            } else if parent.exists() {
                let canonical_parent = std::fs::canonicalize(parent).map_err(|e| {
                    PicoError::Security(format!(
                        "Failed to canonicalize parent {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
                canonical_parent.join(path.file_name().unwrap_or_default())
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        };

        // Check against forbidden / critical roots
        for blocked in &self.blocked_roots {
            if normalized.starts_with(blocked) {
                return Err(PicoError::Security(format!(
                    "Access to sensitive system path '{}' is forbidden by security policy",
                    normalized.display()
                )));
            }
        }

        // Forbid sensitive user credential folders
        let normalized_str = normalized.to_string_lossy().to_lowercase();
        let sensitive_fragments = [
            ".ssh",
            ".aws",
            ".gnupg",
            "id_rsa",
            "id_ed25519",
            "credentials",
            "system32",
            "systemroot",
        ];

        for fragment in sensitive_fragments {
            if normalized_str.contains(fragment) {
                return Err(PicoError::Security(format!(
                    "Path '{}' contains protected credential or system target '{}'",
                    normalized.display(),
                    fragment
                )));
            }
        }

        // If allowed_roots is empty, default to current workspace
        if self.allowed_roots.is_empty() {
            return Ok(normalized);
        }

        // Ensure path lies within at least one allowed root
        let is_allowed = self
            .allowed_roots
            .iter()
            .any(|root| normalized.starts_with(root));

        if !is_allowed {
            return Err(PicoError::Security(format!(
                "Path '{}' is outside the permitted sandbox boundaries",
                normalized.display()
            )));
        }

        Ok(normalized)
    }
}
