use std::collections::{HashMap, VecDeque};
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
    EnqueueJobResult, EnqueueScrubSpriteJob, JobPriority, JobProgress, JobStatus, JobView, JobsSnapshot,
};

const MAX_PARALLEL_SETTING: &str = "jobs_max_parallel";
const DEFAULT_MAX_PARALLEL: u32 = 2;
const MAX_PARALLEL_CAP: u32 = 8;
const HISTORY_CAP: usize = 200;

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
    queued_ids: Vec<String>,
    records: HashMap<String, JobRecord>,
    identity_to_id: HashMap<String, String>,
    history: VecDeque<JobView>,
}

impl JobManager {
    pub fn new(app: AppHandle, db: &AppDatabase) -> Self {
        let max_parallel = load_max_parallel(db);
        Self {
            app,
            max_parallel,
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
            active_count,
        }
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

    pub fn enqueue_scrub_sprite(
        &mut self,
        request: EnqueueScrubSpriteJob,
    ) -> Result<EnqueueJobResult, String> {
        let identity = scrub_sprite_identity(&request.path)?;
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if let Some(record) = self.records.get_mut(&existing_id) {
                if priority_rank(request.priority) > priority_rank(record.view.priority) {
                    record.view.priority = request.priority;
                    if request.priority == JobPriority::High && record.view.status == JobStatus::Queued {
                        self.start_job(&existing_id);
                    } else if record.view.status == JobStatus::Queued {
                        self.pump();
                    }
                    self.emit_snapshot();
                }
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
            self.start_job(&id);
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

    /// Low/medium jobs respect `max_parallel` against **all** running jobs (including high).
    /// High-priority jobs always start on enqueue and are not gated by the limit.
    fn pump(&mut self) {
        loop {
            if self.running_count() >= self.max_parallel {
                break;
            }
            let Some(next_id) = self.pick_next_queued_id() else {
                break;
            };
            self.start_job(&next_id);
        }
    }

    fn pick_next_queued_id(&self) -> Option<String> {
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
            return Some(id.clone());
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
            WorkerOutcome::Failed(message, _) => {
                if let Some(path) = path_for_emit {
                    emit_scrub_sprite_status(
                        &self.app,
                        scrub_preview::ScrubSpriteStatus::Unavailable { path },
                    );
                }
                self.finish_job(job_id, JobStatus::Failed, Some(message));
            }
            WorkerOutcome::Canceled(_) => {
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
    Failed(String, Option<String>),
    Canceled(Option<String>),
}

pub fn run_scrub_job_worker(
    path: &str,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled(Some(path.to_string()));
    }
    match run_scrub_sprite_job(path, cancel, |step, total, label| on_step(step, total, label)) {
        Ok(ready) => WorkerOutcome::Done(
            format!("Sprite sheet ready for {}", ready.path),
            Some(ready),
        ),
        Err(e) if e.contains("cancelled") => WorkerOutcome::Canceled(Some(path.to_string())),
        Err(e) => WorkerOutcome::Failed(e, Some(path.to_string())),
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
    if parts.is_empty() {
        request.path.clone()
    } else {
        format!("{} — {}", parts.join(" — "), request.path)
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
