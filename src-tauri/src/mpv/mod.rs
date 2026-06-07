//! Embedded mpv backend using libmpv via FFI.
//!
//! mpv runs **in-process** as `libmpv-2.dll` (loaded via `mpv.lib` import
//! library — see `src-tauri/build.rs`). We pass the Tauri main window's
//! `HWND` to libmpv via the `wid` option and let libmpv's `gpu-next` /
//! `gpu-context=d3d11` backend create its own DirectComposition swap
//! chain under that HWND. Combined with `transparent: true` on the Tauri
//! window, the WebView2 visual and mpv's swap chain compose together in
//! the same DWM tree, so the player appears as a single window to the OS
//! (one PID, one HWND — useful for audio mixers and window-mover scripts).
//!
//! This is fundamentally different from the previous popup-window
//! approach: there is no separate `WS_POPUP` HWND, no `mpv.exe`
//! subprocess, and no JSON-IPC named pipe. All commands flow through
//! `mpv_command`, all state flows back through `mpv_observe_property` /
//! `mpv_wait_event` on a dedicated event-loop thread.

mod event_loop;
mod ffi;
mod handle;

pub use handle::{MpvHandle, MpvPlaybackEndState, MpvTrack, MpvVideoGeometry};

fn windows_path_key(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

/// Release mpv's handle when it has one of the given paths open (e.g. before a rename).
pub fn unload_if_loading_any_of(mpv: Option<&MpvHandle>, paths: &[String]) -> Result<(), String> {
    let Some(handle) = mpv else {
        return Ok(());
    };
    let Some(loaded) = handle.loaded_path() else {
        return Ok(());
    };
    let loaded_key = windows_path_key(&loaded);
    if paths
        .iter()
        .any(|path| windows_path_key(path) == loaded_key)
    {
        handle.unload()?;
    }
    Ok(())
}
