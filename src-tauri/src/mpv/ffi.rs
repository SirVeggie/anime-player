//! Minimal FFI declarations for libmpv 2.x. We only declare the symbols
//! we actually use; libmpv's full surface area is much larger.
//!
//! Reference: <https://github.com/mpv-player/mpv/blob/master/include/mpv/client.h>

#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum mpv_format {
    MPV_FORMAT_NONE = 0,
    MPV_FORMAT_STRING = 1,
    MPV_FORMAT_OSD_STRING = 2,
    MPV_FORMAT_FLAG = 3,
    MPV_FORMAT_INT64 = 4,
    MPV_FORMAT_DOUBLE = 5,
    MPV_FORMAT_NODE = 6,
    MPV_FORMAT_NODE_ARRAY = 7,
    MPV_FORMAT_NODE_MAP = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum mpv_event_id {
    MPV_EVENT_NONE = 0,
    MPV_EVENT_SHUTDOWN = 1,
    MPV_EVENT_LOG_MESSAGE = 2,
    MPV_EVENT_GET_PROPERTY_REPLY = 3,
    MPV_EVENT_SET_PROPERTY_REPLY = 4,
    MPV_EVENT_COMMAND_REPLY = 5,
    MPV_EVENT_START_FILE = 6,
    MPV_EVENT_END_FILE = 7,
    MPV_EVENT_FILE_LOADED = 8,
    MPV_EVENT_IDLE = 11,
    MPV_EVENT_TICK = 14,
    MPV_EVENT_CLIENT_MESSAGE = 16,
    MPV_EVENT_VIDEO_RECONFIG = 17,
    MPV_EVENT_AUDIO_RECONFIG = 18,
    MPV_EVENT_SEEK = 20,
    MPV_EVENT_PLAYBACK_RESTART = 21,
    MPV_EVENT_PROPERTY_CHANGE = 22,
    MPV_EVENT_QUEUE_OVERFLOW = 24,
    MPV_EVENT_HOOK = 25,
}

impl From<i32> for mpv_event_id {
    fn from(value: i32) -> Self {
        match value {
            1 => mpv_event_id::MPV_EVENT_SHUTDOWN,
            2 => mpv_event_id::MPV_EVENT_LOG_MESSAGE,
            3 => mpv_event_id::MPV_EVENT_GET_PROPERTY_REPLY,
            4 => mpv_event_id::MPV_EVENT_SET_PROPERTY_REPLY,
            5 => mpv_event_id::MPV_EVENT_COMMAND_REPLY,
            6 => mpv_event_id::MPV_EVENT_START_FILE,
            7 => mpv_event_id::MPV_EVENT_END_FILE,
            8 => mpv_event_id::MPV_EVENT_FILE_LOADED,
            11 => mpv_event_id::MPV_EVENT_IDLE,
            14 => mpv_event_id::MPV_EVENT_TICK,
            16 => mpv_event_id::MPV_EVENT_CLIENT_MESSAGE,
            17 => mpv_event_id::MPV_EVENT_VIDEO_RECONFIG,
            18 => mpv_event_id::MPV_EVENT_AUDIO_RECONFIG,
            20 => mpv_event_id::MPV_EVENT_SEEK,
            21 => mpv_event_id::MPV_EVENT_PLAYBACK_RESTART,
            22 => mpv_event_id::MPV_EVENT_PROPERTY_CHANGE,
            24 => mpv_event_id::MPV_EVENT_QUEUE_OVERFLOW,
            25 => mpv_event_id::MPV_EVENT_HOOK,
            _ => mpv_event_id::MPV_EVENT_NONE,
        }
    }
}

#[repr(C)]
pub struct MpvEvent {
    pub event_id: mpv_event_id,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: mpv_format,
    pub data: *mut c_void,
}

#[link(name = "mpv")]
unsafe extern "C" {
    pub fn mpv_create() -> *mut c_void;
    pub fn mpv_initialize(ctx: *mut c_void) -> c_int;
    pub fn mpv_terminate_destroy(ctx: *mut c_void);
    pub fn mpv_command(ctx: *mut c_void, args: *const *const c_char) -> c_int;
    pub fn mpv_set_option_string(
        ctx: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub fn mpv_observe_property(
        ctx: *mut c_void,
        reply_userdata: u64,
        name: *const c_char,
        format: mpv_format,
    ) -> c_int;
    pub fn mpv_wait_event(ctx: *mut c_void, timeout: f64) -> *mut MpvEvent;
    pub fn mpv_wakeup(ctx: *mut c_void);
    pub fn mpv_get_property_string(ctx: *mut c_void, name: *const c_char) -> *mut c_char;
    pub fn mpv_free(data: *mut c_void);
}
