use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Serialize)]
pub struct AuditRecord {
    pub timestamp: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub permission_tier: String,
    pub arguments_masked: serde_json::Value,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct AuditEntry<'a> {
    pub session_id: Option<&'a str>,
    pub tool_name: &'a str,
    pub permission_tier: &'a str,
    pub arguments: &'a serde_json::Value,
    pub status: &'a str,
    pub duration_ms: Option<u64>,
    pub message: Option<&'a str>,
}

/// Thread-safe append-only security audit log manager with recursive key & token redaction.
pub struct AuditLogger {
    log_file: PathBuf,
    lock: Mutex<()>,
}

impl AuditLogger {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let log_file = path.as_ref().to_path_buf();
        Self {
            log_file,
            lock: Mutex::new(()),
        }
    }

    /// Records an execution attempt to the audit log, creating parent directories and masking secrets.
    pub fn log_attempt(
        &self,
        tool_name: &str,
        tier: &str,
        arguments: &serde_json::Value,
        status: &str,
        duration_ms: Option<u64>,
        message: Option<&str>,
    ) {
        self.log_entry(AuditEntry {
            session_id: None,
            tool_name,
            permission_tier: tier,
            arguments,
            status,
            duration_ms,
            message,
        });
    }

    /// Records a structured execution attempt with optional session scoping.
    pub fn log_entry(&self, entry: AuditEntry<'_>) {
        let masked = Self::mask_sensitive_data(entry.arguments);
        let masked_msg = entry.message.map(Self::redact_string);
        let record = AuditRecord {
            timestamp: Utc::now().to_rfc3339(),
            session_id: entry.session_id.map(|s| s.to_string()),
            tool_name: entry.tool_name.to_string(),
            permission_tier: entry.permission_tier.to_string(),
            arguments_masked: masked,
            status: entry.status.to_string(),
            duration_ms: entry.duration_ms,
            message: masked_msg,
        };

        if let Ok(json_line) = serde_json::to_string(&record) {
            let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(parent) = self.log_file.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }

            let mut opts = OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }

            if let Ok(mut file) = opts.open(&self.log_file) {
                let _ = writeln!(file, "{}", json_line);
            }
        }
    }

    /// Masks sensitive object keys AND credential/token patterns embedded inside string values.
    pub fn mask_sensitive_data(val: &serde_json::Value) -> serde_json::Value {
        match val {
            serde_json::Value::Object(map) => {
                let mut sanitized = serde_json::Map::new();
                for (k, v) in map {
                    let k_lower = k.to_lowercase();
                    if k_lower.contains("key")
                        || k_lower.contains("secret")
                        || k_lower.contains("password")
                        || k_lower.contains("passwd")
                        || k_lower.contains("token")
                        || k_lower.contains("auth")
                        || k_lower.contains("credential")
                        || k_lower.contains("private")
                        || k_lower.contains("cookie")
                        || k_lower.contains("bearer")
                    {
                        sanitized.insert(
                            k.clone(),
                            serde_json::Value::String("***REDACTED***".into()),
                        );
                    } else {
                        sanitized.insert(k.clone(), Self::mask_sensitive_data(v));
                    }
                }
                serde_json::Value::Object(sanitized)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::mask_sensitive_data).collect())
            }
            serde_json::Value::String(s) => serde_json::Value::String(Self::redact_string(s)),
            other => other.clone(),
        }
    }

    /// Redacts bearer headers, common API tokens (`ghp_`, `github_pat_`, `sk-`, `AIza`),
    /// PEM private key blocks, and embedded URL credentials (`https://user:pass@host`).
    pub fn redact_string(input: &str) -> String {
        static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
        static URL_CRED_RE: OnceLock<Regex> = OnceLock::new();
        static PEM_RE: OnceLock<Regex> = OnceLock::new();

        let step1 = PEM_RE
            .get_or_init(|| {
                Regex::new(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----").unwrap()
            })
            .replace_all(input, "***REDACTED_PRIVATE_KEY***");

        let step2 = URL_CRED_RE
            .get_or_init(|| Regex::new(r"(https?://)[^/\s:@]+:[^/\s@]+@").unwrap())
            .replace_all(&step1, "${1}***:***@");

        TOKEN_RE
            .get_or_init(|| {
                Regex::new(r"(?i)\b(bearer\s+[A-Za-z0-9._\-~+/]+=*|gh[pousr]_[A-Za-z0-9_]{16,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9\-_]{16,}|AIza[0-9A-Za-z\-_]{20,})\b").unwrap()
            })
            .replace_all(&step2, "***REDACTED_TOKEN***")
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_creates_nested_directory_and_redacts_nested_secrets() {
        let tmp = std::env::temp_dir().join(format!(
            "pico_audit_test_{}/nested/audit.jsonl",
            uuid::Uuid::new_v4().simple()
        ));
        let logger = AuditLogger::new(&tmp);
        let payload = serde_json::json!({
            "url": "https://alice:hunter2@api.example.com/v1",
            "header": "Bearer super_secret_jwt_12345",
            "api_key": "raw-secret",
            "gh_token": "ghp_1234567890abcdefghijklmnop",
            "command_note": "push with sk-testSecretValue12345678"
        });
        logger.log_entry(AuditEntry {
            session_id: Some("sess-42"),
            tool_name: "fetch_web_content",
            permission_tier: "ReadOnly",
            arguments: &payload,
            status: "success",
            duration_ms: Some(12),
            message: Some("Connected using Authorization: Bearer abcdefghij_secret999"),
        });

        let contents = std::fs::read_to_string(&tmp).expect("Audit log file should be created");
        assert!(!contents.contains("hunter2"));
        assert!(!contents.contains("super_secret_jwt_12345"));
        assert!(!contents.contains("raw-secret"));
        assert!(!contents.contains("ghp_1234567890"));
        assert!(!contents.contains("sk-testSecretValue12345678"));
        assert!(!contents.contains("abcdefghij_secret999"));
        assert!(contents.contains("sess-42"));

        if let Some(dir) = tmp.parent().and_then(|p| p.parent()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
