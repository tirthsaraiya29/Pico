use pico_core::error::{PicoError, Result};
use std::process::Command;

pub struct PlatformClipboard;

impl PlatformClipboard {
    /// Gets current text content from the system clipboard.
    pub fn get_text() -> Result<String> {
        #[cfg(target_os = "windows")]
        {
            let output = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard"])
                .output()
                .map_err(|e| PicoError::Platform(format!("Failed to read clipboard: {}", e)))?;

            if !output.status.success() {
                return Err(PicoError::Platform(
                    "Failed to get clipboard content".into(),
                ));
            }

            Ok(String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string())
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(String::new())
        }
    }

    /// Sets text content into the system clipboard.
    pub fn set_text(text: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            use std::io::Write;
            let mut child = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$input | Set-Clipboard",
                ])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| {
                    PicoError::Platform(format!("Failed to spawn clipboard setter: {}", e))
                })?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes()).map_err(|e| {
                    PicoError::Platform(format!("Failed to write to clipboard stdin: {}", e))
                })?;
            }

            let status = child.wait().map_err(|e| {
                PicoError::Platform(format!("Failed waiting for clipboard setter: {}", e))
            })?;

            if !status.success() {
                return Err(PicoError::Platform("Failed to set clipboard text".into()));
            }
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = text;
            Ok(())
        }
    }
}
