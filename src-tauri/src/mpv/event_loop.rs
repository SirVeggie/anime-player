//! Background thread that drains libmpv's event queue and republishes
//! the interesting bits as Tauri events.
//!
//! We observe a small set of properties that the React UI needs to drive
//! its custom controls bar:
//!
//! - `time-pos`     — current playback position in seconds.
//! - `duration`     — total duration of the loaded file in seconds.
//! - `pause`        — current pause flag.
//! - `eof-reached`  — true when playback has finished.
//!
//! Each property change is fanned out as `mpv://<name>` so the frontend
//! can `listen("mpv://time-pos", ...)`.

use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::ffi::{
    mpv_event_id, mpv_format, mpv_observe_property, mpv_wait_event, MpvEventProperty,
};

#[derive(Serialize, Clone, Debug)]
#[serde(untagged)]
enum PropPayload {
    Bool(bool),
    Float(f64),
    Null,
}

const TIME_POS_ID: u64 = 1;
const DURATION_ID: u64 = 2;
const PAUSE_ID: u64 = 3;
const EOF_REACHED_ID: u64 = 4;

fn observe(ctx: *mut c_void, id: u64, name: &str, format: mpv_format) {
    let Ok(c_name) = CString::new(name) else {
        return;
    };
    unsafe {
        mpv_observe_property(ctx, id, c_name.as_ptr(), format);
    }
}

pub fn run_event_loop(ctx: *mut c_void, app_handle: AppHandle, stop: Arc<AtomicBool>) {
    if ctx.is_null() {
        return;
    }

    observe(ctx, TIME_POS_ID, "time-pos", mpv_format::MPV_FORMAT_DOUBLE);
    observe(ctx, DURATION_ID, "duration", mpv_format::MPV_FORMAT_DOUBLE);
    observe(ctx, PAUSE_ID, "pause", mpv_format::MPV_FORMAT_FLAG);
    observe(
        ctx,
        EOF_REACHED_ID,
        "eof-reached",
        mpv_format::MPV_FORMAT_FLAG,
    );

    while !stop.load(Ordering::Acquire) {
        // -1.0 == block forever (until mpv_wakeup or an event arrives);
        // we rely on mpv_wakeup() in MpvHandle::drop to break out.
        let event_ptr = unsafe { mpv_wait_event(ctx, -1.0) };
        if event_ptr.is_null() {
            continue;
        }
        let event = unsafe { &*event_ptr };
        match event.event_id {
            mpv_event_id::MPV_EVENT_NONE => continue,
            mpv_event_id::MPV_EVENT_SHUTDOWN => break,
            mpv_event_id::MPV_EVENT_PROPERTY_CHANGE => {
                handle_property_change(&app_handle, event.reply_userdata, event.data);
            }
            mpv_event_id::MPV_EVENT_END_FILE => {
                let _ = app_handle.emit("mpv://end-file", ());
            }
            mpv_event_id::MPV_EVENT_FILE_LOADED => {
                let _ = app_handle.emit("mpv://file-loaded", ());
            }
            _ => {}
        }
    }
}

fn handle_property_change(app_handle: &AppHandle, id: u64, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    let prop = unsafe { &*(data as *const MpvEventProperty) };
    let name = if prop.name.is_null() {
        return;
    } else {
        match unsafe { CStr::from_ptr(prop.name) }.to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let payload = decode_property(prop);
    let event_name = format!("mpv://{name}");
    let _ = app_handle.emit(event_name.as_str(), payload);
    // id is unused but kept around so future code can verify the
    // observation matches the property name we expected.
    let _ = id;
}

fn decode_property(prop: &MpvEventProperty) -> PropPayload {
    if prop.data.is_null() {
        return PropPayload::Null;
    }
    match prop.format {
        mpv_format::MPV_FORMAT_FLAG => {
            let value = unsafe { *(prop.data as *const i32) };
            PropPayload::Bool(value != 0)
        }
        mpv_format::MPV_FORMAT_DOUBLE => {
            let value = unsafe { *(prop.data as *const f64) };
            PropPayload::Float(value)
        }
        mpv_format::MPV_FORMAT_INT64 => {
            let value = unsafe { *(prop.data as *const i64) };
            PropPayload::Float(value as f64)
        }
        _ => PropPayload::Null,
    }
}
