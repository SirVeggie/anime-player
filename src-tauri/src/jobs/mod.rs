mod manager;
mod types;

pub use types::*;

use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::db::AppDatabase;

use manager::{JobManager, WorkerOutcome};

#[cfg(windows)]
pub struct JobsState {
    pub manager: Mutex<JobManager>,
}

#[cfg(windows)]
impl JobsState {
    pub fn new(app: AppHandle, db: &AppDatabase) -> Self {
        Self {
            manager: Mutex::new(JobManager::new(app, db)),
        }
    }
}

#[cfg(windows)]
pub fn notify_job_step(app: &AppHandle, job_id: &str, current: u32, total: u32, label: &str) {
    let Some(state) = app.try_state::<JobsState>() else {
        return;
    };
    if let Ok(mut guard) = state.manager.lock() {
        guard.update_step(job_id, current, total, label);
    };
}

#[cfg(windows)]
pub use manager::RescanScrubImport;

#[cfg(windows)]
pub fn enqueue_scrub_for_rescan_imports(
    jobs: &JobsState,
    imports: &[RescanScrubImport],
) -> Result<(), String> {
    if imports.len() > manager::RESCAN_AUTO_SCRUB_MAX {
        return Ok(());
    }
    let mut guard = jobs.manager.lock().map_err(|e| e.to_string())?;
    guard.enqueue_scrub_for_rescan_imports(imports)
}

#[cfg(windows)]
fn with_manager<T>(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    f: impl FnOnce(&mut JobManager, &AppDatabase) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = jobs.manager.lock().map_err(|e| e.to_string())?;
    f(&mut guard, &db)
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_get_snapshot(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
) -> Result<JobsSnapshot, String> {
    with_manager(jobs, db, |manager, _| Ok(manager.snapshot()))
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_set_max_parallel(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    max_parallel: u32,
) -> Result<(), String> {
    with_manager(jobs, db, |manager, db| manager.set_max_parallel(db, max_parallel))
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_set_type_max_parallel(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    resource_type: String,
    max_parallel: u32,
) -> Result<(), String> {
    with_manager(jobs, db, |manager, db| {
        manager.set_type_max_parallel(db, &resource_type, max_parallel)
    })
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_cancel(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    job_id: String,
) -> Result<(), String> {
    with_manager(jobs, db, |manager, _| {
        manager.cancel(&job_id)?;
        manager.emit_snapshot();
        Ok(())
    })
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_cancel_all(jobs: State<'_, JobsState>, db: State<'_, AppDatabase>) -> Result<(), String> {
    with_manager(jobs, db, |manager, _| {
        manager.cancel_all();
        manager.emit_snapshot();
        Ok(())
    })
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_enqueue_scrub_sprite(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    request: EnqueueScrubSpriteJob,
) -> Result<EnqueueJobResult, String> {
    with_manager(jobs, db, |manager, _| manager.enqueue_scrub_sprite(request))
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_set_job_priority(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    job_id: String,
    priority: JobPriority,
) -> Result<(), String> {
    with_manager(jobs, db, |manager, _| manager.set_job_priority(&job_id, priority))
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_set_scrub_sprite_priority_for_paths(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    paths: Vec<String>,
    priority: JobPriority,
) -> Result<(), String> {
    with_manager(jobs, db, |manager, _| {
        manager.set_scrub_sprite_priority_for_paths(&paths, priority)
    })
}

#[cfg(windows)]
fn complete_worker_task(app: AppHandle, job_id: String, outcome: WorkerOutcome) {
    let Some(jobs_state) = app.try_state::<JobsState>() else {
        return;
    };
    if let Ok(mut guard) = jobs_state.manager.lock() {
        guard.complete_worker(&job_id, outcome);
    };
}

#[cfg(windows)]
pub fn spawn_scrub_worker(app: AppHandle, job_id: String, path: String, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let job_id_for_step = job_id.clone();
        let app_for_step = app.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            manager::run_scrub_job_worker(&path, &cancel, |step, total, label| {
                notify_job_step(&app_for_step, &job_id_for_step, step, total, label);
            })
        })
        .await;

        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => WorkerOutcome::Failed(e.to_string()),
        };
        complete_worker_task(app, job_id, outcome);
    });
}

#[cfg(windows)]
pub fn spawn_op_ed_worker(
    app: AppHandle,
    job_id: String,
    anime_id: i64,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        let job_id_for_step = job_id.clone();
        let app_for_step = app.clone();
        let app_for_blocking = app.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            manager::run_op_ed_job_worker(&app_for_blocking, anime_id, &cancel, |step, total, label| {
                notify_job_step(&app_for_step, &job_id_for_step, step, total, label);
            })
        })
        .await;

        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => WorkerOutcome::Failed(e.to_string()),
        };
        complete_worker_task(app, job_id, outcome);
    });
}

#[cfg(windows)]
#[tauri::command]
pub fn jobs_enqueue_op_ed_detect(
    jobs: State<'_, JobsState>,
    db: State<'_, AppDatabase>,
    request: EnqueueOpEdDetectJob,
) -> Result<EnqueueJobResult, String> {
    with_manager(jobs, db, |manager, _| manager.enqueue_op_ed_detect(request))
}
