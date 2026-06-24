use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::AppDatabase;
use crate::library::{self, LocalDataCleanupSummary, LocalDataStats, ScanSummary};

const HISTORY_LIMIT: i64 = 100;

pub struct LibraryOpsState {
    worker_running: Mutex<bool>,
}

impl LibraryOpsState {
    pub fn new(db: &AppDatabase) -> Self {
        let _ = db.with_conn(|conn| {
            conn.execute(
                "UPDATE library_operations
                 SET status = 'queued',
                     phase = 'queued',
                     started_at = NULL,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE status = 'running'",
                [],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        });
        Self {
            worker_running: Mutex::new(false),
        }
    }
}

pub fn start_queued_operations(app: AppHandle) {
    let state_app = app.clone();
    let Some(ops_state) = state_app.try_state::<LibraryOpsState>() else {
        return;
    };
    wake_worker(app, ops_state);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryOperationType {
    DeleteAnime,
    DeleteEpisode,
    CleanLocalData,
    RescanLibrary,
    LocalDataStats,
}

impl LibraryOperationType {
    fn as_str(self) -> &'static str {
        match self {
            Self::DeleteAnime => "delete_anime",
            Self::DeleteEpisode => "delete_episode",
            Self::CleanLocalData => "clean_local_data",
            Self::RescanLibrary => "rescan_library",
            Self::LocalDataStats => "local_data_stats",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "delete_anime" => Some(Self::DeleteAnime),
            "delete_episode" => Some(Self::DeleteEpisode),
            "clean_local_data" => Some(Self::CleanLocalData),
            "rescan_library" => Some(Self::RescanLibrary),
            "local_data_stats" => Some(Self::LocalDataStats),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryOperationStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

impl LibraryOperationStatus {
    fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOperationView {
    id: i64,
    operation_type: LibraryOperationType,
    status: LibraryOperationStatus,
    phase: String,
    target_anime_id: Option<i64>,
    target_episode_id: Option<i64>,
    progress_current: i64,
    progress_total: i64,
    summary_json: Option<String>,
    error_text: Option<String>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOpsSnapshot {
    active: Vec<LibraryOperationView>,
    history: Vec<LibraryOperationView>,
    active_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOperationFinishedEvent {
    operation_id: i64,
    operation_type: LibraryOperationType,
    status: LibraryOperationStatus,
    target_anime_id: Option<i64>,
    target_episode_id: Option<i64>,
    summary_json: Option<String>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUpdatedEvent {
    reason: String,
    operation_id: Option<i64>,
    stats_changed: bool,
}

#[derive(Debug, Clone)]
struct OperationRecord {
    id: i64,
    operation_type: LibraryOperationType,
    target_anime_id: Option<i64>,
    target_episode_id: Option<i64>,
}

#[tauri::command]
pub fn library_ops_get_snapshot(db: State<'_, AppDatabase>) -> Result<LibraryOpsSnapshot, String> {
    db.with_conn(load_snapshot)
}

#[tauri::command]
pub fn library_ops_request_delete_anime(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
    anime_id: i64,
) -> Result<LibraryOperationView, String> {
    let operation = db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let visible_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM episodes
                 WHERE anime_id = ?1 AND missing = 0 AND pending_delete = 0",
                params![anime_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if visible_count == 0 {
            return Err("No visible episode files are available to delete.".to_string());
        }
        tx.execute(
            "UPDATE anime
             SET pending_delete = 1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE episodes
             SET pending_delete = 1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE anime_id = ?1 AND missing = 0",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
        let operation_id = insert_operation(
            &tx,
            LibraryOperationType::DeleteAnime,
            Some(anime_id),
            None,
            "{}",
            visible_count,
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        load_operation(conn, operation_id)
    })?;
    emit_library_updated(&app, "delete-queued", Some(operation.id), false);
    wake_worker(app, ops);
    Ok(operation)
}

#[tauri::command]
pub fn library_ops_request_delete_episode(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
    episode_id: i64,
) -> Result<LibraryOperationView, String> {
    let operation = db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let anime_id: i64 = tx
            .query_row(
                "SELECT anime_id FROM episodes
                 WHERE id = ?1 AND missing = 0 AND pending_delete = 0",
                params![episode_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Episode file is no longer visible.".to_string())?;
        tx.execute(
            "UPDATE episodes
             SET pending_delete = 1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![episode_id],
        )
        .map_err(|e| e.to_string())?;
        let remaining_visible: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM episodes
                 WHERE anime_id = ?1 AND missing = 0 AND pending_delete = 0",
                params![anime_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if remaining_visible == 0 {
            tx.execute(
                "UPDATE anime
                 SET pending_delete = 1,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![anime_id],
            )
            .map_err(|e| e.to_string())?;
        }
        let operation_id = insert_operation(
            &tx,
            LibraryOperationType::DeleteEpisode,
            Some(anime_id),
            Some(episode_id),
            "{}",
            1,
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        load_operation(conn, operation_id)
    })?;
    emit_library_updated(&app, "delete-queued", Some(operation.id), false);
    wake_worker(app, ops);
    Ok(operation)
}

#[tauri::command]
pub fn library_ops_request_clean_local_data(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
) -> Result<LibraryOperationView, String> {
    enqueue_simple_operation(
        app,
        db,
        ops,
        LibraryOperationType::CleanLocalData,
        Some("clean-queued"),
    )
}

#[tauri::command]
pub fn library_ops_request_rescan(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
) -> Result<LibraryOperationView, String> {
    enqueue_simple_operation(app, db, ops, LibraryOperationType::RescanLibrary, None)
}

#[tauri::command]
pub fn library_ops_request_local_data_stats_refresh(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
) -> Result<LibraryOperationView, String> {
    enqueue_simple_operation(
        app,
        db,
        ops,
        LibraryOperationType::LocalDataStats,
        Some("stats-queued"),
    )
}

fn enqueue_simple_operation(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    ops: State<'_, LibraryOpsState>,
    operation_type: LibraryOperationType,
    reason: Option<&str>,
) -> Result<LibraryOperationView, String> {
    let operation = db.with_conn(|conn| {
        let operation_id = insert_operation(conn, operation_type, None, None, "{}", 1)?;
        load_operation(conn, operation_id)
    })?;
    if let Some(reason) = reason {
        emit_library_updated(&app, reason, Some(operation.id), false);
    }
    wake_worker(app, ops);
    Ok(operation)
}

fn wake_worker(app: AppHandle, ops: State<'_, LibraryOpsState>) {
    let Ok(mut running) = ops.worker_running.lock() else {
        return;
    };
    if *running {
        return;
    }
    *running = true;
    drop(running);

    tauri::async_runtime::spawn_blocking(move || {
        run_worker_loop(app.clone());
        if let Some(ops_state) = app.try_state::<LibraryOpsState>() {
            if let Ok(mut running) = ops_state.worker_running.lock() {
                *running = false;
            }
        }
        let should_restart = app
            .try_state::<AppDatabase>()
            .and_then(|db| db.with_conn(has_queued_operations).ok())
            .unwrap_or(false);
        if should_restart {
            if let Some(ops_state) = app.try_state::<LibraryOpsState>() {
                wake_worker(app.clone(), ops_state);
            }
        }
    });
}

fn run_worker_loop(app: AppHandle) {
    loop {
        let Some(record) = (match app
            .state::<AppDatabase>()
            .with_conn(take_next_operation)
        {
            Ok(record) => record,
            Err(error) => {
                crate::crash_log::log("ERROR", &format!("library operation dequeue failed: {error}"));
                None
            }
        }) else {
            emit_snapshot(&app);
            break;
        };
        emit_snapshot(&app);
        run_operation(&app, record);
    }
}

fn run_operation(app: &AppHandle, record: OperationRecord) {
    let result = match record.operation_type {
        LibraryOperationType::DeleteAnime => run_delete_anime(app, &record),
        LibraryOperationType::DeleteEpisode => run_delete_episode(app, &record),
        LibraryOperationType::CleanLocalData => run_clean_local_data(app, &record),
        LibraryOperationType::RescanLibrary => run_rescan(app, &record),
        LibraryOperationType::LocalDataStats => run_local_data_stats(app, &record),
    };
    if let Err(error) = result {
        let db = app.state::<AppDatabase>();
        let _ = db.with_conn(|conn| fail_operation(conn, record.id, &error));
        emit_snapshot(app);
        emit_finished(app, record.id);
    }
}

fn run_delete_anime(app: &AppHandle, record: &OperationRecord) -> Result<(), String> {
    let anime_id = match record.target_anime_id {
        Some(id) => id,
        None => {
            finish_operation(app, record.id, "done", None, None, true)?;
            return Ok(());
        }
    };
    update_phase(app, record.id, "deleting files", 0, 1)?;
    let db = app.state::<AppDatabase>();
    let summary = library::delete_anime_files_for_operation(&db, anime_id)?;
    if summary.episodes_failed > 0 || summary.cover_failed {
        db.with_conn(|conn| clear_pending_delete_for_anime(conn, anime_id))?;
        let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
        finish_operation(
            app,
            record.id,
            "failed",
            Some(summary_json),
            Some("Some files could not be deleted.".to_string()),
            true,
        )?;
    } else {
        let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
        finish_operation(app, record.id, "done", Some(summary_json), None, true)?;
    }
    Ok(())
}

fn run_delete_episode(app: &AppHandle, record: &OperationRecord) -> Result<(), String> {
    let episode_id = match record.target_episode_id {
        Some(id) => id,
        None => {
            finish_operation(app, record.id, "done", None, None, true)?;
            return Ok(());
        }
    };
    update_phase(app, record.id, "deleting file", 0, 1)?;
    let db = app.state::<AppDatabase>();
    let summary = library::delete_episode_files_for_operation(&db, episode_id)?;
    if summary.episodes_failed > 0 || summary.cover_failed {
        db.with_conn(|conn| clear_pending_delete_for_episode(conn, episode_id))?;
        if let Some(anime_id) = record.target_anime_id {
            db.with_conn(|conn| clear_pending_delete_for_anime_if_visible(conn, anime_id))?;
        }
        let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
        finish_operation(
            app,
            record.id,
            "failed",
            Some(summary_json),
            Some("The episode file could not be deleted.".to_string()),
            true,
        )?;
    } else {
        let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
        finish_operation(app, record.id, "done", Some(summary_json), None, true)?;
    }
    Ok(())
}

fn run_clean_local_data(app: &AppHandle, record: &OperationRecord) -> Result<(), String> {
    update_phase(app, record.id, "cleaning local data", 0, 1)?;
    let db = app.state::<AppDatabase>();
    let summary: LocalDataCleanupSummary = library::clean_local_data_for_operation(&db)?;
    let _ = library::refresh_local_data_stats_cache(&db);
    let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
    finish_operation(app, record.id, "done", Some(summary_json), None, true)?;
    emit_library_updated(app, "stats-updated", Some(record.id), true);
    Ok(())
}

fn run_rescan(app: &AppHandle, record: &OperationRecord) -> Result<(), String> {
    update_phase(app, record.id, "scanning library", 0, 1)?;
    let db = app.state::<AppDatabase>();
    #[cfg(windows)]
    let summary: ScanSummary = library::rescan_library_for_operation(app.clone(), &db)?;
    #[cfg(not(windows))]
    let summary: ScanSummary = library::rescan_library_for_operation(&db)?;
    let _ = library::refresh_local_data_stats_cache(&db);
    let summary_json = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
    finish_operation(app, record.id, "done", Some(summary_json), None, false)?;
    emit_library_updated(app, "done", Some(record.id), true);
    Ok(())
}

fn run_local_data_stats(app: &AppHandle, record: &OperationRecord) -> Result<(), String> {
    update_phase(app, record.id, "measuring local data", 0, 1)?;
    let db = app.state::<AppDatabase>();
    let stats: LocalDataStats = library::refresh_local_data_stats_cache(&db)?;
    let summary_json = serde_json::to_string(&stats).map_err(|e| e.to_string())?;
    finish_operation(app, record.id, "done", Some(summary_json), None, false)?;
    emit_library_updated(app, "stats-updated", Some(record.id), true);
    Ok(())
}

fn insert_operation(
    conn: &Connection,
    operation_type: LibraryOperationType,
    target_anime_id: Option<i64>,
    target_episode_id: Option<i64>,
    payload_json: &str,
    progress_total: i64,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO library_operations
            (operation_type, status, phase, target_anime_id, target_episode_id,
             payload_json, progress_total)
         VALUES (?1, 'queued', 'queued', ?2, ?3, ?4, ?5)",
        params![
            operation_type.as_str(),
            target_anime_id,
            target_episode_id,
            payload_json,
            progress_total
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn take_next_operation(conn: &mut Connection) -> Result<Option<OperationRecord>, String> {
    let record = conn
        .query_row(
            "SELECT id, operation_type, target_anime_id, target_episode_id
             FROM library_operations
             WHERE status = 'queued'
             ORDER BY id
             LIMIT 1",
            [],
            |row| {
                let operation_type = row.get::<_, String>(1)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    operation_type,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, operation_type, target_anime_id, target_episode_id)) = record else {
        return Ok(None);
    };
    let operation_type = LibraryOperationType::parse(&operation_type)
        .ok_or_else(|| format!("Unknown library operation type: {operation_type}"))?;
    conn.execute(
        "UPDATE library_operations
         SET status = 'running',
             phase = 'starting',
             started_at = COALESCE(started_at, CURRENT_TIMESTAMP),
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(OperationRecord {
        id,
        operation_type,
        target_anime_id,
        target_episode_id,
    }))
}

fn update_phase(
    app: &AppHandle,
    operation_id: i64,
    phase: &str,
    progress_current: i64,
    progress_total: i64,
) -> Result<(), String> {
    app.state::<AppDatabase>().with_conn(|conn| {
        conn.execute(
            "UPDATE library_operations
             SET phase = ?2,
                 progress_current = ?3,
                 progress_total = ?4,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![operation_id, phase, progress_current, progress_total],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    emit_snapshot(app);
    Ok(())
}

fn finish_operation(
    app: &AppHandle,
    operation_id: i64,
    status: &str,
    summary_json: Option<String>,
    error_text: Option<String>,
    library_changed: bool,
) -> Result<(), String> {
    app.state::<AppDatabase>().with_conn(|conn| {
        conn.execute(
            "UPDATE library_operations
             SET status = ?2,
                 phase = ?2,
                 progress_current = progress_total,
                 summary_json = ?3,
                 error_text = ?4,
                 finished_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![operation_id, status, summary_json, error_text],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    emit_snapshot(app);
    emit_finished(app, operation_id);
    if library_changed {
        emit_library_updated(app, status, Some(operation_id), false);
    }
    Ok(())
}

fn fail_operation(conn: &mut Connection, operation_id: i64, error_text: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE library_operations
         SET status = 'failed',
             phase = 'failed',
             error_text = ?2,
             finished_at = CURRENT_TIMESTAMP,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![operation_id, error_text],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn has_queued_operations(conn: &mut Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_operations WHERE status = 'queued')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|e| e.to_string())
}

fn load_snapshot(conn: &mut Connection) -> Result<LibraryOpsSnapshot, String> {
    let active = load_operations_by_status(conn, &["queued", "running"], None)?;
    let history = load_operations_by_status(conn, &["done", "failed", "canceled"], Some(HISTORY_LIMIT))?;
    Ok(LibraryOpsSnapshot {
        active_count: active.len() as u32,
        active,
        history,
    })
}

fn load_operations_by_status(
    conn: &Connection,
    statuses: &[&str],
    limit: Option<i64>,
) -> Result<Vec<LibraryOperationView>, String> {
    let status_list = statuses
        .iter()
        .map(|status| format!("'{status}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "SELECT id, operation_type, status, phase, target_anime_id, target_episode_id,
                progress_current, progress_total, summary_json, error_text,
                created_at, started_at, finished_at, updated_at
         FROM library_operations
         WHERE status IN ({status_list})
         ORDER BY id DESC"
    );
    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], operation_from_row)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

fn load_operation(conn: &Connection, operation_id: i64) -> Result<LibraryOperationView, String> {
    conn.query_row(
        "SELECT id, operation_type, status, phase, target_anime_id, target_episode_id,
                progress_current, progress_total, summary_json, error_text,
                created_at, started_at, finished_at, updated_at
         FROM library_operations
         WHERE id = ?1",
        params![operation_id],
        operation_from_row,
    )
    .map_err(|e| e.to_string())
}

fn operation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryOperationView> {
    let operation_type = row.get::<_, String>(1)?;
    let status = row.get::<_, String>(2)?;
    Ok(LibraryOperationView {
        id: row.get(0)?,
        operation_type: LibraryOperationType::parse(&operation_type)
            .unwrap_or(LibraryOperationType::CleanLocalData),
        status: LibraryOperationStatus::parse(&status),
        phase: row.get(3)?,
        target_anime_id: row.get(4)?,
        target_episode_id: row.get(5)?,
        progress_current: row.get(6)?,
        progress_total: row.get(7)?,
        summary_json: row.get(8)?,
        error_text: row.get(9)?,
        created_at: row.get(10)?,
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn emit_snapshot(app: &AppHandle) {
    if let Some(db) = app.try_state::<AppDatabase>() {
        if let Ok(snapshot) = db.with_conn(load_snapshot) {
            let _ = app.emit("library-ops://updated", snapshot);
        }
    }
}

fn emit_finished(app: &AppHandle, operation_id: i64) {
    let Some(db) = app.try_state::<AppDatabase>() else {
        return;
    };
    let Ok(operation) = db.with_conn(|conn| load_operation(conn, operation_id)) else {
        return;
    };
    let _ = app.emit(
        "library-ops://finished",
        LibraryOperationFinishedEvent {
            operation_id: operation.id,
            operation_type: operation.operation_type,
            status: operation.status,
            target_anime_id: operation.target_anime_id,
            target_episode_id: operation.target_episode_id,
            summary_json: operation.summary_json,
            error_text: operation.error_text,
        },
    );
}

fn emit_library_updated(
    app: &AppHandle,
    reason: &str,
    operation_id: Option<i64>,
    stats_changed: bool,
) {
    let _ = app.emit(
        "library://updated",
        LibraryUpdatedEvent {
            reason: reason.to_string(),
            operation_id,
            stats_changed,
        },
    );
}

fn clear_pending_delete_for_anime(conn: &mut Connection, anime_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE anime SET pending_delete = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE episodes
         SET pending_delete = 0,
             updated_at = CURRENT_TIMESTAMP
         WHERE anime_id = ?1",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn clear_pending_delete_for_episode(conn: &mut Connection, episode_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE episodes
         SET pending_delete = 0,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![episode_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn clear_pending_delete_for_anime_if_visible(
    conn: &mut Connection,
    anime_id: i64,
) -> Result<(), String> {
    let visible_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes
             WHERE anime_id = ?1 AND missing = 0 AND pending_delete = 0",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if visible_count > 0 {
        conn.execute(
            "UPDATE anime
             SET pending_delete = 0,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
