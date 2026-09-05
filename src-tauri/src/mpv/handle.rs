use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::AppHandle;

use super::event_loop::run_event_loop;
use super::ffi::{
    mpv_command, mpv_create, mpv_format, mpv_free_node_contents, mpv_get_property, mpv_initialize,
    mpv_set_option_string, mpv_terminate_destroy, mpv_wakeup, MpvNode,
};

/// In-process libmpv handle. Owns the mpv context plus the event-loop
/// thread that pumps property changes back into Tauri events.
pub struct MpvHandle {
    ctx: AtomicPtr<c_void>,
    event_loop_stop: std::sync::Arc<AtomicBool>,
    event_loop_handle: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MpvTrack {
    pub id: i64,
    pub kind: String,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
    pub external: bool,
    pub external_filename: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MpvVideoGeometry {
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MpvPlaybackEndState {
    pub time_pos: f64,
    pub duration: f64,
    pub eof_reached: bool,
    pub paused: bool,
}

// libmpv's API is documented as thread-safe (any thread can call mpv_*
// functions on the same context). Sending the raw pointer between
// threads is fine.
unsafe impl Send for MpvHandle {}
unsafe impl Sync for MpvHandle {}

fn cstring(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("string contained interior NUL byte: {s:?}"))
}

impl MpvHandle {
    /// Create a new libmpv context, embed it into the given Win32 HWND
    /// via the `wid` option, and start a background event loop that
    /// republishes property changes as Tauri events.
    ///
    /// `hwnd` must outlive the handle; in practice it's the Tauri main
    /// window and the handle is dropped on `WindowEvent::CloseRequested`.
    pub fn new(hwnd: usize, app_handle: AppHandle) -> Result<Self, String> {
        let ctx = unsafe { mpv_create() };
        if ctx.is_null() {
            return Err("mpv_create() returned null".to_string());
        }

        // The `wid` option must be set before mpv_initialize so libmpv's
        // d3d11 backend creates its DComp swap-chain under our HWND
        // during init.
        unsafe {
            set_option(ctx, "wid", &hwnd.to_string())?;
            // gpu-next is the modern default and uses gpu-context=d3d11
            // on Windows, which paints via DirectComposition. That's
            // what makes it composite cleanly with the (transparent)
            // Tauri WebView2 surface in the same DWM tree.
            set_option(ctx, "vo", "gpu-next")?;
            set_option(ctx, "gpu-context", "d3d11")?;
            set_option(ctx, "hwdec", "auto-safe")?;
            // Keep the last frame visible after EOF; let us decide what
            // to do (instead of mpv tearing down the window).
            set_option(ctx, "keep-open", "yes")?;
            set_option(ctx, "keep-open-pause", "yes")?;
            // We render our own HTML controls; mpv's OSC would draw on
            // top of them.
            set_option(ctx, "osc", "no")?;
            set_option(ctx, "input-default-bindings", "yes")?;
            set_option(ctx, "input-vo-keyboard", "yes")?;

            let init = mpv_initialize(ctx);
            if init < 0 {
                mpv_terminate_destroy(ctx);
                return Err(format!("mpv_initialize() failed: {init}"));
            }
        }

        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let ctx_addr = ctx as usize;
        let join = std::thread::spawn(move || {
            run_event_loop(ctx_addr as *mut c_void, app_handle, stop_clone);
        });

        Ok(Self {
            ctx: AtomicPtr::new(ctx),
            event_loop_stop: stop,
            event_loop_handle: Mutex::new(Some(join)),
        })
    }

    fn ctx(&self) -> Result<*mut c_void, String> {
        let ptr = self.ctx.load(Ordering::Acquire);
        if ptr.is_null() {
            Err("mpv context has been destroyed".to_string())
        } else {
            Ok(ptr)
        }
    }

    /// Run an mpv command. Args follow mpv's command syntax — e.g.
    /// `&["loadfile", path]`, `&["cycle", "pause"]`,
    /// `&["seek", "10", "relative"]`.
    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let ctx = self.ctx()?;
        let owned: Vec<CString> = args.iter().map(|s| cstring(s)).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe { mpv_command(ctx, ptrs.as_ptr()) };
        if rc < 0 {
            Err(format!("mpv command {args:?} failed: {rc}"))
        } else {
            Ok(())
        }
    }

    pub fn set_option_string(&self, name: &str, value: &str) -> Result<(), String> {
        let ctx = self.ctx()?;
        unsafe { set_option(ctx, name, value) }
    }

    pub fn load(&self, path: &str) -> Result<(), String> {
        self.command(&["loadfile", path])?;
        self.set_pause(false)?;
        Ok(())
    }

    pub fn cycle_pause(&self) -> Result<(), String> {
        self.command(&["cycle", "pause"])
    }

    pub fn set_pause(&self, paused: bool) -> Result<(), String> {
        let flag = if paused { "yes" } else { "no" };
        self.command(&["set", "pause", flag])
    }

    pub fn tracks(&self) -> Result<Vec<MpvTrack>, String> {
        let mut node = self.get_property_node("track-list")?;
        let tracks = unsafe { decode_track_list(&node) };
        unsafe {
            mpv_free_node_contents(&mut node);
        }
        Ok(tracks)
    }

    pub fn select_audio_track(&self, track_id: i64) -> Result<(), String> {
        self.command(&["set", "aid", &track_id.to_string()])
    }

    pub fn select_subtitle_track(&self, track_id: Option<i64>) -> Result<(), String> {
        let value = track_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "no".to_string());
        self.command(&["set", "sid", &value])
    }

    pub fn add_subtitle_file(&self, path: &str) -> Result<(), String> {
        self.command(&["sub-add", path, "select"])
    }

    pub fn video_geometry(&self) -> Result<Option<MpvVideoGeometry>, String> {
        let width = self
            .optional_property_i64("video-out-params/dw")
            .or_else(|| self.optional_property_i64("width"));
        let height = self
            .optional_property_i64("video-out-params/dh")
            .or_else(|| self.optional_property_i64("height"));

        match (width, height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                Ok(Some(MpvVideoGeometry { width, height }))
            }
            _ => Ok(None),
        }
    }

    /// Absolute seek in seconds.
    pub fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        let s = format!("{seconds}");
        self.command(&["seek", &s, "absolute"])
    }

    /// Fast absolute seek to the nearest keyframe (for scrub preview).
    pub fn seek_absolute_keyframes(&self, seconds: f64) -> Result<(), String> {
        let s = format!("{seconds}");
        self.command(&["seek", &s, "absolute+keyframes"])
    }

    /// Relative seek in seconds (may be negative).
    pub fn seek_relative(&self, seconds: f64) -> Result<(), String> {
        let s = format!("{seconds}");
        self.command(&["seek", &s, "relative"])
    }

    pub fn set_volume(&self, volume: f64) -> Result<(), String> {
        let v = format!("{volume}");
        self.command(&["set", "volume", &v])
    }

    pub fn stop(&self) -> Result<(), String> {
        self.unload()
    }

    /// Stops playback and clears the current file so the path is no longer open.
    pub fn unload(&self) -> Result<(), String> {
        self.command(&["stop"])
    }

    /// Absolute path of the file mpv currently has open, if any.
    pub fn loaded_path(&self) -> Option<String> {
        self.get_property_string("path")
            .ok()
            .filter(|path| !path.is_empty())
    }

    pub fn time_pos(&self) -> Result<f64, String> {
        self.get_property_f64("time-pos")
    }

    /// Snapshot of properties the UI uses to detect end-of-episode after seeks.
    pub fn playback_end_state(&self) -> Result<MpvPlaybackEndState, String> {
        Ok(MpvPlaybackEndState {
            time_pos: self.time_pos().unwrap_or(0.0),
            duration: self.get_property_f64("duration").unwrap_or(0.0),
            eof_reached: self.get_property_flag("eof-reached").unwrap_or(false),
            paused: self.get_property_flag("pause").unwrap_or(false),
        })
    }

    fn get_property_f64(&self, name: &str) -> Result<f64, String> {
        let ctx = self.ctx()?;
        let c_name = cstring(name)?;
        let mut value = 0.0_f64;
        let rc = unsafe {
            mpv_get_property(
                ctx,
                c_name.as_ptr(),
                mpv_format::MPV_FORMAT_DOUBLE,
                (&mut value as *mut f64).cast(),
            )
        };
        if rc < 0 {
            Err(format!("mpv_get_property({name}) failed: {rc}"))
        } else {
            Ok(value)
        }
    }

    fn get_property_flag(&self, name: &str) -> Result<bool, String> {
        let ctx = self.ctx()?;
        let c_name = cstring(name)?;
        let mut value = 0_i32;
        let rc = unsafe {
            mpv_get_property(
                ctx,
                c_name.as_ptr(),
                mpv_format::MPV_FORMAT_FLAG,
                (&mut value as *mut i32).cast(),
            )
        };
        if rc < 0 {
            Err(format!("mpv_get_property({name}) failed: {rc}"))
        } else {
            Ok(value != 0)
        }
    }

    fn get_property_node(&self, name: &str) -> Result<MpvNode, String> {
        let ctx = self.ctx()?;
        let c_name = cstring(name)?;
        let mut node = MpvNode {
            u: super::ffi::MpvNodeValue { int64: 0 },
            format: mpv_format::MPV_FORMAT_NONE,
        };
        let rc = unsafe {
            mpv_get_property(
                ctx,
                c_name.as_ptr(),
                mpv_format::MPV_FORMAT_NODE,
                (&mut node as *mut MpvNode).cast(),
            )
        };
        if rc < 0 {
            Err(format!("mpv_get_property({name}) failed: {rc}"))
        } else {
            Ok(node)
        }
    }

    /// Returns `None` when the property is missing or unavailable (e.g. before the first frame).
    fn optional_property_i64(&self, name: &str) -> Option<i64> {
        let mut node = self.get_property_node(name).ok()?;
        let value = unsafe { node_i64(&node) };
        unsafe {
            mpv_free_node_contents(&mut node);
        }
        value
    }

    fn get_property_string(&self, name: &str) -> Result<String, String> {
        let ctx = self.ctx()?;
        let c_name = cstring(name)?;
        let mut ptr: *const c_char = std::ptr::null();
        let rc = unsafe {
            mpv_get_property(
                ctx,
                c_name.as_ptr(),
                mpv_format::MPV_FORMAT_STRING,
                (&mut ptr as *mut *const c_char).cast(),
            )
        };
        if rc < 0 {
            return Err(format!("mpv_get_property({name}) failed: {rc}"));
        }
        if ptr.is_null() {
            return Ok(String::new());
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .map(str::to_owned)
            .map_err(|_| format!("mpv property {name} was not valid UTF-8"))
    }
}

unsafe fn decode_track_list(node: &MpvNode) -> Vec<MpvTrack> {
    if node.format != mpv_format::MPV_FORMAT_NODE_ARRAY {
        return Vec::new();
    }
    let list = unsafe { node.u.list };
    if list.is_null() {
        return Vec::new();
    }
    let list = unsafe { &*list };
    if list.values.is_null() || list.num <= 0 {
        return Vec::new();
    }

    (0..list.num)
        .filter_map(|idx| {
            let item = unsafe { &*list.values.add(idx as usize) };
            decode_track(item)
        })
        .collect()
}

unsafe fn decode_track(node: &MpvNode) -> Option<MpvTrack> {
    if node.format != mpv_format::MPV_FORMAT_NODE_MAP {
        return None;
    }
    let map = unsafe { node.u.list };
    if map.is_null() {
        return None;
    }
    let map = unsafe { &*map };
    if map.keys.is_null() || map.values.is_null() || map.num <= 0 {
        return None;
    }

    let mut id = None;
    let mut kind = None;
    let mut title = None;
    let mut lang = None;
    let mut selected = false;
    let mut external = false;
    let mut external_filename = None;

    for idx in 0..map.num {
        let key_ptr = unsafe { *map.keys.add(idx as usize) };
        if key_ptr.is_null() {
            continue;
        }
        let Ok(key) = (unsafe { CStr::from_ptr(key_ptr) }).to_str() else {
            continue;
        };
        let value = unsafe { &*map.values.add(idx as usize) };
        match key {
            "id" => id = node_i64(value),
            "type" => kind = node_string(value),
            "title" => title = node_string(value),
            "lang" => lang = node_string(value),
            "selected" => selected = node_bool(value).unwrap_or(false),
            "external" => external = node_bool(value).unwrap_or(false),
            "external-filename" => external_filename = node_string(value),
            _ => {}
        }
    }

    let kind = kind?;
    if kind != "audio" && kind != "sub" {
        return None;
    }

    Some(MpvTrack {
        id: id?,
        kind,
        title,
        lang,
        selected,
        external,
        external_filename,
    })
}

unsafe fn node_string(node: &MpvNode) -> Option<String> {
    if node.format != mpv_format::MPV_FORMAT_STRING {
        return None;
    }
    let ptr = unsafe { node.u.string };
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

unsafe fn node_i64(node: &MpvNode) -> Option<i64> {
    if node.format != mpv_format::MPV_FORMAT_INT64 {
        return None;
    }
    Some(unsafe { node.u.int64 })
}

unsafe fn node_bool(node: &MpvNode) -> Option<bool> {
    if node.format != mpv_format::MPV_FORMAT_FLAG {
        return None;
    }
    Some(unsafe { node.u.flag != 0 })
}

unsafe fn set_option(ctx: *mut c_void, name: &str, value: &str) -> Result<(), String> {
    let c_name = cstring(name)?;
    let c_value = cstring(value)?;
    let rc = unsafe { mpv_set_option_string(ctx, c_name.as_ptr(), c_value.as_ptr()) };
    if rc < 0 {
        Err(format!(
            "mpv_set_option_string({name}={value}) failed: {rc}"
        ))
    } else {
        Ok(())
    }
}

impl Drop for MpvHandle {
    fn drop(&mut self) {
        // Take ownership of the context first so the event loop can
        // observe a null pointer if it polls during teardown.
        let ctx = self.ctx.swap(std::ptr::null_mut(), Ordering::AcqRel);
        // Tell the event loop to bail and wake it from any pending
        // mpv_wait_event call.
        self.event_loop_stop.store(true, Ordering::SeqCst);
        if !ctx.is_null() {
            unsafe { mpv_wakeup(ctx) };
        }
        if let Ok(mut guard) = self.event_loop_handle.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
        if !ctx.is_null() {
            unsafe { mpv_terminate_destroy(ctx) };
        }
    }
}
