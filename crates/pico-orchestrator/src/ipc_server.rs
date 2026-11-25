use crate::coordinator::PicoRuntime;
use pico_core::error::{PicoError, Result};
use pico_core::ipc::{
    decode_ipc_envelope, default_ipc_endpoint, default_ipc_token_path, encode_ipc_line,
    DaemonSnapshot, IpcRequest, IpcResponse, PendingConfirmationView, IPC_PROTOCOL_VERSION,
    MAX_IPC_FRAME_BYTES,
};
use pico_core::session::SessionId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Shared daemon state visible to local desktop & CLI IPC clients.
#[derive(Default)]
pub struct DaemonSharedState {
    pub status: Mutex<String>,
    pub voice_paused: AtomicBool,
    pub pending_confirmation: Mutex<Option<PendingConfirmationView>>,
    pub last_reply: Mutex<Option<String>>,
}

/// Active local IPC daemon server handle that removes ephemeral tokens/sockets on drop.
pub struct IpcServerHandle {
    endpoint: String,
    token_path: PathBuf,
    auth_token: String,
    shutdown: Arc<AtomicBool>,
    shared: Arc<DaemonSharedState>,
}

impl IpcServerHandle {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn token_path(&self) -> &PathBuf {
        &self.token_path
    }

    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub fn shared_state(&self) -> &Arc<DaemonSharedState> {
        &self.shared
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

impl Drop for IpcServerHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.token_path);
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.endpoint);
        }
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn write_token_file(path: &PathBuf, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(token.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, token)?;
    }

    Ok(())
}

/// Starts the authenticated local IPC server for headless assistant daemon operation.
///
/// - **Windows**: Named Pipe (`PIPE_REJECT_REMOTE_CLIENTS`, `first_pipe_instance` on initial listener,
///   explicit Owner+SYSTEM SDDL DACL) + constant-time random per-daemon bearer token.
/// - **Linux**: Unix Domain Socket with `0o600` mode + `SO_PEERCRED` effective-UID enforcement
///   + constant-time random per-daemon bearer token.
pub async fn start_ipc_server(
    runtime: Arc<PicoRuntime>,
    endpoint: Option<String>,
    token_path: Option<PathBuf>,
) -> Result<IpcServerHandle> {
    let endpoint = endpoint.unwrap_or_else(default_ipc_endpoint);
    let token_path = token_path.unwrap_or_else(default_ipc_token_path);
    let auth_token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    write_token_file(&token_path, &auth_token)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(DaemonSharedState {
        status: Mutex::new("Ready".into()),
        voice_paused: AtomicBool::new(false),
        pending_confirmation: Mutex::new(None),
        last_reply: Mutex::new(None),
    });

    #[cfg(target_os = "windows")]
    spawn_windows_pipe_loop(
        runtime,
        endpoint.clone(),
        auth_token.clone(),
        shutdown.clone(),
        shared.clone(),
    )?;

    #[cfg(unix)]
    spawn_unix_socket_loop(
        runtime,
        endpoint.clone(),
        auth_token.clone(),
        shutdown.clone(),
        shared.clone(),
    )?;

    info!(endpoint = %endpoint, "Authenticated local IPC server listening");

    Ok(IpcServerHandle {
        endpoint,
        token_path,
        auth_token,
        shutdown,
        shared,
    })
}

async fn handle_request_line(
    line: &str,
    runtime: &Arc<PicoRuntime>,
    expected_token: &str,
    shared: &Arc<DaemonSharedState>,
) -> Vec<u8> {
    let response = match decode_ipc_envelope(line) {
        Ok(env) => {
            let supplied = env.token.as_deref().unwrap_or("");
            if !constant_time_eq(supplied, expected_token) {
                IpcResponse::Error {
                    code: "unauthorized".into(),
                    message: "Invalid or missing IPC authentication token".into(),
                }
            } else {
                dispatch_request(env.request, runtime, shared).await
            }
        }
        Err(e) => IpcResponse::Error {
            code: "bad_request".into(),
            message: e.to_string(),
        },
    };

    encode_ipc_line(&response).unwrap_or_else(|_| {
        b"{\"kind\":\"Error\",\"payload\":{\"code\":\"internal\",\"message\":\"Response overflow\"}}\n"
            .to_vec()
    })
}

async fn dispatch_request(
    req: IpcRequest,
    runtime: &Arc<PicoRuntime>,
    shared: &Arc<DaemonSharedState>,
) -> IpcResponse {
    match req {
        IpcRequest::Ping => IpcResponse::Pong {
            protocol: IPC_PROTOCOL_VERSION,
            pid: std::process::id(),
        },
        IpcRequest::GetSnapshot => {
            let sid = runtime.current_session_id().await;
            let clarification = runtime.pending_clarification_view(sid).await;
            let status = shared.status.lock().await.clone();
            let pending_confirmation = shared.pending_confirmation.lock().await.clone();
            let last_reply = shared.last_reply.lock().await.clone();
            IpcResponse::Snapshot(Box::new(DaemonSnapshot {
                protocol: IPC_PROTOCOL_VERSION,
                pid: std::process::id(),
                status,
                voice_paused: shared.voice_paused.load(Ordering::Relaxed),
                is_playing_speech: runtime.voice().is_playing(),
                stt_engine: runtime.voice().stt_engine().to_string(),
                tts_engine: runtime.voice().tts_engine().to_string(),
                registered_tools: runtime.registry().all_tools().len(),
                active_session_id: sid.to_string(),
                pending_confirmation,
                pending_clarification: clarification,
                last_reply,
            }))
        }
        IpcRequest::SubmitPrompt {
            session_id,
            text,
            speak_response,
        } => {
            if text.trim().is_empty() {
                return IpcResponse::Error {
                    code: "empty_prompt".into(),
                    message: "Prompt text cannot be empty".into(),
                };
            }
            let sid = match session_id
                .as_deref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
            {
                Some(u) => SessionId(u),
                None => runtime.current_session_id().await,
            };
            {
                let mut st = shared.status.lock().await;
                *st = "Thinking".into();
            }
            let result = runtime
                .process_turn_in_session(sid, &text, speak_response)
                .await;
            {
                let mut st = shared.status.lock().await;
                *st = "Ready".into();
            }
            match result {
                Ok(reply) => {
                    let mut last = shared.last_reply.lock().await;
                    *last = Some(reply.clone());
                    IpcResponse::TurnResult {
                        session_id: sid.to_string(),
                        reply,
                    }
                }
                Err(e) => IpcResponse::Error {
                    code: "turn_error".into(),
                    message: e.to_string(),
                },
            }
        }
        IpcRequest::RespondConfirmation {
            request_id,
            approved,
        } => match runtime.respond_confirmation(&request_id, approved).await {
            Ok(()) => {
                let mut lock = shared.pending_confirmation.lock().await;
                if lock
                    .as_ref()
                    .is_some_and(|c| c.request_id == request_id)
                {
                    *lock = None;
                }
                IpcResponse::Ack { ok: true }
            }
            Err(e) => IpcResponse::Error {
                code: "confirm_error".into(),
                message: e.to_string(),
            },
        },
        IpcRequest::SetVoicePaused { paused } => {
            shared.voice_paused.store(paused, Ordering::SeqCst);
            if paused {
                runtime.voice().interrupt_playback();
            }
            IpcResponse::Ack { ok: true }
        }
        IpcRequest::InterruptSpeech => {
            runtime.cancel();
            IpcResponse::Ack { ok: true }
        }
    }
}

#[cfg(target_os = "windows")]
fn create_owner_only_pipe_instance(
    endpoint: &str,
    first_instance: bool,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use std::ffi::c_void;
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first_instance)
        .reject_remote_clients(true)
        .in_buffer_size(MAX_IPC_FRAME_BYTES as u32)
        .out_buffer_size(MAX_IPC_FRAME_BYTES as u32);

    // Owner Rights (Creator Owner) + LocalSystem DACL (blocks other local users & network guests).
    let sddl: Vec<u16> = "D:(A;;GA;;;OW)(A;;GA;;;SY)\0".encode_utf16().collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    };

    if converted.is_ok() && !sd.0.is_null() {
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let res = unsafe {
            opts.create_with_security_attributes_raw(endpoint, (&mut sa as *mut SECURITY_ATTRIBUTES).cast::<c_void>())
        };
        unsafe {
            let _ = LocalFree(HLOCAL(sd.0));
        }
        res
    } else {
        opts.create(endpoint)
    }
}

#[cfg(target_os = "windows")]
fn spawn_windows_pipe_loop(
    runtime: Arc<PicoRuntime>,
    endpoint: String,
    expected_token: String,
    shutdown: Arc<AtomicBool>,
    shared: Arc<DaemonSharedState>,
) -> Result<()> {
    let mut server = create_owner_only_pipe_instance(&endpoint, true).map_err(|e| {
        PicoError::Platform(format!(
            "Failed creating owner-restricted named pipe '{}': {}",
            endpoint, e
        ))
    })?;

    tokio::spawn(async move {
        while !shutdown.load(Ordering::Relaxed) {
            if let Err(e) = server.connect().await {
                warn!(error = %e, "Named pipe connect error");
                break;
            }
            let conn = server;
            server = match create_owner_only_pipe_instance(&endpoint, false) {
                Ok(next) => next,
                Err(e) => {
                    warn!(error = %e, "Failed rotating next named pipe instance");
                    break;
                }
            };

            let rt = Arc::clone(&runtime);
            let tok = expected_token.clone();
            let st = Arc::clone(&shared);
            tokio::spawn(async move {
                let (rx, mut tx) = tokio::io::split(conn);
                let mut reader = BufReader::new(rx);
                let mut line = String::new();
                if let Ok(n) = reader.read_line(&mut line).await {
                    if n > 0 {
                        let reply_bytes = handle_request_line(&line, &rt, &tok, &st).await;
                        let _ = tx.write_all(&reply_bytes).await;
                        let _ = tx.flush().await;
                    }
                }
            });
        }
    });

    Ok(())
}

#[cfg(unix)]
fn current_effective_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

#[cfg(unix)]
fn spawn_unix_socket_loop(
    runtime: Arc<PicoRuntime>,
    endpoint: String,
    expected_token: String,
    shutdown: Arc<AtomicBool>,
    shared: Arc<DaemonSharedState>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let sock_path = std::path::PathBuf::from(&endpoint);
    if sock_path.exists() {
        let _ = std::fs::remove_file(&sock_path);
    }
    if let Some(parent) = sock_path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let listener = tokio::net::UnixListener::bind(&sock_path).map_err(|e| {
        PicoError::Platform(format!(
            "Failed binding Unix IPC socket '{}': {}",
            endpoint, e
        ))
    })?;
    std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))?;
    let host_uid = current_effective_uid();

    tokio::spawn(async move {
        while !shutdown.load(Ordering::Relaxed) {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            if let (Some(expected_uid), Ok(cred)) = (host_uid, stream.peer_cred()) {
                if cred.uid() != expected_uid {
                    warn!(peer_uid = cred.uid(), expected_uid, "Rejected unauthorized IPC peer UID");
                    continue;
                }
            }

            let rt = Arc::clone(&runtime);
            let tok = expected_token.clone();
            let st = Arc::clone(&shared);
            tokio::spawn(async move {
                let (rx, mut tx) = tokio::io::split(stream);
                let mut reader = BufReader::new(rx);
                let mut line = String::new();
                if let Ok(n) = reader.read_line(&mut line).await {
                    if n > 0 {
                        let reply_bytes = handle_request_line(&line, &rt, &tok, &st).await;
                        let _ = tx.write_all(&reply_bytes).await;
                        let _ = tx.flush().await;
                    }
                }
            });
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::PicoConfig;
    use pico_core::ipc::{read_ipc_token, send_ipc_request_sync};

    #[tokio::test]
    async fn test_authenticated_local_ipc_server_and_token_rejection() {
        let mut cfg = PicoConfig::default();
        cfg.memory.database_path = std::env::temp_dir().join(format!(
            "pico_ipc_mem_{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        cfg.security.audit_log_path = std::env::temp_dir().join(format!(
            "pico_ipc_audit_{}.jsonl",
            uuid::Uuid::new_v4().simple()
        ));
        cfg.voice.stt_engine = "mock".into();
        cfg.voice.tts_engine = "mock".into();

        let rt = Arc::new(PicoRuntime::new(cfg.clone()).unwrap());
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        #[cfg(target_os = "windows")]
        let endpoint = format!(r"\\.\pipe\pico-ipc-test-{}", suffix);
        #[cfg(not(target_os = "windows"))]
        let endpoint = std::env::temp_dir()
            .join(format!("pico-ipc-test-{}.sock", suffix))
            .to_string_lossy()
            .into_owned();
        let token_path = std::env::temp_dir().join(format!("pico_ipc_tok_{}.txt", suffix));

        let server = start_ipc_server(rt, Some(endpoint.clone()), Some(token_path.clone()))
            .await
            .unwrap();
        let saved_token = read_ipc_token(&token_path).expect("Token file must exist");

        // 1. Valid token -> Ping succeeds and Snapshot works
        let ep_ok = endpoint.clone();
        let tok_ok = saved_token.clone();
        let pong = tokio::task::spawn_blocking(move || {
            send_ipc_request_sync(&ep_ok, Some(&tok_ok), IpcRequest::Ping)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(pong, IpcResponse::Pong { .. }));

        // 2. Wrong token -> rejected with Error("unauthorized")
        let ep_bad = endpoint.clone();
        let unauth = tokio::task::spawn_blocking(move || {
            send_ipc_request_sync(&ep_bad, Some("wrong-secret-token"), IpcRequest::Ping)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(unauth, IpcResponse::Error { code, .. } if code == "unauthorized"));

        // 3. Submit safe offline calculation prompt over IPC
        let ep_calc = endpoint.clone();
        let tok_calc = saved_token.clone();
        let turn_resp = tokio::task::spawn_blocking(move || {
            send_ipc_request_sync(
                &ep_calc,
                Some(&tok_calc),
                IpcRequest::SubmitPrompt {
                    session_id: None,
                    text: "calculate 19 * 3".into(),
                    speak_response: false,
                },
            )
        })
        .await
        .unwrap()
        .unwrap();
        match turn_resp {
            IpcResponse::TurnResult { reply, .. } => assert!(reply.contains("57"), "reply: {}", reply),
            other => panic!("Expected TurnResult, got {:?}", other),
        }

        drop(server);
        let _ = std::fs::remove_file(&cfg.memory.database_path);
        let _ = std::fs::remove_file(&cfg.security.audit_log_path);
    }
}
