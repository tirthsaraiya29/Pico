use pico_core::error::PicoError;
use pico_core::session::ToolResult;
use pico_core::traits::Tool;
use pico_security::{AuditLogger, ConfirmationProvider, SecurityPolicy};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    security_policy: RwLock<SecurityPolicy>,
    audit_logger: Option<Arc<AuditLogger>>,
    default_timeout: Duration,
}

impl ToolRegistry {
    pub fn new(security_policy: SecurityPolicy, audit_logger: Option<Arc<AuditLogger>>) -> Self {
        Self {
            tools: HashMap::new(),
            security_policy: RwLock::new(security_policy),
            audit_logger,
            default_timeout: Duration::from_secs(30),
        }
    }

    /// Dynamically sets an asynchronous confirmation provider.
    pub async fn set_confirmation_provider(&self, provider: Arc<dyn ConfirmationProvider>) {
        let mut policy = self.security_policy.write().await;
        policy.set_confirmation_provider(provider);
    }

    /// Registers a tool in the registry.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        debug!(name = %tool.name(), "Registered tool");
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Registers a batch of tools into the registry efficiently.
    pub fn register_batch(&mut self, tools: Vec<Arc<dyn Tool>>) {
        let count = tools.len();
        for tool in tools {
            self.tools.insert(tool.name().to_string(), tool);
        }
        info!(count = count, "Registered batch of tools");
    }

    /// Sets tool execution timeout.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.default_timeout = timeout;
    }

    /// Returns a list of all registered tools.
    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }

    /// Retrieves a specific tool by name.
    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Returns OpenAI/Ollama tool definitions JSON array.
    pub fn get_tool_definitions(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters_schema()
                    }
                })
            })
            .collect()
    }

    /// Executes a tool call with safety validation, permission check, and audit logging.
    pub async fn execute_call(
        &self,
        call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let start = Instant::now();
        let tool = match self.tools.get(tool_name) {
            Some(t) => t.clone(),
            None => {
                let err_msg = format!("Tool '{}' is not registered", tool_name);
                if let Some(ref audit) = self.audit_logger {
                    audit.log_attempt(
                        tool_name,
                        "Unknown",
                        &arguments,
                        "NotFound",
                        Some(start.elapsed().as_millis() as u64),
                        Some(&err_msg),
                    );
                }
                return ToolResult {
                    call_id: call_id.to_string(),
                    name: tool_name.to_string(),
                    success: false,
                    content: serde_json::json!({ "error": err_msg }),
                    error: Some(err_msg),
                    execution_duration_ms: start.elapsed().as_millis() as u64,
                };
            }
        };

        let tier = tool.permission_tier();
        let tier_str = format!("{:?}", tier);

        // Security permission check
        let verify_res = {
            let policy = self.security_policy.read().await;
            policy.verify_permission(tool_name, tier, &arguments).await
        };

        if let Err(e) = verify_res {
            let err_msg = e.to_string();
            warn!(tool = %tool_name, error = %err_msg, "Tool execution denied by security policy");
            if let Some(ref audit) = self.audit_logger {
                audit.log_attempt(
                    tool_name,
                    &tier_str,
                    &arguments,
                    "Denied",
                    Some(start.elapsed().as_millis() as u64),
                    Some(&err_msg),
                );
            }
            return ToolResult {
                call_id: call_id.to_string(),
                name: tool_name.to_string(),
                success: false,
                content: serde_json::json!({ "error": err_msg }),
                error: Some(err_msg),
                execution_duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Execute tool with timeout
        debug!(tool = %tool_name, "Executing tool");
        let timeout = self.default_timeout;
        let exec_future = tool.execute(arguments.clone());

        let res = match tokio::time::timeout(timeout, exec_future).await {
            Ok(inner_res) => inner_res,
            Err(_) => Err(PicoError::Timeout(timeout.as_millis() as u64)),
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        match res {
            Ok(val) => {
                info!(tool = %tool_name, duration_ms, "Tool executed successfully");
                if let Some(ref audit) = self.audit_logger {
                    audit.log_attempt(
                        tool_name,
                        &tier_str,
                        &arguments,
                        "Success",
                        Some(duration_ms),
                        None,
                    );
                }
                ToolResult {
                    call_id: call_id.to_string(),
                    name: tool_name.to_string(),
                    success: true,
                    content: val,
                    error: None,
                    execution_duration_ms: duration_ms,
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                error!(tool = %tool_name, error = %err_msg, "Tool execution failed");
                if let Some(ref audit) = self.audit_logger {
                    audit.log_attempt(
                        tool_name,
                        &tier_str,
                        &arguments,
                        "Failed",
                        Some(duration_ms),
                        Some(&err_msg),
                    );
                }
                ToolResult {
                    call_id: call_id.to_string(),
                    name: tool_name.to_string(),
                    success: false,
                    content: serde_json::json!({ "error": err_msg }),
                    error: Some(err_msg),
                    execution_duration_ms: duration_ms,
                }
            }
        }
    }
}
