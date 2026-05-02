use std::sync::Mutex;

use tauri::{Manager, State};
use tauri_plugin_deep_link::DeepLinkExt;

mod anilist;
mod db;
mod library;
mod scanner;

#[cfg(windows)]
mod mpv;
#[cfg(windows)]
mod thumbnails;

#[cfg(windows)]
use mpv::MpvHandle;

/// Default sidebar width in CSS pixels. Mirrors `SIDEBAR_PX` in
/// `src/App.tsx`. Used by the native resize handler before the frontend
/// has had a chance to register its own value via `mpv_init`.
#[cfg(windows)]
const DEFAULT_SIDEBAR_PX: f64 = 360.0;

#[cfg(windows)]
struct AppState {
    mpv: Mutex<Option<MpvHandle>>,
    /// Last sidebar width (CSS px) the frontend asked us to apply. Read
    /// by the native resize handler so it can re-issue
    /// `video-margin-ratio-left` on every WM_SIZE without needing a JS
    /// round-trip per frame.
    sidebar_px: Mutex<f64>,
}

#[cfg(windows)]
impl Default for AppState {
    fn default() -> Self {
        Self {
            mpv: Mutex::new(None),
            sidebar_px: Mutex::new(DEFAULT_SIDEBAR_PX),
        }
    }
}

/// Re-compute the `video-margin-ratio-left` for the current sidebar
/// width. The Tauri main HWND covers the whole client area; libmpv
/// renders into the same canvas. To keep the video confined to the
/// right pane (or pushed fully off-screen while hidden) we tell mpv to
/// leave that fraction of its canvas empty on the left. The opaque
/// sidebar then visually covers the empty strip.
#[cfg(windows)]
fn apply_layout_to_mpv(mpv: &MpvHandle, window_width: f64, sidebar_px: f64) -> Result<(), String> {
    let ratio = if window_width > 0.0 {
        (sidebar_px / window_width).clamp(0.0, 1.0)
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
    if let Ok(mut g) = state.sidebar_px.lock() {
        *g = sidebar_px;
    }

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
fn mpv_set_pause(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.set_pause(paused)?;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn mpv_get_tracks(state: State<'_, AppState>) -> Result<Vec<mpv::MpvTrack>, String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.tracks()
}

#[cfg(windows)]
#[tauri::command]
fn mpv_select_audio_track(state: State<'_, AppState>, track_id: i64) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.select_audio_track(track_id)
}

#[cfg(windows)]
#[tauri::command]
fn mpv_select_subtitle_track(
    state: State<'_, AppState>,
    track_id: Option<i64>,
) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.select_subtitle_track(track_id)
}

#[cfg(windows)]
#[tauri::command]
fn mpv_get_video_geometry(
    state: State<'_, AppState>,
) -> Result<Option<mpv::MpvVideoGeometry>, String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let m = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    m.video_geometry()
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
fn mpv_seek_relative(state: State<'_, AppState>, delta: f64) -> Result<(), String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        m.seek_relative(delta)?;
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
    if let Ok(mut g) = state.sidebar_px.lock() {
        *g = sidebar_px;
    }
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        apply_layout_to_mpv(m, window_width, sidebar_px)?;
    }
    Ok(())
}

/// Native `WindowEvent::Resized` / `ScaleFactorChanged` hook. Called on
/// the Tauri UI thread on every WM_SIZE. We update mpv's
/// `video-margin-ratio-left` in-process here instead of from a
/// JS `resize` listener; the JS path goes through `invoke()` which
/// adds an IPC round-trip per frame and noticeably degrades resize
/// smoothness while a video is playing.
#[cfg(windows)]
fn handle_native_resize(app_handle: &tauri::AppHandle, physical_width: u32, scale_factor: f64) {
    let Some(state) = app_handle.try_state::<AppState>() else {
        return;
    };
    let Ok(guard) = state.mpv.lock() else {
        return;
    };
    let Some(m) = guard.as_ref() else {
        return;
    };
    let logical_width = (physical_width as f64 / scale_factor.max(0.01)).max(1.0);
    let sidebar_px = state
        .sidebar_px
        .lock()
        .map(|g| *g)
        .unwrap_or(DEFAULT_SIDEBAR_PX);
    let _ = apply_layout_to_mpv(m, logical_width, sidebar_px);
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
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            app.deep_link().handle_cli_arguments(argv.into_iter());
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let db = db::AppDatabase::open_portable()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            app.manage(db);

            #[cfg(any(windows, target_os = "linux"))]
            let _ = app.deep_link().register_all();

            #[cfg(windows)]
            {
                app.manage(AppState::default());

                // Tear libmpv down before the main window's HWND becomes
                // invalid, and re-issue the video margin natively on every
                // resize so the modal resize loop doesn't have to wait on
                // a JS -> invoke() round-trip per frame.
                if let Some(window) = app.get_webview_window("main") {
                    let app_handle = app.handle().clone();
                    let window_for_handler = window.clone();
                    window.on_window_event(move |event| match event {
                        tauri::WindowEvent::CloseRequested { .. } => {
                            if let Some(state) = app_handle.try_state::<AppState>() {
                                if let Ok(mut guard) = state.mpv.lock() {
                                    guard.take();
                                }
                            }
                        }
                        tauri::WindowEvent::Resized(size) => {
                            let scale = window_for_handler.scale_factor().unwrap_or(1.0);
                            handle_native_resize(&app_handle, size.width, scale);
                        }
                        tauri::WindowEvent::ScaleFactorChanged {
                            scale_factor,
                            new_inner_size,
                            ..
                        } => {
                            handle_native_resize(&app_handle, new_inner_size.width, *scale_factor);
                        }
                        _ => {}
                    });
                }
            }

            Ok(())
        });

    #[cfg(windows)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        library::scan_videos,
        library::get_library_state,
        library::add_root_folder,
        library::remove_root_folder,
        library::create_category,
        library::delete_category,
        library::set_default_category,
        library::create_regex_rule,
        library::update_regex_rule,
        library::delete_regex_rule,
        library::move_anime_to_category,
        library::list_episodes,
        library::get_matching_detection_rule_name,
        library::save_episode_progress,
        library::rescan_library,
        anilist::get_anilist_auth_state,
        anilist::set_anilist_client_id,
        anilist::get_anilist_login_url,
        anilist::complete_anilist_login,
        anilist::logout_anilist,
        anilist::search_anilist_anime,
        anilist::link_anime_anilist,
        anilist::unlink_anime_anilist,
        anilist::get_anilist_cover_image,
        anilist::sync_anilist_episode_progress,
        anilist::get_anilist_media_status,
        anilist::apply_anilist_progress_to_local,
        anilist::set_anilist_media_score,
        thumbnails::get_file_thumbnail,
        mpv_init,
        mpv_load,
        mpv_cycle_pause,
        mpv_set_pause,
        mpv_get_tracks,
        mpv_select_audio_track,
        mpv_select_subtitle_track,
        mpv_get_video_geometry,
        mpv_seek,
        mpv_seek_relative,
        mpv_set_layout,
        mpv_stop,
    ]);

    #[cfg(not(windows))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        library::scan_videos,
        library::get_library_state,
        library::add_root_folder,
        library::remove_root_folder,
        library::create_category,
        library::delete_category,
        library::set_default_category,
        library::create_regex_rule,
        library::update_regex_rule,
        library::delete_regex_rule,
        library::move_anime_to_category,
        library::list_episodes,
        library::get_matching_detection_rule_name,
        library::save_episode_progress,
        library::rescan_library,
        anilist::get_anilist_auth_state,
        anilist::set_anilist_client_id,
        anilist::get_anilist_login_url,
        anilist::complete_anilist_login,
        anilist::logout_anilist,
        anilist::search_anilist_anime,
        anilist::link_anime_anilist,
        anilist::unlink_anime_anilist,
        anilist::get_anilist_cover_image,
        anilist::sync_anilist_episode_progress,
        anilist::get_anilist_media_status,
        anilist::apply_anilist_progress_to_local,
        anilist::set_anilist_media_score,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
