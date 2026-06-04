use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::AppDatabase;
use crate::disk_volume;
use crate::op_ed::{self, op_ed_chroma_job_identity, op_ed_job_identity, OpEdEpisode};
use crate::scrub_preview::{
    self, emit_scrub_sprite_status, run_scrub_sprite_job, scrub_sprite_identity, scrub_sprite_is_cached,
};

use super::types::{
    EnqueueJobResult, EnqueueOpEdChromaAnimeJob, EnqueueOpEdDetectJob, EnqueueScrubSpriteJob,
    JobPrerequisiteView, JobPriority, JobProgress, JobResourceType, JobStatus, JobView,
    JobsSnapshot, TypeMaxParallel,
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
const DEFAULT_CHROMA_MAX_PARALLEL: u32 = 4;
const MAX_PARALLEL_CAP: u32 = 20;
const CHROMA_START_STAGGER_MS: u64 = 2000;
const HISTORY_CAP: usize = 200;

const MANAGED_RESOURCE_TYPES: &[JobResourceType] =
    &[JobResourceType::Ffmpeg, JobResourceType::Chroma];

static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn alloc_job_ids() -> (String, u32) {
    let n = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    (format!("job-{n}"), n as u32)
}

#[derive(Debug, Clone)]
enum JobKind {
    ScrubSprite { path: String },
    OpEdChroma {
        episode: OpEdEpisode,
    },
    OpEdDetect {
        anime_id: i64,
    },
}

struct JobRecord {
    view: JobView,
    cancel: Arc<AtomicBool>,
    kind: JobKind,
    follow_ups: Vec<EnqueueScrubSpriteJob>,
    prerequisite_job_ids: Vec<String>,
}

pub struct JobManager {
    app: AppHandle,
    max_parallel: u32,
    type_max_parallel: HashMap<String, u32>,
    queued_ids: Vec<String>,
    records: HashMap<String, JobRecord>,
    identity_to_id: HashMap<String, String>,
    history: VecDeque<JobView>,
    /// Wall-clock ms when the most recent chroma job was started (for HDD stagger).
    last_chroma_start_ms: Option<u64>,
    chroma_stagger_wakeup_armed: bool,
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
            last_chroma_start_ms: None,
            chroma_stagger_wakeup_armed: false,
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
            .map(|r| self.view_with_waiting_for(r))
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

    fn view_with_waiting_for(&self, record: &JobRecord) -> JobView {
        let mut view = record.view.clone();
        view.waiting_for = self.pending_prerequisites(record);
        view.prerequisite_total = record.prerequisite_job_ids.len() as u32;
        if !view.waiting_for.is_empty() && view.status == JobStatus::Queued {
            view.step_label = "Waiting for prerequisites".to_string();
        }
        view
    }

    fn pending_prerequisites(&self, record: &JobRecord) -> Vec<JobPrerequisiteView> {
        record
            .prerequisite_job_ids
            .iter()
            .filter_map(|prereq_id| {
                let prereq = self.records.get(prereq_id)?;
                if matches!(prereq.view.status, JobStatus::Done) {
                    return None;
                }
                Some(JobPrerequisiteView {
                    job_id: prereq_id.clone(),
                    short_id: prereq.view.short_id,
                })
            })
            .collect()
    }

    fn prerequisites_met(&self, job_id: &str) -> bool {
        let Some(record) = self.records.get(job_id) else {
            return false;
        };
        for prereq_id in &record.prerequisite_job_ids {
            if self.records.contains_key(prereq_id) {
                return false;
            }
            let satisfied = self
                .history
                .iter()
                .any(|h| h.id == *prereq_id && h.status == JobStatus::Done);
            if !satisfied {
                return false;
            }
        }
        true
    }

    fn fail_queued_job(&mut self, job_id: &str, message: String) {
        let Some(record) = self.records.get(job_id) else {
            return;
        };
        if record.view.status != JobStatus::Queued {
            return;
        }
        self.queued_ids.retain(|id| id != job_id);
        self.finish_job(job_id, JobStatus::Failed, Some(message));
    }

    fn on_prerequisite_finished(&mut self, prereq_id: &str, status: JobStatus, prereq_short: u32) {
        if status == JobStatus::Done {
            return;
        }
        let reason = match status {
            JobStatus::Canceled => format!("Prerequisite #{prereq_short} was canceled"),
            JobStatus::Failed => format!("Prerequisite #{prereq_short} failed"),
            _ => format!("Prerequisite #{prereq_short} did not complete"),
        };
        let dependents: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.prerequisite_job_ids.contains(&prereq_id.to_string())
                    && r.view.status == JobStatus::Queued
            })
            .map(|(id, _)| id.clone())
            .collect();
        for dep_id in dependents {
            self.fail_queued_job(&dep_id, reason.clone());
        }
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
        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            short_id,
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
            waiting_for: Vec::new(),
            prerequisite_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::ScrubSprite {
                path: request.path.clone(),
            },
            follow_ups: request.follow_up,
            prerequisite_job_ids: Vec::new(),
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

    pub fn enqueue_op_ed_chroma_for_anime(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdChromaAnimeJob,
    ) -> Result<EnqueueJobResult, String> {
        let episodes = db.with_conn(|conn| op_ed::list_anime_episodes(conn, request.anime_id))?;
        let mut last_job_id: Option<String> = None;
        let mut any_queued = false;
        for ep in episodes {
            let result = self.enqueue_op_ed_chroma_episode(
                db,
                &ep,
                request.priority,
                request.anime_title.as_deref(),
            )?;
            if let Some(id) = result.job_id {
                last_job_id = Some(id);
                if !result.skipped {
                    any_queued = true;
                }
            }
        }
        Ok(EnqueueJobResult {
            job_id: last_job_id,
            skipped: !any_queued,
        })
    }

    fn enqueue_op_ed_chroma_episode(
        &mut self,
        _db: &AppDatabase,
        ep: &OpEdEpisode,
        priority: JobPriority,
        anime_title: Option<&str>,
    ) -> Result<EnqueueJobResult, String> {
        if op_ed::full_episode_fingerprint_cached(ep)? {
            return Ok(EnqueueJobResult {
                job_id: None,
                skipped: true,
            });
        }

        let identity = op_ed_chroma_job_identity(ep.id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.get(&existing_id).is_some_and(|r| {
                matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
            }) {
                if self.records.get(&existing_id).is_some_and(|r| {
                    r.view.status == JobStatus::Queued
                        && priority_rank(priority) > priority_rank(r.view.priority)
                }) {
                    self.set_job_priority(&existing_id, priority)?;
                }
                return Ok(EnqueueJobResult {
                    job_id: Some(existing_id),
                    skipped: false,
                });
            }
        }

        let desc = build_chroma_desc(ep, anime_title);
        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            short_id,
            name: "Fingerprint audio".to_string(),
            desc,
            identity: identity.clone(),
            job_type: "op_ed_chroma".to_string(),
            resource_type: JobResourceType::Chroma,
            priority,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps: 1,
            },
            step_label: "Queued".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            waiting_for: Vec::new(),
            prerequisite_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdChroma {
                episode: ep.clone(),
            },
            follow_ups: Vec::new(),
            prerequisite_job_ids: Vec::new(),
        };
        self.records.insert(id.clone(), record);
        self.identity_to_id.insert(identity, id.clone());

        if priority == JobPriority::High {
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

    pub fn enqueue_op_ed_detect(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdDetectJob,
    ) -> Result<EnqueueJobResult, String> {
        let identity = op_ed_job_identity(request.anime_id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.get(&existing_id).is_some_and(|r| {
                matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
            }) {
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
        }

        let desc = request
            .anime_title
            .as_deref()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| format!("Anime #{}", request.anime_id));

        let episodes = db.with_conn(|conn| op_ed::list_anime_episodes(conn, request.anime_id))?;
        let mut prerequisite_job_ids = Vec::new();
        for ep in &episodes {
            if op_ed::full_episode_fingerprint_cached(ep)? {
                continue;
            }
            let chroma = self.enqueue_op_ed_chroma_episode(
                db,
                ep,
                request.priority,
                request.anime_title.as_deref(),
            )?;
            if let Some(job_id) = chroma.job_id {
                prerequisite_job_ids.push(job_id);
            }
        }

        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let total_steps = 100;
        let view = JobView {
            id: id.clone(),
            short_id,
            name: "Detect OP/ED".to_string(),
            desc,
            identity: identity.clone(),
            job_type: "op_ed_detect".to_string(),
            resource_type: JobResourceType::Ffmpeg,
            priority: request.priority,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps,
            },
            step_label: "Queued".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            waiting_for: Vec::new(),
            prerequisite_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdDetect {
                anime_id: request.anime_id,
            },
            follow_ups: Vec::new(),
            prerequisite_job_ids,
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

    fn chroma_episode_path(&self, job_id: &str) -> Option<&str> {
        self.records.get(job_id).and_then(|r| match &r.kind {
            JobKind::OpEdChroma { episode } => Some(episode.path.as_str()),
            _ => None,
        })
    }

    fn chroma_job_requires_start_stagger(&self, job_id: &str) -> bool {
        let Some(path) = self.chroma_episode_path(job_id) else {
            return true;
        };
        disk_volume::path_requires_chroma_stagger(path)
    }

    fn chroma_stagger_remaining_ms(&self) -> u64 {
        let Some(last) = self.last_chroma_start_ms else {
            return 0;
        };
        let elapsed = now_ms().saturating_sub(last);
        CHROMA_START_STAGGER_MS.saturating_sub(elapsed)
    }

    fn is_chroma_stagger_blocked(&self, job_id: &str) -> bool {
        let Some(record) = self.records.get(job_id) else {
            return true;
        };
        if record.view.resource_type != JobResourceType::Chroma {
            return false;
        }
        if !self.chroma_job_requires_start_stagger(job_id) {
            return false;
        }
        self.chroma_stagger_remaining_ms() > 0
    }

    /// Global cap applies to low/medium only (high may bypass). Resource-type caps apply to all.
    fn can_start_without_stagger(&self, job_id: &str) -> bool {
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
        self.prerequisites_met(job_id)
    }

    fn can_start(&self, job_id: &str) -> bool {
        if !self.can_start_without_stagger(job_id) {
            return false;
        }
        !self.is_chroma_stagger_blocked(job_id)
    }

    fn has_queued_chroma_blocked_by_stagger(&self) -> bool {
        if self.chroma_stagger_remaining_ms() == 0 {
            return false;
        }
        self.queued_ids.iter().any(|id| {
            self.records.get(id).is_some_and(|r| {
                r.view.resource_type == JobResourceType::Chroma
                    && r.view.status == JobStatus::Queued
                    && self.chroma_job_requires_start_stagger(id)
                    && self.can_start_without_stagger(id)
            })
        })
    }

    fn schedule_chroma_stagger_wakeup_if_needed(&mut self) {
        if self.chroma_stagger_wakeup_armed || !self.has_queued_chroma_blocked_by_stagger() {
            return;
        }
        let delay_ms = self.chroma_stagger_remaining_ms().max(1);
        self.chroma_stagger_wakeup_armed = true;
        super::schedule_job_pump_after_ms(&self.app, delay_ms);
    }

    pub fn on_chroma_stagger_wakeup(&mut self) {
        self.chroma_stagger_wakeup_armed = false;
        self.pump();
        self.emit_snapshot();
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
        self.schedule_chroma_stagger_wakeup_if_needed();
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
        let track_chroma_stagger = self.chroma_job_requires_start_stagger(job_id)
            && self
                .records
                .get(job_id)
                .is_some_and(|r| r.view.resource_type == JobResourceType::Chroma);
        let Some(record) = self.records.get_mut(job_id) else {
            return;
        };
        if record.view.status != JobStatus::Queued {
            return;
        }
        record.view.status = JobStatus::Running;
        let started_at = now_ms();
        record.view.started_at = Some(started_at);
        record.view.step_label = "Starting".to_string();
        if track_chroma_stagger {
            self.last_chroma_start_ms = Some(started_at);
        }
        self.queued_ids.retain(|id| id != job_id);

        let cancel = record.cancel.clone();
        let kind = record.kind.clone();
        let job_id_owned = job_id.to_string();
        let app = self.app.clone();

        match kind {
            JobKind::ScrubSprite { path } => {
                super::spawn_scrub_worker(app, job_id_owned, path, cancel);
            }
            JobKind::OpEdChroma { episode } => {
                super::spawn_op_ed_chroma_worker(app, job_id_owned, episode, cancel);
            }
            JobKind::OpEdDetect { anime_id } => {
                super::spawn_op_ed_worker(app, job_id_owned, anime_id, cancel);
            }
        }
        self.emit_snapshot();
    }

    pub fn complete_worker(&mut self, job_id: &str, outcome: WorkerOutcome) {
        let scrub_path_for_emit = self.records.get(job_id).and_then(|r| match &r.kind {
            JobKind::ScrubSprite { path } => Some(path.clone()),
            JobKind::OpEdChroma { .. } | JobKind::OpEdDetect { .. } => None,
        });

        let emit_op_ed_updated = self.records.get(job_id).is_some_and(|r| {
            matches!(r.kind, JobKind::OpEdDetect { .. })
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
                if emit_op_ed_updated {
                    let _ = self.app.emit("op-ed://analysis-updated", ());
                }
            }
            WorkerOutcome::Failed(message) => {
                if let Some(path) = scrub_path_for_emit {
                    emit_scrub_sprite_status(
                        &self.app,
                        scrub_preview::ScrubSpriteStatus::Unavailable { path },
                    );
                }
                self.finish_job(job_id, JobStatus::Failed, Some(message));
            }
            WorkerOutcome::Canceled => {
                if let Some(path) = scrub_path_for_emit {
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
        let prereq_short = record.view.short_id;

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

        self.on_prerequisite_finished(job_id, status, prereq_short);

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

pub fn run_op_ed_job_worker(
    app: &AppHandle,
    anime_id: i64,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled;
    }
    let db = app.state::<AppDatabase>();
    match op_ed::run_op_ed_detect_job(&db, anime_id, cancel, on_step) {
        Ok(()) => WorkerOutcome::Done("OP/ED analysis complete".to_string(), None),
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
        JobResourceType::Chroma => DEFAULT_CHROMA_MAX_PARALLEL,
        JobResourceType::None => MAX_PARALLEL_CAP,
    }
}

fn build_chroma_desc(ep: &OpEdEpisode, anime_title: Option<&str>) -> String {
    let file_label = Path::new(&ep.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&ep.path);
    if let Some(title) = anime_title.map(str::trim).filter(|t| !t.is_empty()) {
        format!("{title} — {file_label}")
    } else {
        file_label.to_string()
    }
}

pub fn run_op_ed_chroma_job_worker(
    ep: &OpEdEpisode,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled;
    }
    match op_ed::run_episode_chroma_fingerprint(ep, cancel, on_step) {
        Ok(()) => WorkerOutcome::Done("Fingerprint ready".to_string(), None),
        Err(e) if e.contains("cancelled") => WorkerOutcome::Canceled,
        Err(e) => WorkerOutcome::Failed(e),
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
