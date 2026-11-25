use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::PermissionTier;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Mutex};
use tracing::warn;

pub type ConfirmationCallback = Arc<dyn Fn(&str, &serde_json::Value) -> Result<bool> + Send + Sync>;
pub type PromptCallback = Arc<dyn Fn(&str, &str, &serde_json::Value) + Send + Sync>;

/// Asynchronous request-scoped confirmation provider interface.
#[async_trait]
pub trait ConfirmationProvider: Send + Sync {
    async fn request_confirmation(
        &self,
        request_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool>;
}

#[derive(Debug)]
struct PendingConfirmationEntry {
    sender: oneshot::Sender<bool>,
    tool_name: String,
    args_digest: String,
    session_id: Option<String>,
    expires_at: Instant,
}

/// Request-scoped confirmation manager binding every approval token strictly to
/// `request_id`, `tool_name`, argument digest, optional `session_id`, single-use consumption, and expiration.
pub struct ChannelConfirmationManager {
    pending: Arc<Mutex<HashMap<String, PendingConfirmationEntry>>>,
    prompt_callback: Option<PromptCallback>,
    timeout: Duration,
}

impl ChannelConfirmationManager {
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            prompt_callback: None,
            timeout,
        }
    }

    pub fn set_prompt_callback(&mut self, callback: PromptCallback) {
        self.prompt_callback = Some(callback);
    }

    /// Computes a deterministic SHA-256 hex digest of canonical serialized arguments.
    pub fn arguments_digest(arguments: &serde_json::Value) -> String {
        let serialized = serde_json::to_vec(arguments).unwrap_or_default();
        sha256_hex(&serialized)
    }

    /// Registers a confirmation request scoped explicitly to an optional session identifier.
    pub async fn request_confirmation_for_session(
        &self,
        session_id: Option<&str>,
        request_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool> {
        let (tx, rx) = oneshot::channel();
        let entry = PendingConfirmationEntry {
            sender: tx,
            tool_name: tool_name.to_string(),
            args_digest: Self::arguments_digest(arguments),
            session_id: session_id.map(|s| s.to_string()),
            expires_at: Instant::now() + self.timeout,
        };

        {
            let mut map = self.pending.lock().await;
            // Prune any already-expired entries first.
            let now = Instant::now();
            map.retain(|_, v| v.expires_at > now);
            if map.contains_key(request_id) {
                return Err(PicoError::Security(format!(
                    "Duplicate pending confirmation request ID '{}'",
                    request_id
                )));
            }
            map.insert(request_id.to_string(), entry);
        }

        if let Some(ref cb) = self.prompt_callback {
            (cb)(request_id, tool_name, arguments);
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(approved)) => Ok(approved),
            Ok(Err(_)) => Err(PicoError::Cancelled(
                "Confirmation request was cancelled".to_string(),
            )),
            Err(_) => {
                let mut map = self.pending.lock().await;
                map.remove(request_id);
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: format!("Confirmation request timed out after {:?}", self.timeout),
                })
            }
        }
    }

    /// Delivers a single-use confirmation response for a request ID (enforces expiration).
    pub async fn respond(&self, request_id: &str, approved: bool) -> Result<()> {
        let entry = {
            let mut map = self.pending.lock().await;
            map.remove(request_id)
        };
        let Some(entry) = entry else {
            return Err(PicoError::NotFound(format!(
                "No pending confirmation request found for ID '{}'",
                request_id
            )));
        };

        if Instant::now() >= entry.expires_at {
            let _ = entry.sender.send(false);
            return Err(PicoError::PermissionDenied {
                action: entry.tool_name,
                reason: "Confirmation token expired".to_string(),
            });
        }

        let _ = entry.sender.send(approved);
        Ok(())
    }

    /// Strict response check validating session binding, tool identity, argument digest, single-use, and expiry.
    pub async fn respond_verified(
        &self,
        request_id: &str,
        session_id: Option<&str>,
        expected_tool: &str,
        expected_arguments: &serde_json::Value,
        approved: bool,
    ) -> Result<()> {
        let mut map = self.pending.lock().await;
        let Some(entry) = map.get(request_id) else {
            return Err(PicoError::NotFound(format!(
                "No pending confirmation request found for ID '{}'",
                request_id
            )));
        };

        if Instant::now() >= entry.expires_at {
            let stale = map.remove(request_id).unwrap();
            let _ = stale.sender.send(false);
            return Err(PicoError::PermissionDenied {
                action: stale.tool_name,
                reason: "Confirmation request expired".to_string(),
            });
        }

        if entry.session_id.as_deref() != session_id
            || entry.tool_name != expected_tool
            || entry.args_digest != Self::arguments_digest(expected_arguments)
        {
            return Err(PicoError::Security(format!(
                "Confirmation context mismatch for request '{}'",
                request_id
            )));
        }

        let consumed = map.remove(request_id).unwrap();
        let _ = consumed.sender.send(approved);
        Ok(())
    }

    /// Cancels a pending confirmation request.
    pub async fn cancel(&self, request_id: &str) -> Result<()> {
        let mut map = self.pending.lock().await;
        if let Some(entry) = map.remove(request_id) {
            let _ = entry.sender.send(false);
        }
        Ok(())
    }

    /// Returns count of currently pending (unexpired) confirmations.
    pub async fn pending_count(&self) -> usize {
        let map = self.pending.lock().await;
        let now = Instant::now();
        map.values().filter(|v| v.expires_at > now).count()
    }
}

#[async_trait]
impl ConfirmationProvider for ChannelConfirmationManager {
    async fn request_confirmation(
        &self,
        request_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool> {
        self.request_confirmation_for_session(None, request_id, tool_name, arguments)
            .await
    }
}

/// Security policy manager controlling tool execution permissions.
#[derive(Clone)]
pub struct SecurityPolicy {
    strict_mode: bool,
    require_confirmation_for_shell: bool,
    confirmation_handler: Option<ConfirmationCallback>,
    confirmation_provider: Option<Arc<dyn ConfirmationProvider>>,
}

impl SecurityPolicy {
    pub fn new(strict_mode: bool, require_confirmation_for_shell: bool) -> Self {
        Self {
            strict_mode,
            require_confirmation_for_shell,
            confirmation_handler: None,
            confirmation_provider: None,
        }
    }

    /// Sets a dynamic synchronous confirmation callback.
    pub fn set_confirmation_handler(&mut self, handler: ConfirmationCallback) {
        self.confirmation_handler = Some(handler);
    }

    /// Sets an asynchronous request-scoped confirmation provider.
    pub fn set_confirmation_provider(&mut self, provider: Arc<dyn ConfirmationProvider>) {
        self.confirmation_provider = Some(provider);
    }

    /// Checks if a tool can be executed given its tier and arguments.
    pub async fn verify_permission(
        &self,
        tool_name: &str,
        tier: PermissionTier,
        arguments: &serde_json::Value,
    ) -> Result<()> {
        match tier {
            // Restricted operations can NEVER bypass explicit confirmation even in permissive mode.
            PermissionTier::Restricted => {
                self.verify_with_confirmation(tool_name, arguments, true)
                    .await
            }
            PermissionTier::ConfirmationRequired => {
                if self.confirmation_provider.is_some()
                    || self.confirmation_handler.is_some()
                    || self.strict_mode
                    || (tool_name == "execute_shell_command" && self.require_confirmation_for_shell)
                {
                    self.verify_with_confirmation(tool_name, arguments, false)
                        .await
                } else {
                    Ok(())
                }
            }
            PermissionTier::LowRisk | PermissionTier::ReadOnly => Ok(()),
        }
    }

    async fn verify_with_confirmation(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        is_restricted: bool,
    ) -> Result<()> {
        let request_id = uuid::Uuid::new_v4().to_string();
        if let Some(ref provider) = self.confirmation_provider {
            let approved = provider
                .request_confirmation(&request_id, tool_name, arguments)
                .await?;
            if approved {
                Ok(())
            } else {
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: if is_restricted {
                        "Dangerous action was declined by the user".to_string()
                    } else {
                        "Action was declined by the user".to_string()
                    },
                })
            }
        } else if let Some(ref handler) = self.confirmation_handler {
            let approved = (handler)(tool_name, arguments)?;
            if approved {
                Ok(())
            } else {
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: "Action was declined by the user".to_string(),
                })
            }
        } else {
            warn!(tool = %tool_name, is_restricted, "Action denied: confirmation provider required");
            Err(PicoError::PermissionDenied {
                action: tool_name.to_string(),
                reason: if is_restricted {
                    "Tool is restricted and requires explicit confirmation, but no confirmation provider is attached".to_string()
                } else {
                    "Tool requires explicit confirmation, but no confirmation handler is attached"
                        .to_string()
                },
            })
        }
    }
}

/// Minimal dependency-free FIPS 180-4 SHA-256 hex encoder for binding confirmation arguments.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    }

    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(out, "{:08x}", word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_restricted_tier_cannot_bypass_even_in_permissive_mode() {
        let policy = SecurityPolicy::new(false, false);
        let res = policy
            .verify_permission(
                "format_disk",
                PermissionTier::Restricted,
                &serde_json::json!({}),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_confirmation_replay_and_context_mismatch_blocked() {
        let mut mgr = ChannelConfirmationManager::new(Duration::from_secs(2));
        let captured = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured_clone = captured.clone();
        mgr.set_prompt_callback(Arc::new(move |id, _, _| {
            let list = captured_clone.clone();
            let id_s = id.to_string();
            tokio::spawn(async move {
                list.lock().await.push(id_s);
            });
        }));
        let mgr = Arc::new(mgr);

        let args = serde_json::json!({ "path": "a.txt" });
        let mgr_task = mgr.clone();
        let args_task = args.clone();
        let req_fut = tokio::spawn(async move {
            mgr_task
                .request_confirmation_for_session(Some("s1"), "req-1", "write_file", &args_task)
                .await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(mgr.pending_count().await, 1);

        // Wrong session or altered arguments must be rejected without consuming token.
        assert!(mgr
            .respond_verified("req-1", Some("s2"), "write_file", &args, true)
            .await
            .is_err());
        assert!(mgr
            .respond_verified(
                "req-1",
                Some("s1"),
                "write_file",
                &serde_json::json!({ "path": "evil.txt" }),
                true
            )
            .await
            .is_err());
        assert_eq!(mgr.pending_count().await, 1);

        // Matching verification succeeds once, then replay is rejected.
        assert!(mgr
            .respond_verified("req-1", Some("s1"), "write_file", &args, true)
            .await
            .is_ok());
        assert!(mgr.respond("req-1", true).await.is_err());
        assert!(req_fut.await.unwrap().unwrap());
    }
}
