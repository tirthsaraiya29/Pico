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
        "Evaluates, calculates, computes, and solves mathematical arithmetic expressions, calculations, equations, and formulas (e.g. '((25 * 4) + 12) / 2', 'sqrt(144)', '2^8')."
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

        // Zero-shell arithmetic AST evaluation
        let val = crate::calculator_parser::evaluate_expression(expr).map_err(|e| {
            PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Calculation failed: {}", e),
            }
        })?;

        // Format result nicely (round to 8 decimal places if float, or display clean integer)
        let formatted = if (val.fract()).abs() < 1e-10 {
            format!("{}", val as i64)
        } else {
            format!("{:.8}", val)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };

        Ok(json!({
            "expression": expr,
            "result": formatted
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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Evaluates whether an IP address is a safe public IP (rejects loopback, private RFC1918, link-local, cloud metadata, multicast, etc.).
pub fn is_safe_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_safe_public_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                is_safe_public_ipv4(v4)
            } else {
                is_safe_public_ipv6(v6)
            }
        }
    }
}

fn is_safe_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    // 0.0.0.0/8 (Current network)
    if octets[0] == 0 {
        return false;
    }
    // 127.0.0.0/8 (Loopback)
    if ip.is_loopback() {
        return false;
    }
    // 10.0.0.0/8 (RFC 1918 Private)
    if octets[0] == 10 {
        return false;
    }
    // 172.16.0.0/12 (RFC 1918 Private)
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return false;
    }
    // 192.168.0.0/16 (RFC 1918 Private)
    if octets[0] == 192 && octets[1] == 168 {
        return false;
    }
    // 169.254.0.0/16 (Link-Local & Cloud Metadata AWS 169.254.169.254 / Azure / GCP)
    if octets[0] == 169 && octets[1] == 254 {
        return false;
    }
    // 100.64.0.0/10 (Shared Address Space / CGNAT)
    if octets[0] == 100 && (octets[1] & 0xC0) == 64 {
        return false;
    }
    // 192.0.0.0/24 (IETF Protocol Assignments)
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return false;
    }
    // 192.0.2.0/24 (TEST-NET-1)
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return false;
    }
    // 198.18.0.0/15 (Network benchmark tests)
    if octets[0] == 198 && (octets[1] & 0xFE) == 18 {
        return false;
    }
    // 198.51.100.0/24 (TEST-NET-2)
    if octets[0] == 198 && octets[1] == 51 && octets[2] == 100 {
        return false;
    }
    // 203.0.113.0/24 (TEST-NET-3)
    if octets[0] == 203 && octets[1] == 0 && octets[2] == 113 {
        return false;
    }
    // 224.0.0.0/4 (Multicast)
    if ip.is_multicast() {
        return false;
    }
    // 240.0.0.0/4 (Reserved) & 255.255.255.255 (Broadcast)
    if octets[0] >= 240 {
        return false;
    }
    true
}

fn is_safe_public_ipv6(ip: Ipv6Addr) -> bool {
    // Loopback ::1
    if ip.is_loopback() {
        return false;
    }
    // Unspecified ::
    if ip.is_unspecified() {
        return false;
    }
    // Multicast ff00::/8
    if ip.is_multicast() {
        return false;
    }
    let segs = ip.segments();
    // Unique Local fc00::/7 (RFC 4193 private)
    if (segs[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // Link-Local Unicast fe80::/10 (RFC 4291)
    if (segs[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    true
}

async fn validate_url_ssrf(url_str: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| PicoError::Security(format!("Invalid URL '{}': {}", url_str, e)))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(PicoError::Security(format!(
            "Scheme '{}' is not permitted. Only http and https are allowed.",
            parsed.scheme()
        )));
    }

    let host_str = parsed
        .host_str()
        .ok_or_else(|| PicoError::Security("URL must contain a valid host".into()))?;

    if host_str.eq_ignore_ascii_case("localhost")
        || host_str.ends_with(".local")
        || host_str.ends_with(".internal")
        || host_str.ends_with(".localhost")
    {
        return Err(PicoError::Security(format!(
            "Host '{}' is an internal or local domain",
            host_str
        )));
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    let socket_addrs = tokio::net::lookup_host((host_str, port))
        .await
        .map_err(|e| PicoError::ToolExecution {
            tool: "fetch_web_content".into(),
            message: format!("DNS resolution failed for '{}': {}", host_str, e),
        })?;

    let mut resolved_any = false;
    for addr in socket_addrs {
        resolved_any = true;
        if !is_safe_public_ip(addr.ip()) {
            return Err(PicoError::Security(format!(
                "Host '{}' resolved to forbidden private/internal IP address '{}'",
                host_str,
                addr.ip()
            )));
        }
    }

    if !resolved_any {
        return Err(PicoError::ToolExecution {
            tool: "fetch_web_content".into(),
            message: format!("No IP addresses resolved for '{}'", host_str),
        });
    }

    Ok(parsed)
}

/// Tool to fetch text from a URL with robust anti-SSRF protections.
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
            .redirect(reqwest::redirect::Policy::none()) // Manual verified redirect validation
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
        "Fetches and extracts text content from a public HTTP/HTTPS URL with strict SSRF defense."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The public HTTP or HTTPS URL to fetch"
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
        let mut target_url_str = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PicoError::ToolExecution {
                tool: self.name().into(),
                message: "Missing 'url' argument".into(),
            })?
            .to_string();

        let mut hops = 0;
        let max_hops = 5;

        loop {
            let validated_url = validate_url_ssrf(&target_url_str).await?;

            let resp = self
                .client
                .get(validated_url.clone())
                .send()
                .await
                .map_err(|e| PicoError::ToolExecution {
                    tool: self.name().into(),
                    message: format!("HTTP request to '{}' failed: {}", target_url_str, e),
                })?;

            let status = resp.status();
            if status.is_redirection() && hops < max_hops {
                hops += 1;
                if let Some(loc) = resp.headers().get(reqwest::header::LOCATION) {
                    let loc_str = loc.to_str().map_err(|_| {
                        PicoError::Security("Malformed redirect location header".into())
                    })?;
                    let next_url = validated_url
                        .join(loc_str)
                        .map_err(|e| PicoError::Security(format!("Invalid redirect URL: {}", e)))?;
                    target_url_str = next_url.to_string();
                    continue;
                }
            }

            let status_code = status.as_u16();
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

            return Ok(json!({
                "url": target_url_str,
                "status": status_code,
                "text": truncated
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_calculator_tool_ast() {
        let calc = CalculatorTool;
        let res = calc
            .execute(json!({ "expression": "((10 + 5) * 2) / 3" }))
            .await
            .unwrap();
        assert_eq!(res["result"], "10");

        let res2 = calc
            .execute(json!({ "expression": "sqrt(16) + 2^3" }))
            .await
            .unwrap();
        assert_eq!(res2["result"], "12");

        // Malicious input attempts MUST fail cleanly without execution
        assert!(calc
            .execute(json!({ "expression": "cmd.exe /c calc" }))
            .await
            .is_err());
        assert!(calc
            .execute(json!({ "expression": "powershell whoami" }))
            .await
            .is_err());
        assert!(calc
            .execute(json!({ "expression": "5 / 0" }))
            .await
            .is_err());
    }

    #[test]
    fn test_ssrf_ip_filtering() {
        use std::str::FromStr;

        // Loopback addresses must be rejected
        assert!(!is_safe_public_ip(IpAddr::from_str("127.0.0.1").unwrap()));
        assert!(!is_safe_public_ip(IpAddr::from_str("127.1.2.3").unwrap()));
        assert!(!is_safe_public_ip(IpAddr::from_str("::1").unwrap()));

        // RFC 1918 Private addresses must be rejected
        assert!(!is_safe_public_ip(IpAddr::from_str("10.0.0.1").unwrap()));
        assert!(!is_safe_public_ip(IpAddr::from_str("172.16.5.10").unwrap()));
        assert!(!is_safe_public_ip(IpAddr::from_str("192.168.1.1").unwrap()));

        // Cloud metadata (169.254.169.254) & Link-local must be rejected
        assert!(!is_safe_public_ip(
            IpAddr::from_str("169.254.169.254").unwrap()
        ));
        assert!(!is_safe_public_ip(IpAddr::from_str("fe80::1").unwrap()));

        // Public IPs should be allowed
        assert!(is_safe_public_ip(IpAddr::from_str("8.8.8.8").unwrap()));
        assert!(is_safe_public_ip(IpAddr::from_str("1.1.1.1").unwrap()));
    }

    #[tokio::test]
    async fn test_ssrf_url_validation() {
        // Localhost and private schemes must be rejected
        assert!(validate_url_ssrf("http://localhost:8080").await.is_err());
        assert!(validate_url_ssrf("http://127.0.0.1:8765").await.is_err());
        assert!(validate_url_ssrf("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(validate_url_ssrf("ftp://example.com").await.is_err());
        assert!(validate_url_ssrf("file:///etc/passwd").await.is_err());
    }
}
