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
        let canonical_allowed: Vec<PathBuf> = allowed_roots
            .into_iter()
            .map(|p| normalize_existing_path(&p).unwrap_or(p))
            .collect();

        let canonical_blocked: Vec<PathBuf> = blocked_roots
            .into_iter()
            .map(|p| normalize_existing_path(&p).unwrap_or(p))
            .collect();

        Self {
            allowed_roots: canonical_allowed,
            blocked_roots: canonical_blocked,
        }
    }

    /// Verifies whether the specified path is permitted for access.
    pub fn validate_path(&self, target: impl AsRef<Path>) -> Result<PathBuf> {
        let path = target.as_ref();
        let path_str = path.to_string_lossy();

        // 1. Block UNC remote network paths (e.g. \\evil.com\share or //server/share)
        if path_str.starts_with(r"\\") || path_str.starts_with("//") {
            // Check if it's Windows extended length path \\?\
            if !path_str.starts_with(r"\\?\") {
                return Err(PicoError::Security(format!(
                    "UNC network paths are prohibited by sandbox policy: {}",
                    path.display()
                )));
            }
        }

        // 2. Block Windows Drive-Relative paths (e.g. C:foo.txt without backslash)
        if path_str.len() >= 2
            && path_str.as_bytes()[1] == b':'
            && (path_str.len() == 2
                || (path_str.as_bytes()[2] != b'\\' && path_str.as_bytes()[2] != b'/'))
        {
            return Err(PicoError::Security(format!(
                "Drive-relative paths without root slash are prohibited: {}",
                path.display()
            )));
        }

        // 3. Block Alternate Data Streams (ADS) (e.g. file.txt:stream or ::$DATA)
        // A colon is ONLY valid as the drive letter indicator at index 1 in "C:\..."
        for (i, ch) in path_str.char_indices() {
            if ch == ':' {
                let is_drive_letter = i == 1
                    && path_str
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic());
                let is_verbatim_drive = i == 5
                    && path_str.starts_with(r"\\?\")
                    && path_str
                        .chars()
                        .nth(4)
                        .is_some_and(|c| c.is_ascii_alphabetic());
                if !is_drive_letter && !is_verbatim_drive {
                    return Err(PicoError::Security(format!(
                        "Alternate Data Stream (ADS) syntax ':' detected in path: {}",
                        path.display()
                    )));
                }
            }
        }

        // 4. Block DOS Legacy Device Names (CON, PRN, AUX, NUL, COM1..9, LPT1..9)
        for component in path.components() {
            if let std::path::Component::Normal(comp_os) = component {
                let comp_str = comp_os.to_string_lossy();
                if is_dos_device_name(&comp_str) {
                    return Err(PicoError::Security(format!(
                        "Reserved Windows DOS device name '{}' is prohibited: {}",
                        comp_str,
                        path.display()
                    )));
                }
            }
        }

        // 5. Block Directory Traversal markers
        if path_str.contains("../") || path_str.contains("..\\") || path_str == ".." {
            return Err(PicoError::Security(format!(
                "Directory traversal detected in path: {}",
                path.display()
            )));
        }

        // 6. Resolve absolute path & canonicalize deepest existing ancestor
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            cwd.join(path)
        };

        let mut curr = absolute_path.as_path();
        let mut non_existing_suffixes = Vec::new();
        while !curr.exists() {
            if let Some(file_name) = curr.file_name() {
                non_existing_suffixes.push(file_name);
            }
            if let Some(parent) = curr.parent() {
                curr = parent;
            } else {
                break;
            }
        }

        let canonical_base = if curr.exists() {
            normalize_existing_path(curr).map_err(|e| {
                PicoError::Security(format!(
                    "Failed to canonicalize path {}: {}",
                    curr.display(),
                    e
                ))
            })?
        } else {
            curr.to_path_buf()
        };

        let mut normalized = canonical_base;
        for suffix in non_existing_suffixes.into_iter().rev() {
            normalized.push(suffix);
        }

        // 7. Check against forbidden / critical system roots
        for blocked in &self.blocked_roots {
            if path_is_within(&normalized, blocked) {
                return Err(PicoError::Security(format!(
                    "Access to sensitive system path '{}' is forbidden by security policy",
                    normalized.display()
                )));
            }
        }

        // 8. Forbid sensitive user credential folders
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

        // 9. If allowed_roots is empty, default to current workspace
        if self.allowed_roots.is_empty() {
            return Ok(normalized);
        }

        // 10. Ensure path lies within at least one allowed root
        let is_allowed = self
            .allowed_roots
            .iter()
            .any(|root| path_is_within(&normalized, root));

        if !is_allowed {
            return Err(PicoError::Security(format!(
                "Path '{}' is outside the permitted sandbox boundaries",
                normalized.display()
            )));
        }

        Ok(normalized)
    }
}

/// Helper to canonicalize and strip Windows extended-length prefix \\?\
fn normalize_existing_path(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)?;
    let p_str = canonical.to_string_lossy();
    if let Some(stripped) = p_str.strip_prefix(r"\\?\") {
        Ok(PathBuf::from(stripped))
    } else {
        Ok(canonical)
    }
}

/// Platform-aware path prefix comparison.
/// Windows: case-insensitive with backslash normalization.
/// Linux/Unix: case-sensitive with forward-slash normalization.
fn path_is_within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let p_str = path
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('/', "\\");
        let r_str = root
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('/', "\\");
        let r_clean = r_str.trim_end_matches('\\');
        p_str == r_clean || p_str.starts_with(&format!("{}\\", r_clean))
    }
    #[cfg(not(windows))]
    {
        let p_str = path.to_string_lossy().replace('\\', "/");
        let r_str = root.to_string_lossy().replace('\\', "/");
        let r_clean = r_str.trim_end_matches('/');
        p_str == r_clean || p_str.starts_with(&format!("{}/", r_clean))
    }
}

/// Checks whether a filename stem is a reserved Windows legacy device name
fn is_dos_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dos_device_names_blocked() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        assert!(sandbox.validate_path("CON").is_err());
        assert!(sandbox.validate_path("aux.txt").is_err());
        assert!(sandbox.validate_path("NUL.json").is_err());
        assert!(sandbox.validate_path("com1").is_err());
    }

    #[test]
    fn test_alternate_data_streams_blocked() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        assert!(sandbox.validate_path("file.txt:stream").is_err());
        assert!(sandbox.validate_path("secret.doc::$DATA").is_err());
    }

    #[test]
    fn test_unc_paths_blocked() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        assert!(sandbox.validate_path(r"\\evil.com\share\file").is_err());
        assert!(sandbox.validate_path("//remote/data").is_err());
    }

    #[test]
    fn test_drive_relative_paths_blocked() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        assert!(sandbox.validate_path("C:relative_file.txt").is_err());
    }

    #[test]
    fn test_non_existent_nested_path_in_allowed_root() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        let non_existent = PathBuf::from("./target/test_sandbox_dummy/nested/new_file.txt");
        let validated = sandbox.validate_path(&non_existent);
        assert!(
            validated.is_ok(),
            "Validation failed: {:?}",
            validated.err()
        );
    }

    #[test]
    fn test_path_is_within_platform_rules() {
        #[cfg(windows)]
        {
            assert!(path_is_within(
                Path::new("C:\\Users\\Test\\Data\\file.txt"),
                Path::new("c:/users/test")
            ));
            assert!(!path_is_within(
                Path::new("C:\\Windows\\System32\\cmd.exe"),
                Path::new("C:\\Users\\Test")
            ));
        }
        #[cfg(not(windows))]
        {
            assert!(path_is_within(
                Path::new("/home/user/project/file.txt"),
                Path::new("/home/user/project")
            ));
            assert!(!path_is_within(
                Path::new("/etc/shadow"),
                Path::new("/home/user/project")
            ));
        }
    }
}
