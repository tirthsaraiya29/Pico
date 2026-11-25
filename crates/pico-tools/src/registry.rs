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

    /// Validates that a tool's JSON Schema is a well-formed object schema with valid
    /// `properties`, `required` fields, and parameter types.
    pub fn validate_schema(
        tool_name: &str,
        schema: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
            return Err(format!(
                "Tool '{}' schema must have type == 'object'",
                tool_name
            ));
        }

        let props = match schema.get("properties") {
            Some(p) => p.as_object().ok_or_else(|| {
                format!(
                    "Tool '{}' schema 'properties' must be a JSON object",
                    tool_name
                )
            })?,
            None => {
                return Err(format!(
                    "Tool '{}' schema must declare 'properties' object",
                    tool_name
                ))
            }
        };

        const VALID_TYPES: [&str; 7] = [
            "string", "number", "integer", "boolean", "array", "object", "null",
        ];

        for (prop_name, prop_def) in props {
            let obj = prop_def.as_object().ok_or_else(|| {
                format!(
                    "Tool '{}' property '{}' definition must be an object",
                    tool_name, prop_name
                )
            })?;
            let has_combinator =
                obj.contains_key("enum") || obj.contains_key("anyOf") || obj.contains_key("oneOf");
            if let Some(ty) = obj.get("type") {
                if let Some(s) = ty.as_str() {
                    if !VALID_TYPES.contains(&s) {
                        return Err(format!(
                            "Tool '{}' property '{}' has invalid type '{}'",
                            tool_name, prop_name, s
                        ));
                    }
                } else if let Some(arr) = ty.as_array() {
                    for item in arr {
                        let s = item.as_str().unwrap_or("");
                        if !VALID_TYPES.contains(&s) {
                            return Err(format!(
                                "Tool '{}' property '{}' has invalid type entry '{}'",
                                tool_name, prop_name, s
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "Tool '{}' property '{}' has non-string 'type'",
                        tool_name, prop_name
                    ));
                }
            } else if !has_combinator {
                return Err(format!(
                    "Tool '{}' property '{}' must specify 'type' or combinator",
                    tool_name, prop_name
                ));
            }
        }

        if let Some(req) = schema.get("required") {
            let req_arr = req.as_array().ok_or_else(|| {
                format!("Tool '{}' schema 'required' must be an array", tool_name)
            })?;
            for item in req_arr {
                let field = item.as_str().ok_or_else(|| {
                    format!(
                        "Tool '{}' schema 'required' contains non-string entry",
                        tool_name
                    )
                })?;
                if !props.contains_key(field) {
                    return Err(format!(
                        "Tool '{}' schema 'required' field '{}' is missing from 'properties'",
                        tool_name, field
                    ));
                }
            }
        }

        Ok(())
    }

    /// Registers a tool in the registry after validating its schema contract.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let schema = tool.parameters_schema();
        if let Err(err) = Self::validate_schema(tool.name(), &schema) {
            warn!(error = %err, "Tool schema validation failed");
            debug_assert!(false, "{}", err);
        }
        debug!(name = %tool.name(), "Registered tool");
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Registers a batch of tools into the registry efficiently.
    pub fn register_batch(&mut self, tools: Vec<Arc<dyn Tool>>) {
        let count = tools.len();
        for tool in tools {
            self.register(tool);
        }
        info!(count = count, "Registered batch of tools");
    }

    /// Sets tool execution timeout.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.default_timeout = timeout;
    }

    /// Returns a deterministically sorted list of all registered tools.
    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut list: Vec<Arc<dyn Tool>> = self.tools.values().cloned().collect();
        list.sort_by(|a, b| a.name().cmp(b.name()));
        list
    }

    /// Retrieves a specific tool by name.
    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Returns deterministically ordered tool definitions JSON array.
    pub fn get_tool_definitions(&self) -> Vec<serde_json::Value> {
        self.all_tools()
            .into_iter()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_schema_catches_malformed_definitions() {
        let valid = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "lines": { "type": "integer" }
            },
            "required": ["path"]
        });
        assert!(ToolRegistry::validate_schema("valid_tool", &valid).is_ok());

        let missing_prop = serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["nonexistent"]
        });
        assert!(ToolRegistry::validate_schema("bad_req", &missing_prop).is_err());

        let bad_type = serde_json::json!({
            "type": "object",
            "properties": { "x": { "type": "float64" } }
        });
        assert!(ToolRegistry::validate_schema("bad_type", &bad_type).is_err());
    }
}
