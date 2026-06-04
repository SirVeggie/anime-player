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
    EnqueueEpisodePageScrubSprites, EnqueueJobResult, EnqueueOpEdChromaAnimeJob,
    EnqueueOpEdDetectJob, EnqueueScrubSpriteJob, JobPrerequisiteView, JobPriority, JobProgress,
    JobResourceType, JobStatus, JobView, JobsSnapshot, TypeMaxParallel,
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
const HISTORY_CAP: usize = 200;
/// Coalesce rapid `jobs://updated` emissions so the WebView is not flooded during parallel work.
const SNAPSHOT_EMIT_MIN_INTERVAL_MS: u64 = 250;
/// Cap prerequisite pills in emitted snapshots (full count stays in `prerequisite_total`).
const SNAPSHOT_WAITING_FOR_CAP: usize = 8;

const MANAGED_RESOURCE_TYPES: &[JobResourceType] =
    &[JobResourceType::Ffmpeg, JobResourceType::Chroma];

static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn now_ms() -> u64 {
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
    /// Last chroma start time per volume key (`G:/`) for HDD deferral gaps.
    last_chroma_start_on_volume: HashMap<String, u64>,
    chroma_disk_poll_wakeup_armed: bool,
    snapshot_emit_wakeup_armed: bool,
    last_snapshot_emit_ms: u64,
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
            last_chroma_start_on_volume: HashMap::new(),
            chroma_disk_poll_wakeup_armed: false,
            snapshot_emit_wakeup_armed: false,
            last_snapshot_emit_ms: 0,
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

    pub fn emit_snapshot(&mut self) {
        self.schedule_snapshot_emit(false);
    }

    pub fn emit_snapshot_now(&mut self) {
        self.snapshot_emit_wakeup_armed = false;
        self.last_snapshot_emit_ms = now_ms();
        let _ = self.app.emit("jobs://updated", self.snapshot());
    }

    fn schedule_snapshot_emit(&mut self, immediate: bool) {
        if immediate {
            self.emit_snapshot_now();
            return;
        }
        let now = now_ms();
        if now.saturating_sub(self.last_snapshot_emit_ms) >= SNAPSHOT_EMIT_MIN_INTERVAL_MS {
            self.emit_snapshot_now();
            return;
        }
        if self.snapshot_emit_wakeup_armed {
            return;
        }
        self.snapshot_emit_wakeup_armed = true;
        let delay = SNAPSHOT_EMIT_MIN_INTERVAL_MS
            .saturating_sub(now.saturating_sub(self.last_snapshot_emit_ms))
            .max(1);
        super::schedule_snapshot_emit_after_ms(&self.app, delay);
    }

    pub fn on_snapshot_emit_wakeup(&mut self) {
        self.snapshot_emit_wakeup_armed = false;
        self.emit_snapshot_now();
    }

    fn view_with_waiting_for(&self, record: &JobRecord) -> JobView {
        let mut view = record.view.clone();
        let mut waiting_for = self.pending_prerequisites(record);
        if waiting_for.len() > SNAPSHOT_WAITING_FOR_CAP {
            waiting_for.truncate(SNAPSHOT_WAITING_FOR_CAP);
        }
        view.waiting_for = waiting_for;
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
        if self.set_job_priority_inner(job_id, priority)? {
            self.emit_snapshot();
        }
        Ok(())
    }

    /// Like [`set_job_priority`] but never emits; batch callers emit once at the end.
    fn set_job_priority_inner(&mut self, job_id: &str, priority: JobPriority) -> Result<bool, String> {
        let Some(record) = self.records.get(job_id) else {
            return Err(format!("job not found: {job_id}"));
        };
        if record.view.status != JobStatus::Queued {
            return Err("only queued jobs can change priority".to_string());
        }
        if record.view.priority == priority {
            return Ok(false);
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
        Ok(true)
    }

    pub fn set_scrub_sprite_priority_for_paths(
        &mut self,
        paths: &[String],
        priority: JobPriority,
    ) -> Result<(), String> {
        let mut changed = false;
        for path in paths {
            let identity = scrub_sprite_identity(path)?;
            let Some(job_id) = self.identity_to_id.get(&identity).cloned() else {
                continue;
            };
            if self
                .records
                .get(&job_id)
                .is_some_and(|r| r.view.status == JobStatus::Queued)
            {
                if self.set_job_priority_inner(&job_id, priority)? {
                    changed = true;
                }
            }
        }
        if changed {
            self.emit_snapshot_now();
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
        self.enqueue_scrub_sprite_inner(request, true)
    }

    pub fn enqueue_episode_page_scrub_sprites(
        &mut self,
        request: EnqueueEpisodePageScrubSprites,
    ) -> Result<(), String> {
        for item in request.episodes {
            let _ = self.enqueue_scrub_sprite_inner(
                EnqueueScrubSpriteJob {
                    path: item.path,
                    priority: request.priority,
                    anime_title: request.anime_title.clone(),
                    episode_label: item.episode_label,
                    follow_up: Vec::new(),
                },
                false,
            )?;
        }
        self.finish_scheduling_batch();
        Ok(())
    }

    fn enqueue_scrub_sprite_inner(
        &mut self,
        request: EnqueueScrubSpriteJob,
        flush_scheduling: bool,
    ) -> Result<EnqueueJobResult, String> {
        let identity = scrub_sprite_identity(&request.path)?;
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.get(&existing_id).is_some_and(|r| {
                r.view.status == JobStatus::Queued
                    && priority_rank(request.priority) > priority_rank(r.view.priority)
            }) {
                let _ = self.set_job_priority_inner(&existing_id, request.priority)?;
            }
            if flush_scheduling {
                self.finish_scheduling_batch();
            }
            return Ok(EnqueueJobResult {
                job_id: Some(existing_id),
                skipped: false,
            });
        }
        if scrub_sprite_is_cached(&request.path)? {
            if flush_scheduling {
                self.finish_scheduling_batch();
            }
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
        } else {
            self.queued_ids.push(id.clone());
        }

        if flush_scheduling {
            self.finish_scheduling_batch();
        }
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
                false,
            )?;
            if let Some(id) = result.job_id {
                last_job_id = Some(id);
                if !result.skipped {
                    any_queued = true;
                }
            }
        }
        self.finish_op_ed_enqueue_batch();
        Ok(EnqueueJobResult {
            job_id: last_job_id,
            skipped: !any_queued,
        })
    }

    fn finish_op_ed_enqueue_batch(&mut self) {
        self.finish_scheduling_batch();
    }

    /// Run scheduler after enqueueing work. Refreshes disk-busy cache **before** taking
    /// further scheduler steps — never call WMI from inside [`Self::pump`] (holds the mutex).
    fn finish_scheduling_batch(&mut self) {
        self.refresh_disk_busy_if_needed();
        self.pump();
        self.emit_snapshot_now();
    }

    fn refresh_disk_busy_if_needed(&self) {
        if self.has_pending_chroma_work() || self.has_queued_chroma_blocked_by_disk_busy() {
            disk_volume::refresh_disk_busy_cache(now_ms());
        }
    }

    fn enqueue_op_ed_chroma_episode(
        &mut self,
        _db: &AppDatabase,
        ep: &OpEdEpisode,
        priority: JobPriority,
        anime_title: Option<&str>,
        flush_scheduling: bool,
    ) -> Result<EnqueueJobResult, String> {
        if op_ed::full_episode_fingerprint_cached_for_enqueue(ep)? {
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
        } else {
            self.queued_ids.push(id.clone());
        }

        if flush_scheduling {
            self.finish_op_ed_enqueue_batch();
        }

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
            if op_ed::full_episode_fingerprint_cached_for_enqueue(ep)? {
                continue;
            }
            let chroma = self.enqueue_op_ed_chroma_episode(
                db,
                ep,
                request.priority,
                request.anime_title.as_deref(),
                false,
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
            resource_type: JobResourceType::None,
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
        } else {
            self.queued_ids.push(id.clone());
        }

        self.finish_op_ed_enqueue_batch();
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

    fn last_chroma_start_ms_for_path(&self, path: &str) -> Option<u64> {
        let volume = disk_volume::volume_key_for_path(path)?;
        self.last_chroma_start_on_volume.get(&volume).copied()
    }

    fn is_chroma_disk_busy_blocked(&self, job_id: &str) -> bool {
        let Some(record) = self.records.get(job_id) else {
            return false;
        };
        if record.view.resource_type != JobResourceType::Chroma {
            return false;
        }
        let Some(path) = self.chroma_episode_path(job_id) else {
            return false;
        };
        if !disk_volume::path_requires_chroma_stagger(path) {
            return false;
        }
        disk_volume::chroma_start_deferred(
            path,
            self.last_chroma_start_ms_for_path(path),
            now_ms(),
        )
    }

    fn record_chroma_volume_start(&mut self, path: &str) {
        if !disk_volume::path_requires_chroma_stagger(path) {
            return;
        }
        if let Some(volume) = disk_volume::volume_key_for_path(path) {
            self.last_chroma_start_on_volume
                .insert(volume, now_ms());
        }
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
        !self.is_chroma_disk_busy_blocked(job_id)
    }

    fn has_queued_chroma_blocked_by_disk_busy(&self) -> bool {
        self.queued_ids.iter().any(|id| {
            self.records.get(id).is_some_and(|r| {
                r.view.resource_type == JobResourceType::Chroma
                    && r.view.status == JobStatus::Queued
                    && self.can_start_without_stagger(id)
                    && self.is_chroma_disk_busy_blocked(id)
            })
        })
    }

    fn has_pending_chroma_work(&self) -> bool {
        self.records.values().any(|r| {
            r.view.resource_type == JobResourceType::Chroma
                && matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
        })
    }

    fn schedule_chroma_disk_poll_if_needed(&mut self) {
        if self.chroma_disk_poll_wakeup_armed || !self.has_queued_chroma_blocked_by_disk_busy() {
            return;
        }
        let now = now_ms();
        let delay_ms = self
            .queued_ids
            .iter()
            .filter_map(|id| {
                let path = self.chroma_episode_path(id)?;
                if !self.is_chroma_disk_busy_blocked(id) {
                    return None;
                }
                Some(disk_volume::chroma_defer_retry_ms(
                    path,
                    self.last_chroma_start_ms_for_path(path),
                    now,
                ))
            })
            .min()
            .unwrap_or(disk_volume::CHROMA_HDD_POLL_MS);
        self.chroma_disk_poll_wakeup_armed = true;
        super::schedule_job_pump_after_ms(&self.app, delay_ms);
    }

    pub fn on_chroma_stagger_wakeup(&mut self) {
        self.chroma_disk_poll_wakeup_armed = false;
        self.refresh_disk_busy_if_needed();
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
        self.schedule_chroma_disk_poll_if_needed();
    }

    fn pick_startable_queued_id(&self) -> Option<String> {
        pick_startable_from_queue(&self.queued_ids, |id| {
            self.records
                .get(id)
                .is_some_and(|r| r.view.status == JobStatus::Queued && self.can_start(id))
        }, |id| {
            self.records
                .get(id)
                .map(|r| r.view.priority)
                .unwrap_or(JobPriority::Low)
        })
    }

    fn start_job(&mut self, job_id: &str) {
        let chroma_path = self
            .chroma_episode_path(job_id)
            .map(str::to_string);
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
        let kind = record.kind.clone();
        let job_id_owned = job_id.to_string();
        let app = self.app.clone();

        if let Some(path) = chroma_path.as_deref() {
            self.record_chroma_volume_start(path);
        }

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

/// Low-priority jobs wait only while some queued medium job can actually start.
fn pick_startable_from_queue<'a>(
    queued_ids: &'a [String],
    can_start: impl Fn(&'a str) -> bool,
    job_priority: impl Fn(&'a str) -> JobPriority,
) -> Option<String> {
    let block_low = queued_ids.iter().any(|id| {
        job_priority(id) == JobPriority::Medium && can_start(id)
    });
    for id in queued_ids {
        if block_low && job_priority(id) == JobPriority::Low {
            continue;
        }
        if can_start(id) {
            return Some(id.clone());
        }
    }
    None
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

#[cfg(test)]
mod scheduler_tests {
    use super::*;

    fn id(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn low_runs_when_medium_is_queued_but_not_startable() {
        let queue = vec![id("detect"), id("scrub-low")];
        let can = |id: &str| id == "scrub-low";
        let pri = |id: &str| {
            if id == "detect" {
                JobPriority::Medium
            } else {
                JobPriority::Low
            }
        };
        assert_eq!(
            pick_startable_from_queue(&queue, can, pri).as_deref(),
            Some("scrub-low")
        );
    }

    #[test]
    fn low_waits_while_a_startable_medium_is_queued() {
        let queue = vec![id("chroma"), id("scrub-low")];
        let can = |_| true;
        let pri = |id: &str| {
            if id == "chroma" {
                JobPriority::Medium
            } else {
                JobPriority::Low
            }
        };
        assert_eq!(
            pick_startable_from_queue(&queue, can, pri).as_deref(),
            Some("chroma")
        );
    }

    #[test]
    fn picks_first_startable_medium_before_low() {
        let queue = vec![id("detect-blocked"), id("chroma"), id("scrub-low")];
        let can = |id: &str| id != "detect-blocked";
        let pri = |id: &str| match id {
            "scrub-low" => JobPriority::Low,
            _ => JobPriority::Medium,
        };
        assert_eq!(
            pick_startable_from_queue(&queue, can, pri).as_deref(),
            Some("chroma")
        );
    }
}
