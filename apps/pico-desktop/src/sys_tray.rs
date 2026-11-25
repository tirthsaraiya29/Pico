use crate::state::CompanionNotification;

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ToggleVisible,
    ToggleCompact,
    ExitApp,
}

#[cfg(target_os = "windows")]
pub mod platform_tray {
    use super::*;
    use std::sync::mpsc::Sender;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
    };
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, PostMessageW,
        PostQuitMessage, RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
        CS_HREDRAW, CS_VREDRAW, HICON, ICONINFO, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        WM_APP, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
        WS_OVERLAPPED,
    };

    const WM_TRAY_ICON: u32 = WM_APP + 1;
    const WM_SHOW_BALLOON: u32 = WM_APP + 2;
    const HOTKEY_ID: i32 = 1001;
    const CMD_TOGGLE: usize = 2001;
    const CMD_COMPACT: usize = 2002;
    const CMD_EXIT: usize = 2003;

    pub struct TrayHandle {
        hwnd_raw: isize,
    }

    impl TrayHandle {
        pub fn notify(&self, n: CompanionNotification) {
            let boxed = Box::into_raw(Box::new(n));
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd_raw), WM_SHOW_BALLOON, WPARAM(0), LPARAM(boxed as isize));
            }
        }
    }

    impl Drop for TrayHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd_raw), WM_DESTROY, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn utf16_null(s: &str) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    fn copy_wide_fixed<const N: usize>(dst: &mut [u16; N], s: &str) {
        dst.fill(0);
        for (slot, ch) in dst.iter_mut().take(N.saturating_sub(1)).zip(s.encode_utf16()) {
            *slot = ch;
        }
    }

    fn build_tray_icon() -> HICON {
        // 16x16 geometric dark slate & teal glyph rendered in memory without needing external .ico files.
        let mut pixels = [0u32; 16 * 16];
        for y in 0..16i32 {
            for x in 0..16i32 {
                let dx = x as f32 - 7.5;
                let dy = y as f32 - 7.5;
                let dist = (dx * dx + dy * dy).sqrt();
                let color = if dist <= 4.2 {
                    0xFF4FB3FFu32
                } else if dist <= 7.2 {
                    0xFF171C28u32
                } else {
                    0x00000000u32
                };
                pixels[(y * 16 + x) as usize] = color;
            }
        }
        unsafe {
            let color_bmp = CreateBitmap(16, 16, 1, 32, Some(pixels.as_ptr().cast()));
            let mask_bmp = CreateBitmap(16, 16, 1, 1, None);
            let dc = CreateCompatibleDC(None);
            let info = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask_bmp,
                hbmColor: color_bmp,
            };
            let icon = CreateIconIndirect(&info).unwrap_or_default();
            let _ = DeleteObject(color_bmp);
            let _ = DeleteObject(mask_bmp);
            let _ = DeleteDC(dc);
            icon
        }
    }

    type WakeCallback = Box<dyn Fn() + Send + Sync>;
    static TRAY_SENDER: std::sync::OnceLock<std::sync::Mutex<Option<Sender<TrayCommand>>>> =
        std::sync::OnceLock::new();
    static HOTKEY_CALLBACK: std::sync::OnceLock<std::sync::Mutex<Option<WakeCallback>>> =
        std::sync::OnceLock::new();

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
                if let Some(mtx) = HOTKEY_CALLBACK.get() {
                    if let Ok(guard) = mtx.lock() {
                        if let Some(ref cb) = *guard {
                            cb();
                        }
                    }
                }
                if let Some(mtx) = TRAY_SENDER.get() {
                    if let Ok(guard) = mtx.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(TrayCommand::ToggleVisible);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_TRAY_ICON => {
                let mouse_msg = (lparam.0 & 0xFFFF) as u32;
                if mouse_msg == WM_LBUTTONUP {
                    if let Some(mtx) = HOTKEY_CALLBACK.get() {
                        if let Ok(guard) = mtx.lock() {
                            if let Some(ref cb) = *guard {
                                cb();
                            }
                        }
                    }
                    if let Some(mtx) = TRAY_SENDER.get() {
                        if let Ok(guard) = mtx.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(TrayCommand::ToggleVisible);
                            }
                        }
                    }
                } else if mouse_msg == WM_RBUTTONUP {
                    let Ok(menu) = CreatePopupMenu() else {
                        return LRESULT(0);
                    };
                    let lbl_show = utf16_null("Show / Hide Companion");
                    let lbl_compact = utf16_null("Toggle Compact Pill");
                    let lbl_exit = utf16_null("Exit Pico");
                    let _ = AppendMenuW(menu, MF_STRING, CMD_TOGGLE, PCWSTR(lbl_show.as_ptr()));
                    let _ = AppendMenuW(menu, MF_STRING, CMD_COMPACT, PCWSTR(lbl_compact.as_ptr()));
                    let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, PCWSTR(lbl_exit.as_ptr()));

                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
                    let _ = DestroyMenu(menu);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let cmd = wparam.0 & 0xFFFF;
                let event = match cmd {
                    CMD_TOGGLE => Some(TrayCommand::ToggleVisible),
                    CMD_COMPACT => Some(TrayCommand::ToggleCompact),
                    CMD_EXIT => Some(TrayCommand::ExitApp),
                    _ => None,
                };
                if let Some(ev) = event {
                    if let Some(mtx) = HOTKEY_CALLBACK.get() {
                        if let Ok(guard) = mtx.lock() {
                            if let Some(ref cb) = *guard {
                                cb();
                            }
                        }
                    }
                    if let Some(mtx) = TRAY_SENDER.get() {
                        if let Ok(guard) = mtx.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(ev);
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_SHOW_BALLOON => {
                if lparam.0 != 0 {
                    let notif = Box::from_raw(lparam.0 as *mut CompanionNotification);
                    let mut nid = NOTIFYICONDATAW {
                        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                        hWnd: hwnd,
                        uID: 1,
                        uFlags: NIF_INFO,
                        dwInfoFlags: NIIF_INFO,
                        ..Default::default()
                    };
                    copy_wide_fixed(&mut nid.szInfoTitle, &notif.title);
                    copy_wide_fixed(&mut nid.szInfo, &notif.body);
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
                let nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    pub fn spawn_tray_and_hotkey<F>(tx: Sender<TrayCommand>, wake_ui: F) -> Option<TrayHandle>
    where
        F: Fn() + Send + Sync + 'static,
    {
        *TRAY_SENDER
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap() = Some(tx);
        *HOTKEY_CALLBACK
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap() = Some(Box::new(wake_ui));

        let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("pico-tray-win32".into())
            .spawn(move || unsafe {
                let hmod = GetModuleHandleW(None).unwrap_or_default();
                let cls_name = utf16_null("PicoDesktopTrayWindowClass");
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hmod.into(),
                    lpszClassName: PCWSTR(cls_name.as_ptr()),
                    ..Default::default()
                };
                RegisterClassW(&wc);

                let win_name = utf16_null("PicoTrayMsg");
                let hwnd = CreateWindowExW(
                    Default::default(),
                    PCWSTR(cls_name.as_ptr()),
                    PCWSTR(win_name.as_ptr()),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    hmod,
                    None,
                );
                if hwnd.0 == 0 {
                    let _ = hwnd_tx.send(0);
                    return;
                }

                let icon = build_tray_icon();
                let mut nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                    uCallbackMessage: WM_TRAY_ICON,
                    hIcon: icon,
                    ..Default::default()
                };
                copy_wide_fixed(&mut nid.szTip, "Pico Desktop Companion (Alt+Space)");
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);

                // Register Alt+Space global toggle; fall back to Ctrl+Shift+P if already bound.
                if RegisterHotKey(hwnd, HOTKEY_ID, MOD_ALT, 0x20).is_err() {
                    let _ = RegisterHotKey(hwnd, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x50);
                }

                let _ = hwnd_tx.send(hwnd.0);

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if !icon.is_invalid() {
                    let _ = DestroyIcon(icon);
                }
            })
            .ok()?;

        let hwnd_raw = hwnd_rx.recv_timeout(std::time::Duration::from_secs(2)).ok()?;
        if hwnd_raw == 0 {
            None
        } else {
            Some(TrayHandle { hwnd_raw })
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod platform_tray {
    use super::*;
    use std::sync::mpsc::Sender;

    pub struct TrayHandle;

    impl TrayHandle {
        pub fn notify(&self, n: CompanionNotification) {
            let _ = std::process::Command::new("notify-send")
                .args(["-a", "Pico", &n.title, &n.body])
                .spawn();
        }
    }

    pub fn spawn_tray_and_hotkey<F>(_tx: Sender<TrayCommand>, _wake_ui: F) -> Option<TrayHandle>
    where
        F: Fn() + Send + Sync + 'static,
    {
        Some(TrayHandle)
    }
}
