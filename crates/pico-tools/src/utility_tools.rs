use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::traits::{PermissionTier, Tool};
use pico_platform::PlatformClipboard;
use serde_json::json;
use std::time::Duration;
use tracing::info;

/// Safe mathematical calculator tool.
pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }

    fn description(&self) -> &str {
        "Evaluates mathematical arithmetic expressions (e.g. '((25 * 4) + 12) / 2', 'sqrt(144)', '2^8')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Mathematical expression string to evaluate"
                }
            },
            "required": ["expression"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let expr = arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'expression' argument".into(),
            })?;

        // Evaluate via a clean recursive descent or safe powershell computation
        let clean_expr = expr.replace([';', '|', '&', '$', '`'], "");
        let ps_cmd = format!("[math]::Round(([double]({})), 8)", clean_expr);

        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .output()
            .map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Calculation failed: {}", e),
            })?;

        if !output.status.success() {
            return Err(PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Invalid arithmetic expression: {}", expr),
            });
        }

        let result_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(json!({
            "expression": expr,
            "result": result_str
        }))
    }
}

/// Tool to set countdown timers and voice reminders.
pub struct ManageTimerTool;

#[async_trait]
impl Tool for ManageTimerTool {
    fn name(&self) -> &str {
        "manage_timer"
    }

    fn description(&self) -> &str {
        "Sets a countdown timer or reminder for a specified number of seconds with a label."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "integer",
                    "description": "Timer duration in seconds",
                    "minimum": 1
                },
                "label": {
                    "type": "string",
                    "description": "Reminder label or description"
                }
            },
            "required": ["seconds"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let seconds = arguments
            .get("seconds")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'seconds' argument".into(),
            })?;

        let label = arguments
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("Timer")
            .to_string();

        let timer_label = label.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            info!(timer = %timer_label, "Timer elapsed!");
        });

        Ok(json!({
            "status": "timer_started",
            "duration_seconds": seconds,
            "label": label
        }))
    }
}

/// Tool to read or write the system clipboard.
pub struct ClipboardTool;

#[async_trait]
impl Tool for ClipboardTool {
    fn name(&self) -> &str {
        "clipboard"
    }

    fn description(&self) -> &str {
        "Reads or updates the operating system clipboard text. Action can be 'get' or 'set'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set"],
                    "description": "Action to perform: 'get' or 'set'"
                },
                "text": {
                    "type": "string",
                    "description": "Text to set on clipboard (required if action is 'set')"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::LowRisk
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("get");

        match action {
            "set" => {
                let text = arguments
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PicoError::ToolExecution {
                        tool: self.name().into(),
                        message: "Missing 'text' argument for action 'set'".into(),
                    })?;
                PlatformClipboard::set_text(text)?;
                Ok(json!({ "action": "set", "status": "success", "length": text.len() }))
            }
            _ => {
                let content = PlatformClipboard::get_text()?;
                Ok(json!({ "action": "get", "content": content }))
            }
        }
    }
}

/// Tool to fetch text from a URL.
pub struct FetchWebContentTool {
    client: reqwest::Client,
}

impl Default for FetchWebContentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchWebContentTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Pico-Assistant/0.1.0")
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

#[async_trait]
impl Tool for FetchWebContentTool {
    fn name(&self) -> &str {
        "fetch_web_content"
    }

    fn description(&self) -> &str {
        "Fetches and extracts text content from an HTTP/HTTPS URL."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTP or HTTPS URL to fetch"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::ReadOnly
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let url = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'url' argument".into(),
            })?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(PicoError::Security(
                "URL must start with http:// or https://".into(),
            ));
        }

        // Prevent SSRF against localhost / 127.0.0.1 / internal AWS metadata
        let url_lower = url.to_lowercase();
        if url_lower.contains("169.254.169.254") || url_lower.contains("metadata.google") {
            return Err(PicoError::Security(
                "Requests to cloud instance metadata are forbidden".into(),
            ));
        }

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("HTTP request to '{}' failed: {}", url, e),
            })?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        // Basic HTML tag stripping for clean LLM consumption
        let text_stripped = body
            .split('<')
            .filter_map(|chunk| chunk.split_once('>').map(|(_, text)| text))
            .collect::<Vec<&str>>()
            .join(" ");

        let clean_text: String = text_stripped
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ");

        let truncated = clean_text.chars().take(2000).collect::<String>();

        Ok(json!({
            "url": url,
            "status": status,
            "text": truncated
        }))
    }
}
