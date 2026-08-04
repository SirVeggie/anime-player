//! Close-into-tray, tray menu, quit coordination, and launch-at-startup sync.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

pub const QUIT_REQUESTED_EVENT: &str = "app://quit-requested";

const TRAY_ID: &str = "main-tray";
const MENU_SHOW: &str = "tray-show";
const MENU_QUIT: &str = "tray-quit";

pub struct AppLifecycleState {
    pub close_into_tray: AtomicBool,
    pub is_quitting: AtomicBool,
    tray: Mutex<Option<TrayIcon>>,
}

impl AppLifecycleState {
    pub fn new(close_into_tray: bool) -> Self {
        Self {
            close_into_tray: AtomicBool::new(close_into_tray),
            is_quitting: AtomicBool::new(false),
            tray: Mutex::new(None),
        }
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn set_close_into_tray(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let state = app.state::<AppLifecycleState>();
    state.close_into_tray.store(enabled, Ordering::SeqCst);
    if enabled {
        ensure_tray(app)?;
    } else {
        remove_tray(app);
    }
    Ok(())
}

pub fn sync_launch_at_startup(app: &AppHandle, enabled: bool) {
    let autolaunch = app.autolaunch();
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Err(error) = result {
        crate::crash_log::log(
            "WARN",
            &format!("launch-at-startup sync failed (enabled={enabled}): {error}"),
        );
    }
}

pub fn reconcile_launch_at_startup_from_db(app: &AppHandle) {
    let enabled = app
        .state::<crate::db::AppDatabase>()
        .with_conn(|conn| crate::library::read_launch_at_startup(conn))
        .unwrap_or(false);
    sync_launch_at_startup(app, enabled);
}

fn ensure_tray(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppLifecycleState>();
    {
        let guard = state
            .tray
            .lock()
            .map_err(|_| "tray lock poisoned".to_string())?;
        if guard.is_some() {
            return Ok(());
        }
    }

    let show_item = MenuItem::with_id(app, MENU_SHOW, "Show", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item]).map_err(|e| e.to_string())?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "default window icon unavailable for tray".to_string())?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Anime Player")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_QUIT => request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;

    let mut guard = state
        .tray
        .lock()
        .map_err(|_| "tray lock poisoned".to_string())?;
    *guard = Some(tray);
    crate::crash_log::log("INFO", "system tray icon created");
    Ok(())
}

fn remove_tray(app: &AppHandle) {
    let state = app.state::<AppLifecycleState>();
    let removed = state
        .tray
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
        .is_some();
    if removed {
        crate::crash_log::log("INFO", "system tray icon removed");
    }
}

pub fn request_quit(app: &AppHandle) {
    let _ = app.emit(QUIT_REQUESTED_EVENT, ());
}

pub fn handle_close_requested(
    app: &AppHandle,
    api: &tauri::CloseRequestApi,
    window: &tauri::WebviewWindow,
) {
    #[cfg(windows)]
    drop_mpv(app);

    let state = app.state::<AppLifecycleState>();
    let close_into_tray = state.close_into_tray.load(Ordering::SeqCst);
    let is_quitting = state.is_quitting.load(Ordering::SeqCst);
    if close_into_tray && !is_quitting {
        api.prevent_close();
        let _ = window.hide();
        crate::crash_log::log("INFO", "window hidden to tray");
    }
}

#[cfg(windows)]
fn drop_mpv(app: &AppHandle) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        if let Ok(mut guard) = state.mpv.lock() {
            guard.take();
        }
    }
}

#[tauri::command]
pub fn confirm_quit(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppLifecycleState>();
    state.is_quitting.store(true, Ordering::SeqCst);
    remove_tray(&app);
    #[cfg(windows)]
    drop_mpv(&app);
    if let Some(window) = app.get_webview_window("main") {
        window.destroy().map_err(|e| e.to_string())?;
    } else {
        app.exit(0);
    }
    Ok(())
}

#[tauri::command]
pub fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppLifecycleState>();
    if !state.close_into_tray.load(Ordering::SeqCst) {
        return Err("Close into tray is disabled.".to_string());
    }
    ensure_tray(&app)?;
    #[cfg(windows)]
    drop_mpv(&app);
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
