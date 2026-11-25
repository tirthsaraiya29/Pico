#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use pico_core::error::PicoError;
use pico_core::error::Result;

pub struct PlatformClipboard;

impl PlatformClipboard {
    /// Gets current text content from the system clipboard.
    pub fn get_text() -> Result<String> {
        #[cfg(target_os = "windows")]
        {
            win_clip::get_text()
        }
        #[cfg(target_os = "linux")]
        {
            linux_clip::get_text()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(PicoError::Platform(
                "Clipboard unsupported on this OS".into(),
            ))
        }
    }

    /// Sets text content into the system clipboard.
    pub fn set_text(text: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            win_clip::set_text(text)
        }
        #[cfg(target_os = "linux")]
        {
            linux_clip::set_text(text)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = text;
            Err(PicoError::Platform(
                "Clipboard unsupported on this OS".into(),
            ))
        }
    }
}

#[cfg(target_os = "windows")]
mod win_clip {
    use pico_core::error::{PicoError, Result};
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT_U32: u32 = 13;

    struct ClipboardLock;
    impl ClipboardLock {
        fn open() -> Result<Self> {
            for _ in 0..8 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Self);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(PicoError::Platform(
                "Unable to acquire system clipboard lock".into(),
            ))
        }
    }
    impl Drop for ClipboardLock {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    pub fn get_text() -> Result<String> {
        let _lock = ClipboardLock::open()?;
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT_U32).is_err() {
                return Ok(String::new());
            }
            let handle = GetClipboardData(CF_UNICODETEXT_U32)
                .map_err(|e| PicoError::Platform(format!("GetClipboardData failed: {}", e)))?;
            let hglobal = HGLOBAL(handle.0 as *mut _);
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return Err(PicoError::Platform(
                    "Clipboard GlobalLock returned null".into(),
                ));
            }
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(hglobal);
            Ok(text)
        }
    }

    pub fn set_text(text: &str) -> Result<()> {
        let _lock = ClipboardLock::open()?;
        unsafe {
            EmptyClipboard()
                .map_err(|e| PicoError::Platform(format!("EmptyClipboard failed: {}", e)))?;
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let byte_len = wide.len() * std::mem::size_of::<u16>();
            let hglobal = GlobalAlloc(GMEM_MOVEABLE, byte_len)
                .map_err(|e| PicoError::Platform(format!("Clipboard GlobalAlloc failed: {}", e)))?;
            let dst = GlobalLock(hglobal) as *mut u16;
            if dst.is_null() {
                return Err(PicoError::Platform(
                    "Clipboard GlobalLock returned null".into(),
                ));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            let _ = GlobalUnlock(hglobal);
            SetClipboardData(CF_UNICODETEXT_U32, HANDLE(hglobal.0 as isize))
                .map_err(|e| PicoError::Platform(format!("SetClipboardData failed: {}", e)))?;
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_clip {
    use pico_core::error::{PicoError, Result};
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub fn get_text() -> Result<String> {
        if let Ok(out) = Command::new("wl-paste").arg("-n").output() {
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).to_string());
            }
        }
        if let Ok(out) = Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
        {
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).to_string());
            }
        }
        if let Ok(out) = Command::new("xsel")
            .args(["--clipboard", "--output"])
            .output()
        {
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).to_string());
            }
        }
        Err(PicoError::Platform(
            "No clipboard provider found (install wl-clipboard, xclip, or xsel)".into(),
        ))
    }

    pub fn set_text(text: &str) -> Result<()> {
        let providers: [(&str, &[&str]); 3] = [
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        for (bin, args) in providers {
            if let Ok(mut child) = Command::new(bin)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(text.as_bytes()).map_err(|e| {
                        PicoError::Platform(format!("Failed writing clipboard stdin: {}", e))
                    })?;
                }
                if child.wait().map(|s| s.success()).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        Err(PicoError::Platform(
            "No clipboard provider found (install wl-clipboard, xclip, or xsel)".into(),
        ))
    }
}
