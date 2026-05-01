use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Manager, State};
use walkdir::WalkDir;

#[cfg(windows)]
mod mpv;

#[cfg(windows)]
use mpv::MpvHandle;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "wmv", "flv", "webm", "ts", "m2ts", "mts", "ogv", "ogm",
    "vob", "3gp", "rm", "rmvb", "mpg", "mpeg",
];

#[derive(Debug, Serialize)]
struct VideoFile {
    path: String,
    name: String,
    /// Path relative to the scanned root, using forward slashes.
    relative_path: String,
    size: u64,
}

fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            VIDEO_EXTENSIONS.iter().any(|allowed| *allowed == lower)
        })
        .unwrap_or(false)
}

#[tauri::command]
fn scan_videos(folder: String) -> Result<Vec<VideoFile>, String> {
    let root = Path::new(&folder);
    if !root.exists() {
        return Err(format!("Folder does not exist: {}", folder));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {}", folder));
    }

    let mut results: Vec<VideoFile> = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if !is_video_file(path) {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        results.push(VideoFile {
            path: path.to_string_lossy().to_string(),
            name,
            relative_path: relative,
            size,
        });
    }

    results.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });

    Ok(results)
}

#[cfg(windows)]
#[derive(Default)]
struct AppState {
    mpv: Mutex<Option<MpvHandle>>,
}

/// Re-compute the `video-margin-ratio-left` for the current sidebar
/// width. The Tauri main HWND covers the whole client area; libmpv
/// renders into the same canvas. To keep the video confined to the
/// right pane (the sidebar is ~360px on the left) we tell mpv to leave
/// that fraction of its canvas empty on the left. The opaque sidebar
/// then visually covers the empty strip.
#[cfg(windows)]
fn apply_layout_to_mpv(mpv: &MpvHandle, window_width: f64, sidebar_px: f64) -> Result<(), String> {
    let ratio = if window_width > 0.0 {
        (sidebar_px / window_width).clamp(0.0, 0.95)
    } else {
        0.0
    };
    mpv.set_option_string("video-margin-ratio-left", &format!("{ratio:.6}"))
}

#[cfg(windows)]
#[tauri::command]
fn mpv_init(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    window: tauri::Window,
    window_width: f64,
    sidebar_px: f64,
) -> Result<(), String> {
    let mut guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = guard.as_ref() {
        apply_layout_to_mpv(existing, window_width, sidebar_px)?;
        return Ok(());
    }

    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let handle = MpvHandle::new(hwnd, app)?;
    apply_layout_to_mpv(&handle, window_width, sidebar_px)?;
    *guard = Some(handle);
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_load(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.load(&path)
}

#[cfg(windows)]
#[tauri::command]
fn mpv_cycle_pause(state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.cycle_pause()?;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_seek(state: State<'_, AppState>, seconds: f64) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.seek_absolute(seconds)?;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_set_layout(
    state: State<'_, AppState>,
    window_width: f64,
    sidebar_px: f64,
) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        apply_layout_to_mpv(m, window_width, sidebar_px)?;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_stop(state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.stop()?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(windows)]
    let builder = builder
        .setup(|app| {
            app.manage(AppState::default());

            // Tear libmpv down before the main window's HWND becomes
            // invalid. Without this, the event-loop thread can outlive
            // the HWND it was rendering into and we get spurious GPU
            // errors on close.
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        if let Some(state) = app_handle.try_state::<AppState>() {
                            if let Ok(mut guard) = state.mpv.lock() {
                                guard.take();
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_videos,
            mpv_init,
            mpv_load,
            mpv_cycle_pause,
            mpv_seek,
            mpv_set_layout,
            mpv_stop,
        ]);

    #[cfg(not(windows))]
    let builder = builder.invoke_handler(tauri::generate_handler![scan_videos]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
