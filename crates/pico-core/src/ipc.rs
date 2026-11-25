use crate::error::{PicoError, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Current wire protocol version for Pico local IPC daemon communications.
pub const IPC_PROTOCOL_VERSION: u32 = 1;

/// Maximum bounded JSON line size accepted over local IPC (64 KB).
pub const MAX_IPC_FRAME_BYTES: usize = 65_536;

/// Live assistant status snapshot exposed to desktop companions & CLI clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonSnapshot {
    pub protocol: u32,
    pub pid: u32,
    pub status: String,
    pub voice_paused: bool,
    pub is_playing_speech: bool,
    pub stt_engine: String,
    pub tts_engine: String,
    pub registered_tools: usize,
    pub active_session_id: String,
    pub pending_confirmation: Option<PendingConfirmationView>,
    pub pending_clarification: Option<ClarificationView>,
    pub last_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingConfirmationView {
    pub request_id: String,
    pub tool_name: String,
    pub arguments_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClarificationView {
    pub question: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum IpcRequest {
    Ping,
    GetSnapshot,
    SubmitPrompt {
        session_id: Option<String>,
        text: String,
        speak_response: bool,
    },
    RespondConfirmation {
        request_id: String,
        approved: bool,
    },
    SetVoicePaused {
        paused: bool,
    },
    InterruptSpeech,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcEnvelope {
    pub protocol: u32,
    #[serde(default)]
    pub token: Option<String>,
    pub request: IpcRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum IpcResponse {
    Pong { protocol: u32, pid: u32 },
    Snapshot(Box<DaemonSnapshot>),
    TurnResult { session_id: String, reply: String },
    Ack { ok: bool },
    Error { code: String, message: String },
}

/// Computes the deterministic per-user OS local IPC endpoint path/name.
pub fn default_ipc_endpoint() -> String {
    #[cfg(target_os = "windows")]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let clean: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!(r"\\.\pipe\pico-assistant-v1-{}", clean)
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            let trimmed = xdg.trim();
            if !trimmed.is_empty() {
                return format!("{}/pico-assistant-v1.sock", trimmed.trim_end_matches('/'));
            }
        }
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let clean: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("/tmp/pico-assistant-v1-{}.sock", clean)
    }
}

/// Returns the user-private path for the ephemeral per-daemon IPC shared secret token.
pub fn default_ipc_token_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data)
                .join("Pico")
                .join("pico-assistant-v1.token");
        }
        PathBuf::from("data/pico-assistant-v1.token")
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            let trimmed = xdg.trim();
            if !trimmed.is_empty() {
                return Path::new(trimmed).join("pico-assistant-v1.token");
            }
        }
        PathBuf::from("data/pico-assistant-v1.token")
    }
}

/// Reads the IPC secret token from disk if present.
pub fn read_ipc_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Serializes any IPC frame as newline-delimited JSON and enforces the 64 KB frame cap.
pub fn encode_ipc_line<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|e| PicoError::Serialization(format!("IPC serialize failed: {}", e)))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_IPC_FRAME_BYTES {
        return Err(PicoError::Security(format!(
            "IPC frame exceeds max size ({} > {} bytes)",
            bytes.len(),
            MAX_IPC_FRAME_BYTES
        )));
    }
    Ok(bytes)
}

/// Parses and validates an incoming IPC request line (protocol version & frame limit).
pub fn decode_ipc_envelope(line: &str) -> Result<IpcEnvelope> {
    if line.len() > MAX_IPC_FRAME_BYTES {
        return Err(PicoError::Security(format!(
            "Incoming IPC frame exceeded max size ({} bytes)",
            line.len()
        )));
    }
    let env: IpcEnvelope = serde_json::from_str(line.trim())
        .map_err(|e| PicoError::Serialization(format!("Malformed IPC envelope: {}", e)))?;
    if env.protocol != IPC_PROTOCOL_VERSION {
        return Err(PicoError::Serialization(format!(
            "Unsupported IPC protocol version {} (expected {})",
            env.protocol, IPC_PROTOCOL_VERSION
        )));
    }
    Ok(env)
}

/// Lightweight blocking client helper for thin desktop UI processes without a heavy async runtime.
pub fn send_ipc_request_sync(
    endpoint: &str,
    token: Option<&str>,
    request: IpcRequest,
) -> Result<IpcResponse> {
    let env = IpcEnvelope {
        protocol: IPC_PROTOCOL_VERSION,
        token: token.map(|t| t.to_string()),
        request,
    };
    let payload = encode_ipc_line(&env)?;

    #[cfg(target_os = "windows")]
    {
        use std::fs::OpenOptions;
        let mut last_err = None;
        for _ in 0..4 {
            match OpenOptions::new().read(true).write(true).open(endpoint) {
                Ok(mut pipe) => {
                    pipe.write_all(&payload).map_err(|e| {
                        PicoError::Platform(format!("Named pipe write failed: {}", e))
                    })?;
                    pipe.flush().ok();

                    let mut reader = BufReader::new(pipe);
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).map_err(|e| {
                        PicoError::Platform(format!("Named pipe read failed: {}", e))
                    })?;
                    if n == 0 || line.len() > MAX_IPC_FRAME_BYTES {
                        return Err(PicoError::Serialization(
                            "Invalid or empty response frame from assistant daemon".into(),
                        ));
                    }
                    return serde_json::from_str::<IpcResponse>(line.trim()).map_err(|e| {
                        PicoError::Serialization(format!("Malformed daemon IPC response: {}", e))
                    });
                }
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        Err(PicoError::NotFound(format!(
            "Assistant daemon offline at '{}': {}",
            endpoint,
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unavailable".into())
        )))
    }

    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(endpoint).map_err(|e| {
            PicoError::NotFound(format!("Assistant daemon offline at '{}': {}", endpoint, e))
        })?;
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(15)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        stream.write_all(&payload)?;
        stream.flush().ok();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line.len() > MAX_IPC_FRAME_BYTES {
            return Err(PicoError::Serialization(
                "Invalid or empty response frame from assistant daemon".into(),
            ));
        }
        serde_json::from_str::<IpcResponse>(line.trim())
            .map_err(|e| PicoError::Serialization(format!("Malformed daemon IPC response: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_envelope_validation_and_bounds() {
        let env = IpcEnvelope {
            protocol: IPC_PROTOCOL_VERSION,
            token: Some("abc".into()),
            request: IpcRequest::Ping,
        };
        let encoded = encode_ipc_line(&env).unwrap();
        let line = String::from_utf8(encoded).unwrap();
        let decoded = decode_ipc_envelope(&line).unwrap();
        assert_eq!(decoded.protocol, IPC_PROTOCOL_VERSION);

        let bad_ver = r#"{"protocol":99,"request":{"kind":"Ping"}}"#;
        assert!(decode_ipc_envelope(bad_ver).is_err());

        let oversized = "x".repeat(MAX_IPC_FRAME_BYTES + 16);
        assert!(decode_ipc_envelope(&oversized).is_err());
    }
}
