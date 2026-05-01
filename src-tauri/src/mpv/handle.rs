use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;

use tauri::AppHandle;

use super::event_loop::run_event_loop;
use super::ffi::{
    mpv_command, mpv_create, mpv_initialize, mpv_set_option_string, mpv_terminate_destroy,
    mpv_wakeup,
};

/// In-process libmpv handle. Owns the mpv context plus the event-loop
/// thread that pumps property changes back into Tauri events.
pub struct MpvHandle {
    ctx: AtomicPtr<c_void>,
    event_loop_stop: std::sync::Arc<AtomicBool>,
    event_loop_handle: Mutex<Option<JoinHandle<()>>>,
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

    /// Absolute seek in seconds.
    pub fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        let s = format!("{seconds}");
        self.command(&["seek", &s, "absolute"])
    }

    /// Relative seek in seconds (may be negative).
    pub fn seek_relative(&self, seconds: f64) -> Result<(), String> {
        let s = format!("{seconds}");
        self.command(&["seek", &s, "relative"])
    }

    pub fn stop(&self) -> Result<(), String> {
        self.command(&["stop"])
    }
}

unsafe fn set_option(ctx: *mut c_void, name: &str, value: &str) -> Result<(), String> {
    let c_name = cstring(name)?;
    let c_value = cstring(value)?;
    let rc = unsafe { mpv_set_option_string(ctx, c_name.as_ptr(), c_value.as_ptr()) };
    if rc < 0 {
        Err(format!("mpv_set_option_string({name}={value}) failed: {rc}"))
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
