use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{State, Window};
use walkdir::WalkDir;

#[cfg(windows)]
mod mpv;

#[cfg(windows)]
use mpv::Mpv;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "wmv", "flv", "webm", "ts", "m2ts", "mts", "ogv", "ogm",
    "vob", "3gp", "rm", "rmvb", "mpg", "mpeg",
];

#[derive(Debug, Serialize)]
struct VideoFile {
    /// Absolute path on disk (used by the asset protocol to load the file).
    path: String,
    /// File name with extension.
    name: String,
    /// Path relative to the scanned root, using forward slashes.
    relative_path: String,
    /// File size in bytes.
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
    mpv: Mutex<Option<Mpv>>,
}

#[cfg(windows)]
fn css_rect_to_physical(window: &Window, x: f64, y: f64, w: f64, h: f64) -> (i32, i32, i32, i32) {
    let scale = window.scale_factor().unwrap_or(1.0);
    (
        (x * scale).round() as i32,
        (y * scale).round() as i32,
        (w * scale).round() as i32,
        (h * scale).round() as i32,
    )
}

#[cfg(windows)]
#[tauri::command]
fn mpv_init(
    state: State<'_, AppState>,
    window: Window,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let (px, py, pw, ph) = css_rect_to_physical(&window, x, y, width, height);
    let parent_hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as windows_sys::Win32::Foundation::HWND;

    let mut guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = guard.as_ref() {
        existing.set_rect(px, py, pw, ph);
        return Ok(());
    }
    let m = Mpv::new(parent_hwnd, px, py, pw, ph)?;
    *guard = Some(m);
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_set_rect(
    state: State<'_, AppState>,
    window: Window,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let (px, py, pw, ph) = css_rect_to_physical(&window, x, y, width, height);
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.set_rect(px, py, pw, ph);
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_load(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.load_file(&path)
}

#[cfg(windows)]
#[tauri::command]
fn mpv_play_pause(state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.play_pause()?;
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
            use tauri::Manager;
            app.manage(AppState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_videos,
            mpv_init,
            mpv_set_rect,
            mpv_load,
            mpv_play_pause,
            mpv_stop,
        ]);

    #[cfg(not(windows))]
    let builder = builder.invoke_handler(tauri::generate_handler![scan_videos]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
