//! Recursive filesystem watchers on configured root folders.
//!
//! Events are path-deduped, quiet-debounced, and size-settled before a single
//! coalesced library rescan is requested (see `library_ops::request_rescan_coalesced`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Manager};

use crate::db::AppDatabase;
use crate::library;
use crate::scanner::is_video_file;

const QUIET_DEBOUNCE: Duration = Duration::from_secs(2);
const SETTLE_POLL: Duration = Duration::from_millis(500);
const SETTLE_STABLE: Duration = Duration::from_millis(1500);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);
const LOOP_TICK: Duration = Duration::from_millis(100);

enum ControlMsg {
    Reconfigure { enabled: bool, roots: Vec<PathBuf> },
}

struct PendingPaths {
    settle: HashSet<PathBuf>,
    deleted: HashSet<PathBuf>,
}

impl PendingPaths {
    fn new() -> Self {
        Self {
            settle: HashSet::new(),
            deleted: HashSet::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.settle.is_empty() && self.deleted.is_empty()
    }

    fn clear(&mut self) {
        self.settle.clear();
        self.deleted.clear();
    }

    fn take(&mut self) -> HashSet<PathBuf> {
        let settle = std::mem::take(&mut self.settle);
        self.deleted.clear();
        settle
    }
}

pub struct LibraryWatcherState {
    control_tx: Mutex<Option<Sender<ControlMsg>>>,
    started: AtomicBool,
}

impl LibraryWatcherState {
    pub fn new() -> Self {
        Self {
            control_tx: Mutex::new(None),
            started: AtomicBool::new(false),
        }
    }
}

/// Start the watcher worker (once) and apply the current DB setting + roots.
pub fn start(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<LibraryWatcherState>();
    if !state.started.swap(true, Ordering::SeqCst) {
        let (tx, rx) = mpsc::channel::<ControlMsg>();
        {
            let mut guard = state
                .control_tx
                .lock()
                .map_err(|_| "library watcher control lock poisoned".to_string())?;
            *guard = Some(tx);
        }
        let app_handle = app.clone();
        thread::Builder::new()
            .name("library-watcher".into())
            .spawn(move || worker_loop(app_handle, rx))
            .map_err(|e| format!("failed to start library watcher thread: {e}"))?;
    }
    reconfigure_from_db(app)
}

pub fn reconfigure_from_db(app: &AppHandle) -> Result<(), String> {
    let db = app.state::<AppDatabase>();
    let (enabled, roots) = db.with_conn(|conn| {
        Ok((
            library::read_automatic_file_discovery(conn)?,
            library::root_folder_paths(conn)?,
        ))
    })?;
    send_control(app, ControlMsg::Reconfigure { enabled, roots })
}

fn send_control(app: &AppHandle, msg: ControlMsg) -> Result<(), String> {
    let state = app.state::<LibraryWatcherState>();
    let guard = state
        .control_tx
        .lock()
        .map_err(|_| "library watcher control lock poisoned".to_string())?;
    let Some(tx) = guard.as_ref() else {
        return Ok(());
    };
    tx.send(msg)
        .map_err(|_| "library watcher worker is not running".to_string())
}

fn worker_loop(app: AppHandle, control_rx: Receiver<ControlMsg>) {
    let (event_tx, event_rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: Option<RecommendedWatcher> = None;
    let pending = Arc::new(Mutex::new(PendingPaths::new()));
    let generation = Arc::new(AtomicU64::new(0));
    let settle_in_flight = Arc::new(AtomicBool::new(false));
    let mut last_event_at: Option<Instant> = None;

    loop {
        // Prefer control messages, then drain FS events, then maybe flush.
        let control = match control_rx.recv_timeout(LOOP_TICK) {
            Ok(msg) => Some(msg),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                drop(watcher);
                return;
            }
        };

        if let Some(ControlMsg::Reconfigure { enabled, roots }) = control {
            apply_reconfigure(
                &mut watcher,
                event_tx.clone(),
                &pending,
                &generation,
                &settle_in_flight,
                &mut last_event_at,
                enabled,
                roots,
            );
        }

        let mut received = false;
        while let Ok(result) = event_rx.try_recv() {
            received = true;
            match result {
                Ok(event) => apply_event(&pending, event),
                Err(error) => {
                    crate::crash_log::log(
                        "WARN",
                        &format!("library watcher notify error: {error}"),
                    );
                }
            }
        }
        if received {
            last_event_at = Some(Instant::now());
        }

        let quiet_enough = last_event_at
            .map(|at| at.elapsed() >= QUIET_DEBOUNCE)
            .unwrap_or(false);
        let has_pending = pending
            .lock()
            .map(|guard| !guard.is_empty())
            .unwrap_or(false);
        let can_flush = quiet_enough
            && has_pending
            && !settle_in_flight.load(Ordering::SeqCst);

        if can_flush {
            last_event_at = None;
            settle_in_flight.store(true, Ordering::SeqCst);
            let pending_clone = Arc::clone(&pending);
            let generation_clone = Arc::clone(&generation);
            let settle_flag = Arc::clone(&settle_in_flight);
            let gen_at_start = generation.load(Ordering::SeqCst);
            let app_clone = app.clone();
            thread::spawn(move || {
                let settle_paths = match pending_clone.lock() {
                    Ok(mut guard) => guard.take(),
                    Err(_) => {
                        settle_flag.store(false, Ordering::SeqCst);
                        return;
                    }
                };
                if generation_clone.load(Ordering::SeqCst) != gen_at_start {
                    settle_flag.store(false, Ordering::SeqCst);
                    return;
                }
                if !settle_paths.is_empty() {
                    wait_for_size_settle(&settle_paths, &generation_clone, gen_at_start);
                }
                if generation_clone.load(Ordering::SeqCst) != gen_at_start {
                    settle_flag.store(false, Ordering::SeqCst);
                    return;
                }
                if let Err(error) = crate::library_ops::request_rescan_coalesced(&app_clone) {
                    crate::crash_log::log(
                        "ERROR",
                        &format!("library watcher rescan request failed: {error}"),
                    );
                }
                settle_flag.store(false, Ordering::SeqCst);
            });
        }
    }
}

fn apply_reconfigure(
    watcher: &mut Option<RecommendedWatcher>,
    event_tx: Sender<notify::Result<Event>>,
    pending: &Arc<Mutex<PendingPaths>>,
    generation: &AtomicU64,
    settle_in_flight: &AtomicBool,
    last_event_at: &mut Option<Instant>,
    enabled: bool,
    roots: Vec<PathBuf>,
) {
    generation.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut guard) = pending.lock() {
        guard.clear();
    }
    *last_event_at = None;
    settle_in_flight.store(false, Ordering::SeqCst);
    *watcher = None;
    if !enabled || roots.is_empty() {
        crate::crash_log::log(
            "INFO",
            "library watcher idle (disabled or no root folders)",
        );
        return;
    }
    match build_watcher(event_tx, &roots) {
        Ok(built) => {
            crate::crash_log::log(
                "INFO",
                &format!("library watcher watching {} root folder(s)", roots.len()),
            );
            *watcher = Some(built);
        }
        Err(error) => {
            crate::crash_log::log(
                "ERROR",
                &format!("library watcher failed to start: {error}"),
            );
        }
    }
}

fn build_watcher(
    event_tx: Sender<notify::Result<Event>>,
    roots: &[PathBuf],
) -> Result<RecommendedWatcher, String> {
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = event_tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    for root in roots {
        if !root.is_dir() {
            crate::crash_log::log(
                "WARN",
                &format!(
                    "library watcher skipping missing root: {}",
                    root.display()
                ),
            );
            continue;
        }
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| format!("watch {}: {e}", root.display()))?;
    }
    Ok(watcher)
}

fn apply_event(pending: &Arc<Mutex<PendingPaths>>, event: Event) {
    let Ok(mut guard) = pending.lock() else {
        return;
    };
    let paths = relevant_paths(&event);
    if paths.is_empty() {
        return;
    }
    match &event.kind {
        EventKind::Remove(_) => {
            for path in paths {
                guard.settle.remove(&path);
                guard.deleted.insert(path);
            }
        }
        EventKind::Modify(_) | EventKind::Create(_) => {
            for path in paths {
                guard.deleted.remove(&path);
                guard.settle.insert(path);
            }
        }
        EventKind::Access(_) => {
            // Ignore read/access noise (browsing a folder must not rescan).
        }
        EventKind::Any | EventKind::Other => {
            for path in paths {
                if path.exists() {
                    guard.deleted.remove(&path);
                    guard.settle.insert(path);
                } else {
                    guard.settle.remove(&path);
                    guard.deleted.insert(path);
                }
            }
        }
    }
}

fn relevant_paths(event: &Event) -> Vec<PathBuf> {
    event
        .paths
        .iter()
        .filter(|path| should_track_path(path))
        .cloned()
        .collect()
}

fn should_track_path(path: &Path) -> bool {
    if is_video_file(path) {
        return true;
    }
    // Incomplete downloads often use temporary extensions; still track if the
    // name embeds a video extension (e.g. `.mkv.part`).
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    crate::scanner::VIDEO_EXTENSIONS.iter().any(|ext| {
        name.contains(&format!(".{ext}."))
            || name.ends_with(&format!(".{ext}.part"))
            || name.ends_with(&format!(".{ext}.tmp"))
    })
}

fn wait_for_size_settle(paths: &HashSet<PathBuf>, generation: &AtomicU64, gen_at_start: u64) {
    let started = Instant::now();
    let mut last_sizes: Vec<(PathBuf, u64)> = paths
        .iter()
        .filter(|path| path.is_file())
        .filter_map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|meta| (path.clone(), meta.len()))
        })
        .collect();
    if last_sizes.is_empty() {
        return;
    }
    let mut stable_since = Instant::now();
    while started.elapsed() < SETTLE_TIMEOUT {
        if generation.load(Ordering::SeqCst) != gen_at_start {
            return;
        }
        thread::sleep(SETTLE_POLL);
        let mut changed = false;
        for (path, prev_size) in &mut last_sizes {
            let Ok(meta) = std::fs::metadata(&*path) else {
                continue;
            };
            let size = meta.len();
            if size != *prev_size {
                *prev_size = size;
                changed = true;
            }
        }
        if changed {
            stable_since = Instant::now();
        } else if stable_since.elapsed() >= SETTLE_STABLE {
            return;
        }
    }
}
