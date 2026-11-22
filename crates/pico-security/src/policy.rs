use pico_core::error::{PicoError, Result};
use pico_core::traits::PermissionTier;
use std::sync::Arc;
use tracing::warn;

pub type ConfirmationCallback = Arc<dyn Fn(&str, &serde_json::Value) -> Result<bool> + Send + Sync>;

/// Security policy manager controlling tool execution permissions.
#[derive(Clone)]
pub struct SecurityPolicy {
    strict_mode: bool,
    require_confirmation_for_shell: bool,
    confirmation_handler: Option<ConfirmationCallback>,
}

impl SecurityPolicy {
    pub fn new(strict_mode: bool, require_confirmation_for_shell: bool) -> Self {
        Self {
            strict_mode,
            require_confirmation_for_shell,
            confirmation_handler: None,
        }
    }

    /// Sets a dynamic confirmation callback (e.g. for CLI prompt or Desktop UI dialog).
    pub fn set_confirmation_handler(&mut self, handler: ConfirmationCallback) {
        self.confirmation_handler = Some(handler);
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
                // If a confirmation callback is installed, query it interactively for this single invocation
                if let Some(ref handler) = self.confirmation_handler {
                    let approved = (handler)(tool_name, arguments)?;
                    if approved {
                        // Action approved once for this exact request - non-reusable
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
