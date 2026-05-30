use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter};

use crate::db::AppDatabase;
use crate::scrub_preview::{
    self, emit_scrub_sprite_status, run_scrub_sprite_job, scrub_sprite_identity, scrub_sprite_is_cached,
};

use super::types::{
    EnqueueJobResult, EnqueueScrubSpriteJob, JobPriority, JobProgress, JobResourceType, JobStatus,
    JobView, JobsSnapshot, TypeMaxParallel,
};

/// Episode newly imported during `rescan_library` (auto scrub enqueue).
#[derive(Debug, Clone)]
pub struct RescanScrubImport {
    pub path: String,
    pub anime_title: String,
    pub episode_label: String,
}

/// When a rescan imports at most this many episodes, queue scrub thumbnails for them.
pub const RESCAN_AUTO_SCRUB_MAX: usize = 20;

const MAX_PARALLEL_SETTING: &str = "jobs_max_parallel";
const TYPE_MAX_PARALLEL_SETTING_PREFIX: &str = "jobs_max_parallel_type_";
const DEFAULT_MAX_PARALLEL: u32 = 5;
const DEFAULT_FFMPEG_MAX_PARALLEL: u32 = 1;
const MAX_PARALLEL_CAP: u32 = 8;
const HISTORY_CAP: usize = 200;

const MANAGED_RESOURCE_TYPES: &[JobResourceType] = &[JobResourceType::Ffmpeg];

static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_job_id() -> String {
    format!("job-{}", JOB_COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone)]
enum JobKind {
    ScrubSprite { path: String },
}

struct JobRecord {
    view: JobView,
    cancel: Arc<AtomicBool>,
    kind: JobKind,
    follow_ups: Vec<EnqueueScrubSpriteJob>,
}

pub struct JobManager {
    app: AppHandle,
    max_parallel: u32,
    type_max_parallel: HashMap<String, u32>,
    queued_ids: Vec<String>,
    records: HashMap<String, JobRecord>,
    identity_to_id: HashMap<String, String>,
    history: VecDeque<JobView>,
}

impl JobManager {
    pub fn new(app: AppHandle, db: &AppDatabase) -> Self {
        let max_parallel = load_max_parallel(db);
        let type_max_parallel = load_type_max_parallel(db);
        Self {
            app,
            max_parallel,
            type_max_parallel,
            queued_ids: Vec::new(),
            records: HashMap::new(),
            identity_to_id: HashMap::new(),
            history: VecDeque::new(),
        }
    }

    pub fn set_max_parallel(&mut self, db: &AppDatabase, value: u32) -> Result<(), String> {
        let clamped = value.clamp(1, MAX_PARALLEL_CAP);
        self.max_parallel = clamped;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![MAX_PARALLEL_SETTING, clamped.to_string()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })?;
        self.pump();
        self.emit_snapshot();
        Ok(())
    }

    pub fn set_type_max_parallel(
        &mut self,
        db: &AppDatabase,
        resource_type: &str,
        value: u32,
    ) -> Result<(), String> {
        let parsed = JobResourceType::parse(resource_type)
            .filter(|t| *t != JobResourceType::None)
            .ok_or_else(|| format!("unknown resource type: {resource_type}"))?;
        let clamped = value.clamp(1, MAX_PARALLEL_CAP);
        self.type_max_parallel
            .insert(parsed.as_str().to_string(), clamped);
        let setting_key = type_max_parallel_setting_key(parsed.as_str());
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![setting_key, clamped.to_string()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })?;
        self.pump();
        self.emit_snapshot();
        Ok(())
    }

    pub fn snapshot(&self) -> JobsSnapshot {
        let mut active: Vec<JobView> = self
            .records
            .values()
            .map(|r| r.view.clone())
            .collect();
        active.sort_by_key(|j| j.created_at);
        let active_count = active
            .iter()
            .filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
            .count() as u32;
        JobsSnapshot {
            active,
            history: self.history.iter().cloned().collect(),
            max_parallel: self.max_parallel,
            type_max_parallel: self.type_max_parallel_snapshot(),
            active_count,
        }
    }

    fn type_max_parallel_snapshot(&self) -> Vec<TypeMaxParallel> {
        MANAGED_RESOURCE_TYPES
            .iter()
            .map(|resource_type| TypeMaxParallel {
                resource_type: resource_type.as_str().to_string(),
                max_parallel: self
                    .type_max_parallel
                    .get(resource_type.as_str())
                    .copied()
                    .unwrap_or(default_type_max_parallel(*resource_type)),
            })
            .collect()
    }

    pub fn emit_snapshot(&self) {
        let _ = self.app.emit("jobs://updated", self.snapshot());
    }

    pub fn cancel(&mut self, job_id: &str) -> Result<(), String> {
        let Some(record) = self.records.get(job_id) else {
            return Err(format!("job not found: {job_id}"));
        };
        if !record.view.cancelable {
            return Err("job is not cancelable".to_string());
        }
        if !matches!(
            record.view.status,
            JobStatus::Queued | JobStatus::Running
        ) {
            return Ok(());
        }
        record.cancel.store(true, Ordering::Relaxed);
        if record.view.status == JobStatus::Queued {
            let job_id = job_id.to_string();
            self.queued_ids.retain(|id| id != &job_id);
            self.finish_job(
                &job_id,
                JobStatus::Canceled,
                Some("Canceled by user".to_string()),
            );
        }
        Ok(())
    }

    pub fn cancel_all(&mut self) {
        let ids: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.view.cancelable
                    && matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let _ = self.cancel(&id);
        }
    }

    /// Changes priority for a **queued** job. High starts when limits allow (type caps
    /// always apply); upgrades call `pump`.
    pub fn set_job_priority(&mut self, job_id: &str, priority: JobPriority) -> Result<(), String> {
        let Some(record) = self.records.get(job_id) else {
            return Err(format!("job not found: {job_id}"));
        };
        if record.view.status != JobStatus::Queued {
            return Err("only queued jobs can change priority".to_string());
        }
        if record.view.priority == priority {
            return Ok(());
        }
        let old = record.view.priority;
        if let Some(record) = self.records.get_mut(job_id) {
            record.view.priority = priority;
        }

        if priority == JobPriority::High {
            self.try_start_or_queue(job_id, true);
            self.pump();
        } else if priority_rank(priority) > priority_rank(old) {
            self.pump();
        }
        self.emit_snapshot();
        Ok(())
    }

    pub fn set_scrub_sprite_priority_for_paths(
        &mut self,
        paths: &[String],
        priority: JobPriority,
    ) -> Result<(), String> {
        let mut job_ids = Vec::new();
        for path in paths {
            let identity = scrub_sprite_identity(path)?;
            if let Some(id) = self.identity_to_id.get(&identity) {
                job_ids.push(id.clone());
            }
        }
        for job_id in job_ids {
            if self
                .records
                .get(&job_id)
                .is_some_and(|r| r.view.status == JobStatus::Queued)
            {
                let _ = self.set_job_priority(&job_id, priority);
            }
        }
        Ok(())
    }

    pub fn enqueue_scrub_for_rescan_imports(
        &mut self,
        imports: &[RescanScrubImport],
    ) -> Result<(), String> {
        if imports.len() > RESCAN_AUTO_SCRUB_MAX {
            return Ok(());
        }
        for item in imports {
            let _ = self.enqueue_scrub_sprite(EnqueueScrubSpriteJob {
                path: item.path.clone(),
                priority: JobPriority::Low,
                anime_title: Some(item.anime_title.clone()),
                episode_label: Some(item.episode_label.clone()),
                follow_up: Vec::new(),
            })?;
        }
        Ok(())
    }

    pub fn enqueue_scrub_sprite(
        &mut self,
        request: EnqueueScrubSpriteJob,
    ) -> Result<EnqueueJobResult, String> {
        let identity = scrub_sprite_identity(&request.path)?;
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.get(&existing_id).is_some_and(|r| {
                r.view.status == JobStatus::Queued
                    && priority_rank(request.priority) > priority_rank(r.view.priority)
            }) {
                self.set_job_priority(&existing_id, request.priority)?;
            }
            return Ok(EnqueueJobResult {
                job_id: Some(existing_id),
                skipped: false,
            });
        }
        if scrub_sprite_is_cached(&request.path)? {
            return Ok(EnqueueJobResult {
                job_id: None,
                skipped: true,
            });
        }

        let desc = build_scrub_desc(&request);
        let id = new_job_id();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            name: "Scrub thumbnails".to_string(),
            desc,
            identity: identity.clone(),
            job_type: "scrub_sprite".to_string(),
            resource_type: JobResourceType::Ffmpeg,
            priority: request.priority,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps: 2,
            },
            step_label: "Queued".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::ScrubSprite {
                path: request.path.clone(),
            },
            follow_ups: request.follow_up,
        };
        self.records.insert(id.clone(), record);
        self.identity_to_id.insert(identity, id.clone());

        if request.priority == JobPriority::High {
            self.try_start_or_queue(&id, true);
            self.pump();
        } else {
            self.queued_ids.push(id.clone());
            self.pump();
        }

        self.emit_snapshot();
        Ok(EnqueueJobResult {
            job_id: Some(id),
            skipped: false,
        })
    }

    fn running_count(&self) -> u32 {
        self.records
            .values()
            .filter(|r| r.view.status == JobStatus::Running)
            .count() as u32
    }

    fn running_count_for_resource_type(&self, resource_type: JobResourceType) -> u32 {
        if resource_type == JobResourceType::None {
            return 0;
        }
        let key = resource_type.as_str();
        self.records
            .values()
            .filter(|r| {
                r.view.status == JobStatus::Running && r.view.resource_type.as_str() == key
            })
            .count() as u32
    }

    fn type_max_for(&self, resource_type: JobResourceType) -> Option<u32> {
        if resource_type == JobResourceType::None {
            return None;
        }
        Some(
            self.type_max_parallel
                .get(resource_type.as_str())
                .copied()
                .unwrap_or(default_type_max_parallel(resource_type)),
        )
    }

    /// Global cap applies to low/medium only (high may bypass). Resource-type caps apply to all.
    fn can_start(&self, job_id: &str) -> bool {
        let Some(record) = self.records.get(job_id) else {
            return false;
        };
        if record.view.status != JobStatus::Queued {
            return false;
        }
        let is_high = record.view.priority == JobPriority::High;
        if !is_high && self.running_count() >= self.max_parallel {
            return false;
        }
        if let Some(type_limit) = self.type_max_for(record.view.resource_type) {
            if self.running_count_for_resource_type(record.view.resource_type) >= type_limit {
                return false;
            }
        }
        true
    }

    fn try_start_or_queue(&mut self, job_id: &str, queue_front: bool) {
        if self.can_start(job_id) {
            self.start_job(job_id);
            return;
        }
        self.ensure_queued(job_id, queue_front);
    }

    fn ensure_queued(&mut self, job_id: &str, front: bool) {
        self.queued_ids.retain(|id| id != job_id);
        if front {
            self.queued_ids.insert(0, job_id.to_string());
        } else {
            self.queued_ids.push(job_id.to_string());
        }
    }

    /// Low/medium jobs respect `max_parallel` against **all** running jobs (including high).
    /// High-priority jobs bypass the global cap but not per-resource-type caps.
    fn pump(&mut self) {
        loop {
            let Some(next_id) = self.pick_startable_queued_id() else {
                break;
            };
            self.start_job(&next_id);
        }
    }

    fn pick_startable_queued_id(&self) -> Option<String> {
        let has_medium = self.queued_ids.iter().any(|id| {
            self.records
                .get(id)
                .is_some_and(|r| r.view.priority == JobPriority::Medium)
        });
        for id in &self.queued_ids {
            let Some(record) = self.records.get(id) else {
                continue;
            };
            if record.view.status != JobStatus::Queued {
                continue;
            }
            if has_medium && record.view.priority == JobPriority::Low {
                continue;
            }
            if self.can_start(id) {
                return Some(id.clone());
            }
        }
        None
    }

    fn start_job(&mut self, job_id: &str) {
        let Some(record) = self.records.get_mut(job_id) else {
            return;
        };
        if record.view.status != JobStatus::Queued {
            return;
        }
        record.view.status = JobStatus::Running;
        record.view.started_at = Some(now_ms());
        record.view.step_label = "Starting".to_string();
        self.queued_ids.retain(|id| id != job_id);

        let cancel = record.cancel.clone();
        let path = match &record.kind {
            JobKind::ScrubSprite { path } => path.clone(),
        };
        let job_id_owned = job_id.to_string();
        let app = self.app.clone();

        super::spawn_scrub_worker(app, job_id_owned, path, cancel);
        self.emit_snapshot();
    }

    pub fn complete_worker(&mut self, job_id: &str, outcome: WorkerOutcome) {
        let path_for_emit = self.records.get(job_id).and_then(|r| match &r.kind {
            JobKind::ScrubSprite { path } => Some(path.clone()),
        });

        match outcome {
            WorkerOutcome::Done(message, scrub_ready) => {
                if let Some(ready) = scrub_ready {
                    emit_scrub_sprite_status(&self.app, scrub_preview::ScrubSpriteStatus::Ready(ready));
                }
                let follow_ups = self
                    .records
                    .get(job_id)
                    .map(|r| r.follow_ups.clone())
                    .unwrap_or_default();
                self.finish_job(job_id, JobStatus::Done, Some(message));
                for follow in follow_ups {
                    let _ = self.enqueue_scrub_sprite(follow);
                }
            }
            WorkerOutcome::Failed(message) => {
                if let Some(path) = path_for_emit {
                    emit_scrub_sprite_status(
                        &self.app,
                        scrub_preview::ScrubSpriteStatus::Unavailable { path },
                    );
                }
                self.finish_job(job_id, JobStatus::Failed, Some(message));
            }
            WorkerOutcome::Canceled => {
                if let Some(path) = path_for_emit {
                    emit_scrub_sprite_status(
                        &self.app,
                        scrub_preview::ScrubSpriteStatus::Unavailable { path },
                    );
                }
                self.finish_job(job_id, JobStatus::Canceled, Some("Canceled".to_string()));
            }
        }
        self.pump();
        self.emit_snapshot();
    }

    fn finish_job(&mut self, job_id: &str, status: JobStatus, completion_message: Option<String>) {
        let Some(record) = self.records.remove(job_id) else {
            return;
        };
        self.identity_to_id.remove(&record.view.identity);

        let mut view = record.view;
        view.status = status;
        view.completion_message = completion_message;
        view.finished_at = Some(now_ms());
        if view.progress.total_steps > 0 && status == JobStatus::Done {
            view.progress.current_step = view.progress.total_steps;
        }

        let finished_event = super::types::JobFinishedEvent {
            job_id: job_id.to_string(),
            identity: view.identity.clone(),
            job_type: view.job_type.clone(),
            status: view.status,
        };

        self.history.push_front(view);
        while self.history.len() > HISTORY_CAP {
            self.history.pop_back();
        }

        let _ = self.app.emit("jobs://finished", finished_event);
    }

    pub fn update_step(&mut self, job_id: &str, current: u32, total: u32, label: &str) {
        if let Some(record) = self.records.get_mut(job_id) {
            record.view.progress.current_step = current;
            record.view.progress.total_steps = total;
            record.view.step_label = label.to_string();
        }
        self.emit_snapshot();
    }
}

pub enum WorkerOutcome {
    Done(String, Option<scrub_preview::ScrubSpriteReady>),
    Failed(String),
    Canceled,
}

pub fn run_scrub_job_worker(
    path: &str,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled;
    }
    match run_scrub_sprite_job(path, cancel, |step, total, label| on_step(step, total, label)) {
        Ok(ready) => WorkerOutcome::Done("Sprite sheet ready".to_string(), Some(ready)),
        Err(e) if e.contains("cancelled") => WorkerOutcome::Canceled,
        Err(e) => WorkerOutcome::Failed(e),
    }
}

fn priority_rank(priority: JobPriority) -> u8 {
    match priority {
        JobPriority::Low => 0,
        JobPriority::Medium => 1,
        JobPriority::High => 2,
    }
}

fn build_scrub_desc(request: &EnqueueScrubSpriteJob) -> String {
    let mut parts = Vec::new();
    if let Some(title) = request.anime_title.as_deref() {
        let t = title.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    if let Some(label) = request.episode_label.as_deref() {
        let l = label.trim();
        if !l.is_empty() {
            parts.push(l.to_string());
        }
    }
    if !parts.is_empty() {
        parts.join(" — ")
    } else {
        Path::new(&request.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&request.path)
            .to_string()
    }
}

fn load_max_parallel(db: &AppDatabase) -> u32 {
    db.with_conn(|conn| {
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![MAX_PARALLEL_SETTING],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(value
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_PARALLEL)
            .clamp(1, MAX_PARALLEL_CAP))
    })
    .unwrap_or(DEFAULT_MAX_PARALLEL)
}

fn type_max_parallel_setting_key(resource_type: &str) -> String {
    format!("{TYPE_MAX_PARALLEL_SETTING_PREFIX}{resource_type}")
}

fn default_type_max_parallel(resource_type: JobResourceType) -> u32 {
    match resource_type {
        JobResourceType::Ffmpeg => DEFAULT_FFMPEG_MAX_PARALLEL,
        JobResourceType::None => MAX_PARALLEL_CAP,
    }
}

fn load_type_max_parallel(db: &AppDatabase) -> HashMap<String, u32> {
    let mut limits = HashMap::new();
    for resource_type in MANAGED_RESOURCE_TYPES {
        let key = type_max_parallel_setting_key(resource_type.as_str());
        let value = db
            .with_conn(|conn| {
                let value: Option<String> = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1",
                        rusqlite::params![key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                Ok(value
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default_type_max_parallel(*resource_type))
                    .clamp(1, MAX_PARALLEL_CAP))
            })
            .unwrap_or(default_type_max_parallel(*resource_type));
        limits.insert(resource_type.as_str().to_string(), value);
    }
    limits
}
