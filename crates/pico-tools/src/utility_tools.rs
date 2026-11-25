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

use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimerEntry {
    pub id: String,
    pub label: String,
    pub duration_seconds: u64,
    pub remaining_seconds: u64,
    pub paused: bool,
    pub deadline_utc: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Thread-safe, wall-clock-backed persistent timer manager (create, list, pause, resume, cancel).
#[derive(Debug, Default)]
pub struct ManageTimerTool {
    timers: Arc<StdMutex<Vec<TimerEntry>>>,
    persist_path: Option<std::path::PathBuf>,
}

impl ManageTimerTool {
    pub fn with_persist_path(path: impl Into<std::path::PathBuf>) -> Self {
        let p = path.into();
        let loaded = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<TimerEntry>>(&s).ok())
            .unwrap_or_default();
        Self {
            timers: Arc::new(StdMutex::new(loaded)),
            persist_path: Some(p),
        }
    }

    fn save_locked(&self, list: &[TimerEntry]) {
        if let Some(ref p) = self.persist_path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(list) {
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
    }

    pub fn snapshot(&self) -> Vec<TimerEntry> {
        let now = Utc::now();
        let mut guard = self.timers.lock().unwrap();
        for t in guard.iter_mut() {
            if !t.paused {
                if let Some(deadline) = t.deadline_utc {
                    let rem = (deadline - now).num_seconds();
                    t.remaining_seconds = rem.max(0) as u64;
                }
            }
        }
        guard.clone()
    }
}

#[async_trait]
impl Tool for ManageTimerTool {
    fn name(&self) -> &str {
        "manage_timer"
    }

    fn description(&self) -> &str {
        "Creates, lists, pauses, resumes, or cancels persistent countdown timers and reminders."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "pause", "resume", "cancel"],
                    "description": "Timer action (defaults to 'create')"
                },
                "seconds": {
                    "type": "integer",
                    "description": "Timer duration in seconds (required when creating)",
                    "minimum": 1
                },
                "label": {
                    "type": "string",
                    "description": "Reminder label or timer identifier"
                }
            },
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
            .unwrap_or("create");
        let label = arguments
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("Timer")
            .trim()
            .to_string();

        match action {
            "list" => {
                let items = self.snapshot();
                let count = items.len();
                Ok(json!({
                    "action": "list",
                    "count": count,
                    "timers": items,
                    "message": format!("You have {} timer(s).", count)
                }))
            }
            "pause" => {
                let now = Utc::now();
                let mut guard = self.timers.lock().unwrap();
                let target = guard.iter_mut().find(|t| {
                    !t.paused
                        && (label == "Timer"
                            || t.label.to_lowercase().contains(&label.to_lowercase()))
                });
                if let Some(t) = target {
                    if let Some(deadline) = t.deadline_utc {
                        t.remaining_seconds = (deadline - now).num_seconds().max(0) as u64;
                    }
                    t.paused = true;
                    t.deadline_utc = None;
                    let t_label = t.label.clone();
                    let rem = t.remaining_seconds;
                    self.save_locked(&guard);
                    Ok(json!({
                        "action": "pause",
                        "label": t_label,
                        "remaining_seconds": rem,
                        "message": format!("Paused timer '{}' with {} seconds remaining.", t_label, rem)
                    }))
                } else {
                    Err(PicoError::NotFound(
                        "No active matching timer to pause".into(),
                    ))
                }
            }
            "resume" => {
                let now = Utc::now();
                let mut guard = self.timers.lock().unwrap();
                let target = guard.iter_mut().find(|t| {
                    t.paused
                        && (label == "Timer"
                            || t.label.to_lowercase().contains(&label.to_lowercase()))
                });
                if let Some(t) = target {
                    t.paused = false;
                    t.deadline_utc =
                        Some(now + chrono::Duration::seconds(t.remaining_seconds as i64));
                    let t_label = t.label.clone();
                    let rem = t.remaining_seconds;
                    self.save_locked(&guard);
                    Ok(json!({
                        "action": "resume",
                        "label": t_label,
                        "remaining_seconds": rem,
                        "message": format!("Resumed timer '{}' ({} seconds remaining).", t_label, rem)
                    }))
                } else {
                    Err(PicoError::NotFound(
                        "No paused matching timer to resume".into(),
                    ))
                }
            }
            "cancel" => {
                let mut guard = self.timers.lock().unwrap();
                let before = guard.len();
                if label == "Timer" {
                    guard.pop();
                } else {
                    let needle = label.to_lowercase();
                    guard.retain(|t| !t.label.to_lowercase().contains(&needle));
                }
                let removed = before.saturating_sub(guard.len());
                self.save_locked(&guard);
                if removed == 0 {
                    return Err(PicoError::NotFound(
                        "No matching timer found to cancel".into(),
                    ));
                }
                Ok(json!({
                    "action": "cancel",
                    "removed": removed,
                    "message": format!("Cancelled {} timer(s).", removed)
                }))
            }
            _ => {
                let seconds = arguments
                    .get("seconds")
                    .and_then(|v| v.as_u64())
                    .filter(|&s| s > 0)
                    .ok_or_else(|| PicoError::ToolExecution {
                        tool: self.name().into(),
                        message: "Missing positive 'seconds' argument for timer creation".into(),
                    })?;

                let now = Utc::now();
                let id = uuid::Uuid::new_v4().to_string();
                let deadline = now + chrono::Duration::seconds(seconds as i64);
                let entry = TimerEntry {
                    id: id.clone(),
                    label: label.clone(),
                    duration_seconds: seconds,
                    remaining_seconds: seconds,
                    paused: false,
                    deadline_utc: Some(deadline),
                    created_at: now,
                };
                {
                    let mut guard = self.timers.lock().unwrap();
                    guard.push(entry);
                    self.save_locked(&guard);
                }

                let timers_ref = Arc::clone(&self.timers);
                let id_for_task = id.clone();
                let label_for_task = label.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                    let still_active = {
                        let mut guard = timers_ref.lock().unwrap();
                        if let Some(pos) = guard.iter().position(|t| {
                            t.id == id_for_task
                                && !t.paused
                                && t.deadline_utc
                                    .map(|d| Utc::now() >= d - chrono::Duration::seconds(1))
                                    .unwrap_or(false)
                        }) {
                            guard.remove(pos);
                            true
                        } else {
                            false
                        }
                    };
                    if still_active {
                        info!(timer = %label_for_task, "Timer elapsed");
                    }
                });

                Ok(json!({
                    "action": "create",
                    "status": "timer_started",
                    "id": id,
                    "duration_seconds": seconds,
                    "label": label,
                    "deadline_utc": deadline.to_rfc3339(),
                    "message": format!("Set a timer for {} seconds ({}).", seconds, label)
                }))
            }
        }
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
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // Check IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) addresses.
    if let Some(v4) = ip.to_ipv4_mapped().or_else(|| ip.to_ipv4()) {
        return is_safe_public_ipv4(v4);
    }

    let segs = ip.segments();
    // Unique Local fc00::/7 (RFC 4193)
    if (segs[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // Link-Local Unicast fe80::/10 (RFC 4291) & deprecated Site-Local fec0::/10
    if (segs[0] & 0xffc0) == 0xfe80 || (segs[0] & 0xffc0) == 0xfec0 {
        return false;
    }
    // Discard-Only 0100::/64 (RFC 6666)
    if segs[0] == 0x0100 && segs[1] == 0 && segs[2] == 0 && segs[3] == 0 {
        return false;
    }
    // NAT64 well-known (64:ff9b::/96) and local-use (64:ff9b:1::/48)
    if segs[0] == 0x0064 && segs[1] == 0xff9b {
        if segs[2] == 0 && segs[3] == 0 && segs[4] == 0 && segs[5] == 0 {
            let embedded = Ipv4Addr::new(
                (segs[6] >> 8) as u8,
                (segs[6] & 0xff) as u8,
                (segs[7] >> 8) as u8,
                (segs[7] & 0xff) as u8,
            );
            return is_safe_public_ipv4(embedded);
        }
        return false;
    }
    // 6to4 2002::/16 (RFC 3056) embeds IPv4 in segs[1..=2]
    if segs[0] == 0x2002 {
        let embedded = Ipv4Addr::new(
            (segs[1] >> 8) as u8,
            (segs[1] & 0xff) as u8,
            (segs[2] >> 8) as u8,
            (segs[2] & 0xff) as u8,
        );
        return is_safe_public_ipv4(embedded);
    }
    // Teredo 2001::/32, Benchmarking 2001:2::/48, ORCHID 2001:10::/28, Documentation 2001:db8::/32
    !(segs[0] == 0x2001
        && (segs[1] == 0 || segs[1] == 0x0002 || (segs[1] & 0xfff0) == 0x0010 || segs[1] == 0x0db8))
}

async fn validate_and_resolve_ssrf(
    url_str: &str,
) -> Result<(reqwest::Url, Vec<std::net::SocketAddr>)> {
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| PicoError::Security(format!("Invalid URL '{}': {}", url_str, e)))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(PicoError::Security(format!(
            "Scheme '{}' is not permitted. Only http and https are allowed.",
            parsed.scheme()
        )));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PicoError::Security(
            "URLs containing embedded user credentials are forbidden".into(),
        ));
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
    let socket_addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host_str, port))
        .await
        .map_err(|e| PicoError::ToolExecution {
            tool: "fetch_web_content".into(),
            message: format!("DNS resolution failed for '{}': {}", host_str, e),
        })?
        .collect();

    if socket_addrs.is_empty() {
        return Err(PicoError::ToolExecution {
            tool: "fetch_web_content".into(),
            message: format!("No IP addresses resolved for '{}'", host_str),
        });
    }

    for addr in &socket_addrs {
        if !is_safe_public_ip(addr.ip()) {
            return Err(PicoError::Security(format!(
                "Host '{}' resolved to forbidden private/internal IP address '{}'",
                host_str,
                addr.ip()
            )));
        }
    }

    Ok((parsed, socket_addrs))
}

/// Tool to fetch text from a URL with anti-SSRF validation, DNS pinning, and bounded response size.
pub struct FetchWebContentTool;

impl Default for FetchWebContentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchWebContentTool {
    pub fn new() -> Self {
        Self
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
        const MAX_HTTP_BODY_BYTES: usize = 256 * 1024;
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
            let (validated_url, pinned_addrs) = validate_and_resolve_ssrf(&target_url_str).await?;
            let host = validated_url
                .host_str()
                .ok_or_else(|| PicoError::Security("URL missing host".into()))?;

            // Pin resolved & validated socket addresses directly to defeat DNS rebinding TOCTOU.
            let pinned_client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Pico-Assistant/0.1.0")
                .resolve_to_addrs(host, &pinned_addrs)
                .build()
                .map_err(|e| PicoError::ToolExecution {
                    tool: self.name().into(),
                    message: format!("HTTP client build error: {}", e),
                })?;

            let mut resp = pinned_client
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
            let mut body_bytes = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| PicoError::ToolExecution {
                tool: self.name().into(),
                message: format!("Failed reading HTTP body: {}", e),
            })? {
                let remaining = MAX_HTTP_BODY_BYTES.saturating_sub(body_bytes.len());
                if remaining == 0 {
                    break;
                }
                let take = chunk.len().min(remaining);
                body_bytes.extend_from_slice(&chunk[..take]);
                if body_bytes.len() >= MAX_HTTP_BODY_BYTES {
                    break;
                }
            }
            let body = String::from_utf8_lossy(&body_bytes);

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
                "text": truncated,
                "bytes_fetched": body_bytes.len()
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

        // Loopback, RFC1918, metadata, IPv6-mapped, NAT64, 6to4, Teredo, documentation all rejected
        for blocked in [
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "10.0.0.1",
            "172.16.5.10",
            "192.168.1.1",
            "169.254.169.254",
            "fe80::1",
            "64:ff9b::127.0.0.1",
            "64:ff9b:1::1",
            "2002:7f00:0001::1",
            "2002:a9fe:a9fe::1",
            "2001:0::1",
            "2001:db8::1",
            "fec0::1",
        ] {
            assert!(
                !is_safe_public_ip(IpAddr::from_str(blocked).unwrap()),
                "Internal/tunnel IP '{}' must be rejected",
                blocked
            );
        }

        // Genuine public IPs remain permitted
        assert!(is_safe_public_ip(IpAddr::from_str("8.8.8.8").unwrap()));
        assert!(is_safe_public_ip(IpAddr::from_str("1.1.1.1").unwrap()));
        assert!(is_safe_public_ip(
            IpAddr::from_str("2001:4860:4860::8888").unwrap()
        ));
    }

    #[tokio::test]
    async fn test_ssrf_url_validation() {
        assert!(validate_and_resolve_ssrf("http://localhost:8080")
            .await
            .is_err());
        assert!(validate_and_resolve_ssrf("http://127.0.0.1:8765")
            .await
            .is_err());
        assert!(
            validate_and_resolve_ssrf("http://169.254.169.254/latest/meta-data")
                .await
                .is_err()
        );
        assert!(validate_and_resolve_ssrf("https://user:pass@8.8.8.8/")
            .await
            .is_err());
        assert!(validate_and_resolve_ssrf("ftp://example.com")
            .await
            .is_err());
        assert!(validate_and_resolve_ssrf("file:///etc/passwd")
            .await
            .is_err());
    }
}
