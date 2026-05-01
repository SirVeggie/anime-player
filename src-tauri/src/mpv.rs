//! Embedded mpv backend.
//!
//! mpv is hosted in a `WS_CHILD` window of the Tauri main HWND, z-ordered
//! beneath the WebView2 child HWND. The Tauri window is configured with
//! `transparent: true`, which puts the whole top-level into DWM/DComp
//! compositing mode. WebView2's DComp surface then alpha-blends against
//! its sibling children, so a transparent CSS region in the player pane
//! reveals the mpv child window underneath.
//!
//! An earlier attempt used `WS_CHILD` against an opaque top-level window;
//! that produced a black video pane because, in non-compositing mode,
//! WebView2's DComp surface always paints over GDI siblings regardless of
//! z-order. Transparency is the missing ingredient.
//!
//! mpv runs as a child process (`mpv.exe --wid=<hwnd>`) and we drive it
//! over its JSON IPC named pipe.

use std::ffi::OsStr;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::json;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SetWindowPos, HWND_BOTTOM, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

const PIPE_NAME: &str = r"\\.\pipe\anime-player-mpv";

pub const MPV_NOT_FOUND_MSG: &str = "mpv was not found on your PATH. Install mpv from https://mpv.io (or via 'scoop install mpv' / 'choco install mpv') and restart the app.";

pub struct Mpv {
    hwnd: HWND,
    process: Child,
}

// HWND is a raw pointer; the Win32 handle is safe to send/share between
// threads because we only use it for thread-agnostic APIs.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn to_wstr(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

impl Mpv {
    pub fn new(parent: HWND, x: i32, y: i32, w: i32, h: i32) -> Result<Self, String> {
        let mpv_path = which::which("mpv").map_err(|_| MPV_NOT_FOUND_MSG.to_string())?;

        let class = to_wstr("STATIC");
        let title = to_wstr("");

        // WS_CHILD with the Tauri main HWND as the actual parent. Coordinates
        // are in the parent's client area. WS_CLIPSIBLINGS so the mpv window
        // does not paint over the WebView2 sibling above it in z-order.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                x,
                y,
                w.max(1),
                h.max(1),
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };

        if hwnd.is_null() {
            return Err("Failed to create mpv host window".to_string());
        }

        // Push behind WebView2 so its DComp surface composites on top of us.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        let process = Command::new(&mpv_path)
            .arg(format!("--wid={}", hwnd as isize))
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

        Ok(Self { hwnd, process })
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
        // Coordinates are in the parent's client area (physical pixels).
        // Children follow the parent on move/resize automatically, so we
        // only need to react to layout changes inside the page.
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x,
                y,
                w.max(1),
                h.max(1),
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
