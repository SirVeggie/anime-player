//! Embedded mpv backend.
//!
//! mpv is hosted inside an **owned top-level popup window** that sits above
//! the Tauri main window in the desktop z-order. We use a popup (rather than
//! a `WS_CHILD` window inside the Tauri HWND) because WebView2 renders via
//! DirectComposition: a composited surface always paints on top of regular
//! GDI child windows in the same parent regardless of Win32 z-order. The
//! popup is "owned" by the main window so it follows minimize/restore and
//! gets destroyed when the owner closes.
//!
//! mpv runs as a child process (`mpv.exe --wid=<popup_hwnd>`) and we drive
//! it over its JSON IPC named pipe.

use std::ffi::OsStr;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SetWindowPos, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
};

const PIPE_NAME: &str = r"\\.\pipe\anime-player-mpv";

pub const MPV_NOT_FOUND_MSG: &str = "mpv was not found on your PATH. Install mpv from https://mpv.io (or via 'scoop install mpv' / 'choco install mpv') and restart the app.";

/// A rect in physical pixels relative to the parent window's client area.
#[derive(Clone, Copy, Debug)]
struct ClientRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

pub struct Mpv {
    parent_hwnd: HWND,
    popup_hwnd: HWND,
    process: Child,
    last_rect: Mutex<Option<ClientRect>>,
}

// HWND is a raw pointer; the Win32 handle is safe to send/share between
// threads because we only use it for thread-agnostic APIs.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn to_wstr(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn client_to_screen(parent: HWND, x: i32, y: i32) -> (i32, i32) {
    let mut pt = POINT { x, y };
    unsafe {
        ClientToScreen(parent, &mut pt);
    }
    (pt.x, pt.y)
}

impl Mpv {
    pub fn new(parent: HWND, x: i32, y: i32, w: i32, h: i32) -> Result<Self, String> {
        let mpv_path = which::which("mpv").map_err(|_| MPV_NOT_FOUND_MSG.to_string())?;

        let class = to_wstr("STATIC");
        let title = to_wstr("");

        let (sx, sy) = client_to_screen(parent, x, y);

        // Owned top-level popup. `parent` here is treated as the OWNER (not a
        // parent in the WS_CHILD sense) because `WS_CHILD` is not in the style.
        // `WS_EX_TOOLWINDOW` keeps it out of the Alt+Tab / taskbar lists,
        // `WS_EX_NOACTIVATE` prevents stealing focus on click (mpv still
        // receives mouse/keyboard events fine).
        let popup_hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                sx,
                sy,
                w.max(1),
                h.max(1),
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };

        if popup_hwnd.is_null() {
            return Err("Failed to create mpv host window".to_string());
        }

        let process = Command::new(&mpv_path)
            .arg(format!("--wid={}", popup_hwnd as isize))
            .arg("--idle=yes")
            .arg("--force-window=yes")
            .arg("--no-terminal")
            .arg("--keep-open=yes")
            .arg("--osc=yes")
            .arg("--input-default-bindings=yes")
            .arg("--input-vo-keyboard=yes")
            .arg(format!("--input-ipc-server={}", PIPE_NAME))
            .spawn()
            .map_err(|e| format!("Failed to launch mpv: {}", e))?;

        // Wait briefly for mpv to create its IPC pipe so the first command
        // (e.g. loadfile) doesn't race the spawn.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if std::fs::OpenOptions::new()
                .write(true)
                .open(PIPE_NAME)
                .is_ok()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Ok(Self {
            parent_hwnd: parent,
            popup_hwnd,
            process,
            last_rect: Mutex::new(Some(ClientRect { x, y, w, h })),
        })
    }

    fn send(&self, value: serde_json::Value) -> Result<(), String> {
        let mut pipe = std::fs::OpenOptions::new()
            .write(true)
            .open(PIPE_NAME)
            .map_err(|e| format!("mpv IPC pipe not available: {}", e))?;
        let mut line = value.to_string();
        line.push('\n');
        pipe.write_all(line.as_bytes())
            .map_err(|e| format!("mpv IPC write failed: {}", e))?;
        Ok(())
    }

    pub fn load_file(&self, path: &str) -> Result<(), String> {
        self.send(json!({ "command": ["loadfile", path] }))
    }

    pub fn play_pause(&self) -> Result<(), String> {
        self.send(json!({ "command": ["cycle", "pause"] }))
    }

    pub fn stop(&self) -> Result<(), String> {
        self.send(json!({ "command": ["stop"] }))
    }

    pub fn set_rect(&self, x: i32, y: i32, w: i32, h: i32) {
        if let Ok(mut guard) = self.last_rect.lock() {
            *guard = Some(ClientRect { x, y, w, h });
        }
        self.update_screen_position();
    }

    /// Re-apply the cached rect after the parent window has been moved or
    /// resized. The CSS rect inside the WebView didn't change, but its
    /// projection into screen coordinates did.
    pub fn refresh_position(&self) {
        self.update_screen_position();
    }

    fn update_screen_position(&self) {
        let rect = match self.last_rect.lock().ok().and_then(|g| *g) {
            Some(r) => r,
            None => return,
        };
        let (sx, sy) = client_to_screen(self.parent_hwnd, rect.x, rect.y);
        unsafe {
            SetWindowPos(
                self.popup_hwnd,
                std::ptr::null_mut(),
                sx,
                sy,
                rect.w.max(1),
                rect.h.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            );
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // Try a graceful quit first; if mpv doesn't exit promptly, kill it.
        let _ = self.send(json!({ "command": ["quit"] }));
        for _ in 0..30 {
            if matches!(self.process.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
