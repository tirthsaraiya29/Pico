use chrono::Utc;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

/// Thread-safe append-only security audit log manager.
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

    /// Records an execution attempt to the audit log.
    pub fn log_attempt(
        &self,
        tool_name: &str,
        tier: &str,
        arguments: &serde_json::Value,
        status: &str,
        duration_ms: Option<u64>,
        message: Option<&str>,
    ) {
        let masked = self.mask_sensitive_data(arguments);
        let record = AuditRecord {
            timestamp: Utc::now().to_rfc3339(),
            session_id: None,
            tool_name: tool_name.to_string(),
            permission_tier: tier.to_string(),
            arguments_masked: masked,
            status: status.to_string(),
            duration_ms,
            message: message.map(|s| s.to_string()),
        };

        if let Ok(json_line) = serde_json::to_string(&record) {
            let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_file)
            {
                let _ = writeln!(file, "{}", json_line);
            }
        }
    }

    /// Masks passwords, keys, tokens, and authorization fields in JSON values.
    fn mask_sensitive_data(&self, val: &serde_json::Value) -> serde_json::Value {
        match val {
            serde_json::Value::Object(map) => {
                let mut sanitized = serde_json::Map::new();
                for (k, v) in map {
                    let k_lower = k.to_lowercase();
                    if k_lower.contains("key")
                        || k_lower.contains("secret")
                        || k_lower.contains("password")
                        || k_lower.contains("token")
                        || k_lower.contains("auth")
                    {
                        sanitized.insert(
                            k.clone(),
                            serde_json::Value::String("***REDACTED***".into()),
                        );
                    } else {
                        sanitized.insert(k.clone(), self.mask_sensitive_data(v));
                    }
                }
                serde_json::Value::Object(sanitized)
            }
            serde_json::Value::Array(arr) => {
                let sanitized: Vec<serde_json::Value> = arr
                    .iter()
                    .map(|item| self.mask_sensitive_data(item))
                    .collect();
                serde_json::Value::Array(sanitized)
            }
            other => other.clone(),
        }
    }
}
