use pico_core::error::{PicoError, Result};
use std::path::{Component, Path, PathBuf};

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
            .filter_map(|p| {
                if p.as_os_str().is_empty() {
                    return None;
                }
                if let Ok(resolved) = normalize_existing_path(&p) {
                    Some(resolved)
                } else if p.is_relative() {
                    std::env::current_dir().ok().map(|cwd| cwd.join(p))
                } else {
                    Some(p)
                }
            })
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

    /// Verifies whether the specified path is permitted for access (fail-closed).
    pub fn validate_path(&self, target: impl AsRef<Path>) -> Result<PathBuf> {
        let path = target.as_ref();
        let path_str = path.to_string_lossy();

        if path_str.trim().is_empty() || path_str.contains('\0') {
            return Err(PicoError::Security(
                "Target path cannot be empty or contain NUL bytes".into(),
            ));
        }

        // 0. Fail closed when sandbox has no permitted roots configured.
        if self.allowed_roots.is_empty() {
            return Err(PicoError::Security(
                "Sandbox denied access: no allowed directories configured".into(),
            ));
        }

        // 1. Block UNC remote network paths & extended namespace escapes (\\evil\share, //server/share, \\?\...)
        if path_str.starts_with(r"\\") || path_str.starts_with("//") {
            return Err(PicoError::Security(format!(
                "Remote UNC or raw device paths are forbidden: '{}'",
                path_str
            )));
        }

        // 2. Reject any explicit parent-directory traversal component anywhere in the path
        // (catches "../", "/..", "\..", "foo/..\bar", and bare "..").
        if path.components().any(|c| matches!(c, Component::ParentDir))
            || path_str.contains("..") && contains_parent_segment(&path_str)
        {
            return Err(PicoError::Security(format!(
                "Directory traversal ('..') is forbidden in sandboxed paths: '{}'",
                path_str
            )));
        }

        // 3. Block Windows drive-relative paths (e.g., "C:foo.txt") and NTFS Alternate Data Streams (file.txt:stream).
        #[cfg(windows)]
        {
            let bytes = path_str.as_bytes();
            if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                let third = bytes.get(2).copied();
                if third != Some(b'\\') && third != Some(b'/') {
                    return Err(PicoError::Security(format!(
                        "Drive-relative path '{}' is forbidden",
                        path_str
                    )));
                }
            }
            // Check colons outside the single "X:" drive prefix.
            let after_drive =
                if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                    &path_str[2..]
                } else {
                    &path_str
                };
            if after_drive.contains(':') {
                return Err(PicoError::Security(format!(
                    "NTFS Alternate Data Stream / colon syntax is forbidden in path: '{}'",
                    path_str
                )));
            }
        }
        #[cfg(not(windows))]
        {
            if path_str.contains(':') {
                return Err(PicoError::Security(format!(
                    "Colon stream syntax is forbidden in path: '{}'",
                    path_str
                )));
            }
        }

        // 4. Reject Windows legacy DOS reserved device filenames (CON, PRN, AUX, NUL, COM1-9, LPT1-9).
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if is_dos_device_name(file_name) {
                return Err(PicoError::Security(format!(
                    "Reserved OS device filename '{}' is forbidden",
                    file_name
                )));
            }
        }

        // 5. Reject sensitive credential / secret / OS identity files by normalized path fragment.
        let lower_norm = path_str.replace('\\', "/").to_lowercase();
        let sensitive_fragments = [
            "/.ssh/",
            "/.ssh",
            ".ssh/",
            "/.gnupg/",
            ".gnupg/",
            "/.aws/",
            ".aws/",
            "/.kube/",
            ".kube/",
            "/.docker/",
            ".docker/",
            "/.netrc",
            ".netrc",
            "/.npmrc",
            ".npmrc",
            "/.git-credentials",
            ".git-credentials",
            "credentials.json",
            "/.env",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            "id_dsa",
            "authorized_keys",
            "known_hosts",
            "login data",
            "web data",
            "/cookies",
            "key3.db",
            "key4.db",
            "logins.json",
            "ntds.dit",
            "system32/config/sam",
            "system32/config/system",
            "system32/config/security",
            "/etc/shadow",
            "/etc/gshadow",
            "/etc/sudoers",
            "/etc/passwd",
            ".password-store",
        ];
        if lower_norm == ".env" || lower_norm.ends_with("/.env") {
            return Err(PicoError::Security(format!(
                "Access to sensitive file '{}' is blocked by sandbox policy",
                path_str
            )));
        }
        for frag in sensitive_fragments {
            if lower_norm.contains(frag) {
                return Err(PicoError::Security(format!(
                    "Access to sensitive location ('{}') is blocked by sandbox policy",
                    frag
                )));
            }
        }

        // 6. Anchor relative paths cleanly onto CWD (or first allowed root if CWD unavailable).
        let abs_candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            let cwd = std::env::current_dir().map_err(|e| {
                PicoError::Security(format!("Cannot determine current directory: {}", e))
            })?;
            cwd.join(path)
        };

        // 7. Reject symlinks / reparse points along existing prefixes & leaf to block dangling-symlink writes.
        reject_symlink_chain(&abs_candidate)?;

        // 8. Canonicalize existing path OR nearest existing ancestor cleanly.
        let resolved = resolve_with_existing_ancestor(&abs_candidate)?;

        let resolved_lower = resolved.to_string_lossy().replace('\\', "/").to_lowercase();
        for frag in sensitive_fragments {
            if resolved_lower.contains(frag) {
                return Err(PicoError::Security(format!(
                    "Resolved path contains sensitive fragment '{}'",
                    frag
                )));
            }
        }

        // 9. Enforce explicit blocklist first.
        for blocked in &self.blocked_roots {
            if path_is_within(&resolved, blocked) {
                return Err(PicoError::Security(format!(
                    "Access to blocked directory '{}' is forbidden",
                    resolved.display()
                )));
            }
        }

        // 10. Enforce strict allowlist containment.
        let allowed = self
            .allowed_roots
            .iter()
            .any(|root| path_is_within(&resolved, root));

        if !allowed {
            return Err(PicoError::Security(format!(
                "Path '{}' is outside all permitted sandbox directories",
                resolved.display()
            )));
        }

        Ok(resolved)
    }
}

fn contains_parent_segment(s: &str) -> bool {
    s.split(['/', '\\']).any(|seg| seg == "..")
}

/// Checks every prefix of `candidate` using `symlink_metadata` (does not follow links)
/// so neither an existing symlink nor a broken/dangling symlink can divert writes outside sandbox.
fn reject_symlink_chain(candidate: &Path) -> Result<()> {
    let mut cur = PathBuf::new();
    for comp in candidate.components() {
        cur.push(comp);
        if let Ok(meta) = std::fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                return Err(PicoError::Security(format!(
                    "Symbolic links/junctions are forbidden inside sandbox paths: '{}'",
                    cur.display()
                )));
            }
        }
    }
    Ok(())
}

/// Resolves an absolute path by canonicalizing its deepest existing ancestor and
/// appending non-existent clean segments.
fn resolve_with_existing_ancestor(abs_candidate: &Path) -> Result<PathBuf> {
    if abs_candidate.exists() {
        return normalize_existing_path(abs_candidate).map_err(|e| {
            PicoError::Security(format!(
                "Failed to canonicalize path '{}': {}",
                abs_candidate.display(),
                e
            ))
        });
    }

    let mut tail_segments: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = abs_candidate;
    while !cursor.exists() {
        if let Some(name) = cursor.file_name() {
            tail_segments.push(name.to_os_string());
        } else {
            return Err(PicoError::Security(format!(
                "Path '{}' has no existing root ancestor",
                abs_candidate.display()
            )));
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => {
                return Err(PicoError::Security(format!(
                    "Path '{}' has no valid parent directory",
                    abs_candidate.display()
                )));
            }
        }
    }

    let mut base = normalize_existing_path(cursor).map_err(|e| {
        PicoError::Security(format!(
            "Failed to canonicalize ancestor '{}': {}",
            cursor.display(),
            e
        ))
    })?;

    for seg in tail_segments.into_iter().rev() {
        if seg == "." || seg == ".." {
            return Err(PicoError::Security(
                "Invalid traversal segment in path tail".into(),
            ));
        }
        base.push(seg);
    }

    Ok(base)
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
    fn test_empty_allowed_roots_fails_closed() {
        let sandbox = PathSandbox::new(vec![], vec![]);
        assert!(sandbox.validate_path("Cargo.toml").is_err());
        assert!(sandbox.validate_path("notes.txt").is_err());
    }

    #[test]
    fn test_trailing_parent_dir_and_sensitive_fragments_blocked() {
        let sandbox = PathSandbox::new(vec![PathBuf::from(".")], vec![]);
        assert!(sandbox.validate_path("subdir/..").is_err());
        assert!(sandbox.validate_path("subdir\\..").is_err());
        assert!(sandbox.validate_path("..").is_err());
        assert!(sandbox.validate_path(".kube/config").is_err());
        assert!(sandbox.validate_path(".docker/config.json").is_err());
        assert!(sandbox
            .validate_path("AppData/Local/Google/Login Data")
            .is_err());
        assert!(sandbox.validate_path("configs/.env").is_err());
        assert!(sandbox.validate_path(".git-credentials").is_err());
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
