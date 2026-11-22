use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::PermissionTier;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
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

/// Thread-safe, request-scoped confirmation manager using oneshot channels and timeouts.
/// Prevents confirmation state from leaking across concurrent requests or sessions.
pub struct ChannelConfirmationManager {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
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

    /// Delivers an interactive confirmation response for a specific request ID.
    pub async fn respond(&self, request_id: &str, approved: bool) -> Result<()> {
        let mut map = self.pending.lock().await;
        if let Some(tx) = map.remove(request_id) {
            let _ = tx.send(approved);
            Ok(())
        } else {
            Err(PicoError::NotFound(format!(
                "No pending confirmation request found for ID '{}'",
                request_id
            )))
        }
    }

    /// Cancels a pending confirmation request.
    pub async fn cancel(&self, request_id: &str) -> Result<()> {
        let mut map = self.pending.lock().await;
        if let Some(tx) = map.remove(request_id) {
            let _ = tx.send(false);
            Ok(())
        } else {
            Ok(())
        }
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
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(request_id.to_string(), tx);
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
            PermissionTier::Restricted => {
                warn!(tool = %tool_name, "Security violation: Attempted execution of Restricted tool");
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: "Tool is marked Restricted and cannot be executed under any policy"
                        .to_string(),
                })
            }
            PermissionTier::ConfirmationRequired => {
                let request_id = uuid::Uuid::new_v4().to_string();
                if let Some(ref provider) = self.confirmation_provider {
                    let approved = provider
                        .request_confirmation(&request_id, tool_name, arguments)
                        .await?;
                    if approved {
                        // Action approved once for this exact request - non-reusable
                        Ok(())
                    } else {
                        Err(PicoError::PermissionDenied {
                            action: tool_name.to_string(),
                            reason: "Action was declined by the user".to_string(),
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
                } else if self.strict_mode
                    || (tool_name == "execute_shell_command" && self.require_confirmation_for_shell)
                {
                    Err(PicoError::PermissionDenied {
                        action: tool_name.to_string(),
                        reason: "Tool requires explicit confirmation but no interactive handler is attached".to_string(),
                    })
                } else {
                    Ok(())
                }
            }
            PermissionTier::LowRisk | PermissionTier::ReadOnly => Ok(()),
        }
    }
}
