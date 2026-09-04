use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::AppDatabase;
use crate::disk_volume;
use crate::op_ed::{
    self, op_ed_chroma_job_identity, op_ed_detect_batch_job_identity,
    op_ed_detect_job_identity_prefix, OpEdDetectJobOptions, OpEdEpisode,
};
use crate::scrub_preview::{
    self, emit_scrub_sprite_status, run_scrub_sprite_job, scrub_sprite_identity,
    scrub_sprite_is_cached,
};

use super::types::{
    EnqueueEpisodePageScrubSprites, EnqueueJobResult, EnqueueOpEdChromaAnimeJob,
    EnqueueOpEdChromaEpisodeJob, EnqueueOpEdDetectJob, EnqueueScrubSpriteJob, JobPrerequisiteView,
    JobPriority, JobProgress, JobResourceType, JobStatus, JobView, JobsSnapshot,
    OpEdAnalysisUpdatedEvent, TypeMaxParallel,
};

/// Episode newly imported during `rescan_library` (auto scrub enqueue).
#[derive(Debug, Clone)]
pub struct RescanScrubImport {
    pub path: String,
    pub anime_title: String,
    pub episode_label: String,
}

/// When a rescan imports at most this many episodes, queue scrub thumbnails and OP/ED detect for them.
pub const RESCAN_AUTO_SCRUB_MAX: usize = 50;

/// Title affected by a small rescan import batch (OP/ED detect is per anime, not per file).
#[derive(Debug, Clone)]
pub struct RescanOpEdImport {
    pub anime_id: i64,
    pub anime_title: String,
}

const MAX_PARALLEL_SETTING: &str = "jobs_max_parallel";
const TYPE_MAX_PARALLEL_SETTING_PREFIX: &str = "jobs_max_parallel_type_";
const DEFAULT_MAX_PARALLEL: u32 = 12;
const DEFAULT_FFMPEG_MAX_PARALLEL: u32 = 1;
const DEFAULT_CHROMA_MAX_PARALLEL: u32 = 12;
const MAX_PARALLEL_CAP: u32 = 20;
const HISTORY_CAP: usize = 200;
/// Coalesce rapid `jobs://updated` emissions so the WebView is not flooded during parallel work.
const SNAPSHOT_EMIT_MIN_INTERVAL_MS: u64 = 250;
/// Coalesce `op-ed://analysis-updated` when many per-episode rematch jobs finish together.
const OP_ED_ANALYSIS_UPDATED_EMIT_MIN_INTERVAL_MS: u64 = 500;
/// Cap prerequisite pills in emitted snapshots (`prerequisite_pending` stays accurate).
const SNAPSHOT_WAITING_FOR_CAP: usize = 8;
/// Keep startup rescan preprocessing from launching the configured maximum
/// number of media processes while WebView2 is still settling.
const RESCAN_LOW_PRIORITY_CAP: u32 = 2;
const RESCAN_LOW_PRIORITY_THROTTLE_MS: u64 = 30_000;

const MANAGED_RESOURCE_TYPES: &[JobResourceType] =
    &[JobResourceType::Ffmpeg, JobResourceType::Chroma];

static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn rescan_throttle_blocks_low_priority(
    priority: JobPriority,
    now: u64,
    throttle_until: u64,
    running_low_priority: u32,
) -> bool {
    priority == JobPriority::Low
        && now < throttle_until
        && running_low_priority >= RESCAN_LOW_PRIORITY_CAP
}

fn alloc_job_ids() -> (String, u32) {
    let n = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    (format!("job-{n}"), n as u32)
}

#[derive(Debug, Clone)]
enum JobKind {
    ScrubSprite {
        path: String,
    },
    OpEdChroma {
        episode: OpEdEpisode,
        /// Fingerprint was already on disk at job start — do not stagger the next chroma.
        skip_volume_stagger: bool,
        cache_requirement: op_ed::OpEdChromaCacheRequirement,
    },
    OpEdDetect {
        anime_id: i64,
        episode_ids: Vec<i64>,
        options: OpEdDetectJobOptions,
    },
    OpEdManualRematch {
        anime_id: i64,
        episode_id: i64,
    },
    /// Episode-page progress shell; waits on per-episode rematch jobs then finishes immediately.
    OpEdManualRematchSummary {
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
    rescan_throttle_until_ms: u64,
    rescan_throttle_wakeup_armed: bool,
    snapshot_emit_wakeup_armed: bool,
    last_snapshot_emit_ms: u64,
    op_ed_analysis_emit_wakeup_armed: bool,
    last_op_ed_analysis_emit_ms: u64,
    pending_op_ed_analysis_anime_ids: HashSet<i64>,
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
            rescan_throttle_until_ms: 0,
            rescan_throttle_wakeup_armed: false,
            snapshot_emit_wakeup_armed: false,
            last_snapshot_emit_ms: 0,
            op_ed_analysis_emit_wakeup_armed: false,
            last_op_ed_analysis_emit_ms: 0,
            pending_op_ed_analysis_anime_ids: HashSet::new(),
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

    pub fn on_op_ed_analysis_emit_wakeup(&mut self) {
        self.op_ed_analysis_emit_wakeup_armed = false;
        self.emit_op_ed_analysis_updated_now();
    }

    fn schedule_op_ed_analysis_updated(&mut self, anime_id: i64) {
        self.pending_op_ed_analysis_anime_ids.insert(anime_id);
        let now = now_ms();
        if now.saturating_sub(self.last_op_ed_analysis_emit_ms)
            >= OP_ED_ANALYSIS_UPDATED_EMIT_MIN_INTERVAL_MS
        {
            self.emit_op_ed_analysis_updated_now();
            return;
        }
        if self.op_ed_analysis_emit_wakeup_armed {
            return;
        }
        self.op_ed_analysis_emit_wakeup_armed = true;
        let delay = OP_ED_ANALYSIS_UPDATED_EMIT_MIN_INTERVAL_MS
            .saturating_sub(now.saturating_sub(self.last_op_ed_analysis_emit_ms))
            .max(1);
        super::schedule_op_ed_analysis_emit_after_ms(&self.app, delay);
    }

    fn emit_op_ed_analysis_updated_now(&mut self) {
        self.op_ed_analysis_emit_wakeup_armed = false;
        self.last_op_ed_analysis_emit_ms = now_ms();
        let anime_ids: Vec<i64> = self.pending_op_ed_analysis_anime_ids.drain().collect();
        for anime_id in anime_ids {
            let _ = self.app.emit(
                "op-ed://analysis-updated",
                OpEdAnalysisUpdatedEvent { anime_id },
            );
        }
    }

    fn op_ed_job_anime_id(kind: &JobKind) -> Option<i64> {
        match kind {
            JobKind::OpEdDetect { anime_id, .. } => Some(*anime_id),
            JobKind::OpEdManualRematch { anime_id, .. } => Some(*anime_id),
            JobKind::OpEdManualRematchSummary { .. } => None,
            _ => None,
        }
    }

    fn view_with_waiting_for(&self, record: &JobRecord) -> JobView {
        let mut view = record.view.clone();
        let pending = self.pending_prerequisites(record);
        view.prerequisite_pending = pending.len() as u32;
        let mut waiting_for = pending;
        if waiting_for.len() > SNAPSHOT_WAITING_FOR_CAP {
            waiting_for.truncate(SNAPSHOT_WAITING_FOR_CAP);
        }
        view.waiting_for = waiting_for;
        view.prerequisite_total = record.prerequisite_job_ids.len() as u32;
        let (prereq_current, prereq_total) = self.prerequisite_progress_steps(record);
        view.prerequisite_progress_current = prereq_current;
        view.prerequisite_progress_total = prereq_total;
        if !view.waiting_for.is_empty() && view.status == JobStatus::Queued {
            view.step_label = "Waiting for prerequisites".to_string();
        }
        view
    }

    /// Two steps per prerequisite: one when it starts, one when it completes.
    fn prerequisite_progress_steps(&self, record: &JobRecord) -> (u32, u32) {
        let total = record.prerequisite_job_ids.len() as u32 * 2;
        if total == 0 {
            return (0, 0);
        }
        let mut current = 0u32;
        for prereq_id in &record.prerequisite_job_ids {
            if let Some(prereq) = self.records.get(prereq_id) {
                if prereq.view.status == JobStatus::Running {
                    current += 1;
                }
                continue;
            }
            if self
                .history
                .iter()
                .any(|h| h.id == *prereq_id && h.status == JobStatus::Done)
            {
                current += 2;
            }
        }
        (current, total)
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
        let child_ids: Vec<String> = self
            .records
            .get(job_id)
            .filter(|r| matches!(r.kind, JobKind::OpEdManualRematchSummary { .. }))
            .map(|r| r.prerequisite_job_ids.clone())
            .unwrap_or_default();

        let Some(record) = self.records.get(job_id) else {
            return Err(format!("job not found: {job_id}"));
        };
        if !record.view.cancelable {
            return Err("job is not cancelable".to_string());
        }
        if !matches!(record.view.status, JobStatus::Queued | JobStatus::Running) {
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
        for child_id in child_ids {
            let _ = self.cancel(&child_id);
        }
        Ok(())
    }

    pub fn cancel_all(&mut self) {
        let ids: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.view.cancelable && matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
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
    fn set_job_priority_inner(
        &mut self,
        job_id: &str,
        priority: JobPriority,
    ) -> Result<bool, String> {
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

    /// Scrub + OP/ED auto-enqueue after `rescan_library` (one scheduler flush at the end).
    pub fn enqueue_rescan_import_jobs(
        &mut self,
        db: &AppDatabase,
        scrub_imports: &[RescanScrubImport],
        op_ed_imports: &[RescanOpEdImport],
    ) -> Result<(), String> {
        self.activate_rescan_throttle();
        if scrub_imports.len() <= RESCAN_AUTO_SCRUB_MAX {
            for item in scrub_imports {
                let _ = self.enqueue_scrub_sprite_inner(
                    EnqueueScrubSpriteJob {
                        path: item.path.clone(),
                        priority: JobPriority::Low,
                        anime_title: Some(item.anime_title.clone()),
                        episode_label: Some(item.episode_label.clone()),
                        follow_up: Vec::new(),
                    },
                    false,
                )?;
            }
        }
        if !op_ed_imports.is_empty() && op_ed_imports.len() <= RESCAN_AUTO_SCRUB_MAX {
            self.enqueue_op_ed_for_rescan_imports_inner(db, op_ed_imports)?;
        }
        self.finish_scheduling_batch();
        Ok(())
    }

    fn activate_rescan_throttle(&mut self) {
        let until_ms = now_ms().saturating_add(RESCAN_LOW_PRIORITY_THROTTLE_MS);
        self.rescan_throttle_until_ms = self.rescan_throttle_until_ms.max(until_ms);
        crate::crash_log::log(
            "INFO",
            &format!(
                "rescan jobs: limiting low-priority starts to {RESCAN_LOW_PRIORITY_CAP} for {}s",
                RESCAN_LOW_PRIORITY_THROTTLE_MS / 1_000
            ),
        );
    }

    fn enqueue_op_ed_for_rescan_imports_inner(
        &mut self,
        db: &AppDatabase,
        imports: &[RescanOpEdImport],
    ) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for item in imports {
            if !seen.insert(item.anime_id) {
                continue;
            }
            let allowed =
                db.with_conn(|conn| op_ed::auto_op_ed_enqueue_allowed(conn, item.anime_id))?;
            if !allowed {
                continue;
            }
            let needs =
                db.with_conn(|conn| op_ed::anime_needs_op_ed_enqueue(conn, item.anime_id))?;
            if !needs {
                continue;
            }
            let _ = self.enqueue_op_ed_detect_inner(
                db,
                EnqueueOpEdDetectJob {
                    anime_id: item.anime_id,
                    priority: JobPriority::Low,
                    anime_title: Some(item.anime_title.clone()),
                },
                false,
            )?;
        }
        Ok(())
    }

    pub fn enqueue_episode_page_op_ed(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdDetectJob,
    ) -> Result<(), String> {
        if self
            .active_manual_op_ed_rematch_job_id(request.anime_id)
            .is_some()
        {
            return Ok(());
        }
        let allowed =
            db.with_conn(|conn| op_ed::auto_op_ed_enqueue_allowed(conn, request.anime_id))?;
        if !allowed {
            return Ok(());
        }
        let needs =
            db.with_conn(|conn| op_ed::anime_needs_op_ed_enqueue(conn, request.anime_id))?;
        if !needs {
            return Ok(());
        }
        let _ = self.enqueue_op_ed_detect(db, request)?;
        self.finish_scheduling_batch();
        Ok(())
    }

    pub fn set_op_ed_detect_priority_for_anime(
        &mut self,
        anime_id: i64,
        priority: JobPriority,
    ) -> Result<(), String> {
        let prefix = op_ed_detect_job_identity_prefix(anime_id);
        let legacy = op_ed::op_ed_job_identity(anime_id);
        let mut job_ids: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.view.status == JobStatus::Queued
                    && (r.view.identity.starts_with(&prefix) || r.view.identity == legacy)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for job_id in job_ids.drain(..) {
            if self.set_job_priority_inner(&job_id, priority)? {
                changed = true;
            }
        }
        if changed {
            self.emit_snapshot_now();
        }
        Ok(())
    }

    pub fn set_op_ed_chroma_priority_for_anime(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        priority: JobPriority,
    ) -> Result<(), String> {
        let episode_ids: Vec<i64> = db.with_conn(|conn| {
            Ok(op_ed::list_anime_episodes(conn, anime_id)?
                .into_iter()
                .map(|ep| ep.id)
                .collect())
        })?;
        self.set_op_ed_chroma_priority_for_episodes(&episode_ids, priority)
    }

    pub fn set_op_ed_chroma_priority_for_episodes(
        &mut self,
        episode_ids: &[i64],
        priority: JobPriority,
    ) -> Result<(), String> {
        let wanted: std::collections::HashSet<i64> = episode_ids.iter().copied().collect();
        let mut job_ids: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.view.status == JobStatus::Queued
                    && r.view.job_type == "op_ed_chroma"
                    && r.view
                        .identity
                        .strip_prefix("op_ed_chroma:")
                        .and_then(|s| s.parse::<i64>().ok())
                        .is_some_and(|ep_id| wanted.contains(&ep_id))
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for job_id in job_ids.drain(..) {
            if self.set_job_priority_inner(&job_id, priority)? {
                changed = true;
            }
        }
        if changed {
            self.emit_snapshot_now();
        }
        Ok(())
    }

    fn ensure_op_ed_detect_jobs_high_for_anime(&mut self, anime_id: i64) -> Result<(), String> {
        let prefix = op_ed_detect_job_identity_prefix(anime_id);
        let legacy = op_ed::op_ed_job_identity(anime_id);
        let job_ids: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| {
                r.view.status == JobStatus::Queued
                    && r.view.priority != JobPriority::High
                    && (r.view.identity.starts_with(&prefix) || r.view.identity == legacy)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for job_id in job_ids {
            if self.set_job_priority_inner(&job_id, JobPriority::High)? {
                changed = true;
            }
        }
        if changed {
            self.emit_snapshot_now();
        }
        Ok(())
    }

    fn bump_queued_chroma_for_anime(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        priority: JobPriority,
    ) -> Result<(), String> {
        self.set_op_ed_chroma_priority_for_anime(db, anime_id, priority)
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
            return Ok(EnqueueJobResult::queued(Some(existing_id)));
        }
        if scrub_sprite_is_cached(&request.path)? {
            if flush_scheduling {
                self.finish_scheduling_batch();
            }
            return Ok(EnqueueJobResult::skipped());
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
            prerequisite_pending: 0,
            prerequisite_progress_current: 0,
            prerequisite_progress_total: 0,
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
        Ok(EnqueueJobResult::queued(Some(id)))
    }

    pub fn enqueue_op_ed_chroma_for_episode(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdChromaEpisodeJob,
    ) -> Result<EnqueueJobResult, String> {
        let ep = db.with_conn(|conn| op_ed::load_episode_by_id(conn, request.episode_id))?;
        let Some(ep) = ep else {
            return Ok(EnqueueJobResult::skipped());
        };
        let result = self.enqueue_op_ed_chroma_episode(
            db,
            &ep,
            request.priority,
            request.anime_title.as_deref(),
            false,
            op_ed::OpEdChromaCacheRequirement::FullEpisodeAndDiscovery,
        )?;
        self.finish_op_ed_enqueue_batch();
        Ok(result)
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
                op_ed::OpEdChromaCacheRequirement::FullEpisodeAndDiscovery,
            )?;
            if let Some(id) = result.job_id {
                last_job_id = Some(id);
                if !result.skipped {
                    any_queued = true;
                }
            }
        }
        self.finish_op_ed_enqueue_batch();
        Ok(EnqueueJobResult::with_skip(last_job_id, !any_queued))
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
        cache_requirement: op_ed::OpEdChromaCacheRequirement,
    ) -> Result<EnqueueJobResult, String> {
        if op_ed::op_ed_chroma_cache_satisfied(ep, cache_requirement)? {
            return Ok(EnqueueJobResult::skipped());
        }

        let identity = op_ed_chroma_job_identity(ep.id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self
                .records
                .get(&existing_id)
                .is_some_and(|r| matches!(r.view.status, JobStatus::Queued | JobStatus::Running))
            {
                if self.records.get(&existing_id).is_some_and(|r| {
                    r.view.status == JobStatus::Queued
                        && priority_rank(priority) > priority_rank(r.view.priority)
                }) {
                    self.set_job_priority(&existing_id, priority)?;
                }
                return Ok(EnqueueJobResult::queued(Some(existing_id)));
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
            prerequisite_pending: 0,
            prerequisite_progress_current: 0,
            prerequisite_progress_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdChroma {
                episode: ep.clone(),
                skip_volume_stagger: false,
                cache_requirement,
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

        Ok(EnqueueJobResult::queued(Some(id)))
    }

    pub fn enqueue_op_ed_detect(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdDetectJob,
    ) -> Result<EnqueueJobResult, String> {
        self.enqueue_op_ed_detect_inner(db, request, true)
    }

    fn enqueue_op_ed_detect_inner(
        &mut self,
        db: &AppDatabase,
        request: EnqueueOpEdDetectJob,
        flush_scheduling: bool,
    ) -> Result<EnqueueJobResult, String> {
        let chroma_priority = request.priority;

        // Manual skip templates own matching — do not return a stale auto-detect job.
        let has_manual =
            db.with_conn(|conn| op_ed::has_manual_templates(conn, request.anime_id))?;
        if has_manual {
            return self.enqueue_manual_op_ed_rematch_inner(
                db,
                request.anime_id,
                request.anime_title.as_deref(),
                chroma_priority,
                flush_scheduling,
            );
        }

        if let Some(existing_id) = self.active_op_ed_detect_job_id(request.anime_id) {
            self.bump_queued_chroma_for_anime(db, request.anime_id, chroma_priority)?;
            self.ensure_op_ed_detect_jobs_high_for_anime(request.anime_id)?;
            return Ok(EnqueueJobResult::queued(Some(existing_id)));
        }

        let needs = db.with_conn(|conn| op_ed::anime_needs_op_ed_detect(conn, request.anime_id))?;
        if !needs {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Ok(EnqueueJobResult::skipped());
        }

        let title = request
            .anime_title
            .as_deref()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| format!("Anime #{}", request.anime_id));

        let episodes = db.with_conn(|conn| op_ed::list_anime_episodes(conn, request.anime_id))?;
        if episodes.is_empty() {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Ok(EnqueueJobResult::skipped());
        }
        let mut chroma_job_by_episode: HashMap<i64, String> = HashMap::new();
        for ep in &episodes {
            if op_ed::full_episode_fingerprint_cached_for_enqueue(ep)? {
                continue;
            }
            let chroma = self.enqueue_op_ed_chroma_episode(
                db,
                ep,
                chroma_priority,
                request.anime_title.as_deref(),
                false,
                op_ed::OpEdChromaCacheRequirement::FullEpisodeAndDiscovery,
            )?;
            if let Some(job_id) = chroma.job_id {
                chroma_job_by_episode.insert(ep.id, job_id);
            }
        }

        let all_episode_ids: Vec<i64> = episodes.iter().map(|ep| ep.id).collect();
        let full_pass_only =
            db.with_conn(|conn| op_ed::anime_redetect_full_pass_only(conn, request.anime_id))?;
        let plans = op_ed::plan_op_ed_detect_jobs(&all_episode_ids, full_pass_only);
        if plans.is_empty() {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Ok(EnqueueJobResult::skipped());
        }
        let mut last_detect_id: Option<String> = None;
        let mut first_detect_id: Option<String> = None;

        for (batch_index, plan) in plans.into_iter().enumerate() {
            let episode_ids = plan.episode_ids;
            let mut prerequisite_job_ids: Vec<String> = episode_ids
                .iter()
                .filter_map(|ep_id| chroma_job_by_episode.get(ep_id).cloned())
                .collect();
            if let Some(prev) = &last_detect_id {
                prerequisite_job_ids.push(prev.clone());
            }

            let identity = op_ed_detect_batch_job_identity(request.anime_id, batch_index);
            let (id, short_id) = alloc_job_ids();
            let cancel = Arc::new(AtomicBool::new(false));
            let view = JobView {
                id: id.clone(),
                short_id,
                name: plan.batch_name,
                desc: title.clone(),
                identity: identity.clone(),
                job_type: "op_ed_detect".to_string(),
                resource_type: JobResourceType::None,
                priority: JobPriority::High,
                status: JobStatus::Queued,
                cancelable: true,
                progress: JobProgress {
                    current_step: 0,
                    total_steps: 100,
                },
                step_label: "Queued".to_string(),
                completion_message: None,
                created_at: now_ms(),
                started_at: None,
                finished_at: None,
                waiting_for: Vec::new(),
                prerequisite_total: 0,
                prerequisite_pending: 0,
                prerequisite_progress_current: 0,
                prerequisite_progress_total: 0,
            };
            let options = plan.options;
            let record = JobRecord {
                view,
                cancel,
                kind: JobKind::OpEdDetect {
                    anime_id: request.anime_id,
                    episode_ids,
                    options,
                },
                follow_ups: Vec::new(),
                prerequisite_job_ids,
            };
            self.records.insert(id.clone(), record);
            self.identity_to_id.insert(identity, id.clone());

            self.try_start_or_queue(&id, true);

            if first_detect_id.is_none() {
                first_detect_id = Some(id.clone());
            }
            last_detect_id = Some(id);
        }

        if flush_scheduling {
            self.finish_op_ed_enqueue_batch();
        }
        Ok(EnqueueJobResult::queued(first_detect_id))
    }

    pub fn enqueue_op_ed_auto_rematch(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        anime_title: Option<&str>,
    ) -> Result<EnqueueJobResult, String> {
        let title = anime_title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| format!("Anime #{anime_id}"));

        let episodes = db.with_conn(|conn| op_ed::list_anime_episodes(conn, anime_id))?;
        if episodes.len() < 2 {
            self.finish_op_ed_enqueue_batch();
            return Ok(EnqueueJobResult::skipped());
        }

        let mut chroma_job_by_episode: HashMap<i64, String> = HashMap::new();
        for ep in &episodes {
            if op_ed::full_episode_fingerprint_cached_for_enqueue(ep)? {
                continue;
            }
            let chroma = self.enqueue_op_ed_chroma_episode(
                db,
                ep,
                JobPriority::Medium,
                anime_title,
                false,
                op_ed::OpEdChromaCacheRequirement::FullEpisodeAndDiscovery,
            )?;
            if let Some(job_id) = chroma.job_id {
                chroma_job_by_episode.insert(ep.id, job_id);
            }
        }

        let all_episode_ids: Vec<i64> = episodes.iter().map(|ep| ep.id).collect();
        let identity = op_ed::op_ed_job_identity(anime_id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            self.finish_op_ed_enqueue_batch();
            return Ok(EnqueueJobResult::queued(Some(existing_id)));
        }

        let prerequisite_job_ids: Vec<String> = all_episode_ids
            .iter()
            .filter_map(|ep_id| chroma_job_by_episode.get(ep_id).cloned())
            .collect();

        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            short_id,
            name: "Rematch OP/ED".to_string(),
            desc: title,
            identity: identity.clone(),
            job_type: "op_ed_detect".to_string(),
            resource_type: JobResourceType::None,
            priority: JobPriority::Medium,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps: 100,
            },
            step_label: "Queued".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            waiting_for: Vec::new(),
            prerequisite_total: 0,
            prerequisite_pending: 0,
            prerequisite_progress_current: 0,
            prerequisite_progress_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdDetect {
                anime_id,
                episode_ids: all_episode_ids,
                options: op_ed::auto_rematch_job_options(),
            },
            follow_ups: Vec::new(),
            prerequisite_job_ids,
        };
        self.records.insert(id.clone(), record);
        self.identity_to_id.insert(identity, id.clone());
        self.try_start_or_queue(&id, false);
        self.queued_ids.push(id.clone());
        self.finish_op_ed_enqueue_batch();
        Ok(EnqueueJobResult::queued(Some(id)))
    }

    /// Queued or running manual rematch work for a title (summary shell or per-episode jobs).
    fn active_manual_op_ed_rematch_job_id(&self, anime_id: i64) -> Option<String> {
        let summary_identity = op_ed::manual_op_ed_rematch_summary_job_identity(anime_id);
        if let Some(existing_id) = self.identity_to_id.get(&summary_identity) {
            if let Some(record) = self.records.get(existing_id) {
                if matches!(record.view.status, JobStatus::Queued | JobStatus::Running) {
                    return Some(existing_id.clone());
                }
            }
        }
        let prefix = op_ed::manual_op_ed_rematch_job_identity_prefix(anime_id);
        for (id, record) in &self.records {
            if record.view.identity.starts_with(&prefix)
                && matches!(record.view.status, JobStatus::Queued | JobStatus::Running)
            {
                return Some(id.clone());
            }
        }
        None
    }

    pub fn enqueue_manual_op_ed_rematch(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        anime_title: Option<&str>,
        chroma_priority: JobPriority,
    ) -> Result<EnqueueJobResult, String> {
        self.enqueue_manual_op_ed_rematch_inner(db, anime_id, anime_title, chroma_priority, true)
    }

    fn enqueue_manual_op_ed_rematch_inner(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        anime_title: Option<&str>,
        chroma_priority: JobPriority,
        flush_scheduling: bool,
    ) -> Result<EnqueueJobResult, String> {
        db.with_conn(|conn| op_ed::validate_manual_templates_for_rematch(conn, anime_id))?;

        if let Some(existing_id) = self.active_manual_op_ed_rematch_job_id(anime_id) {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Ok(EnqueueJobResult::queued(Some(existing_id)));
        }

        let title = anime_title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| format!("Anime #{anime_id}"));

        let episodes = db.with_conn(|conn| op_ed::list_anime_episodes(conn, anime_id))?;
        if episodes.is_empty() {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Err("no episodes to rematch".to_string());
        }

        db.with_conn(|conn| {
            op_ed::clear_episode_op_ed_segments_for_anime(conn, anime_id)?;
            op_ed::ensure_pending_segments_for_anime(conn, anime_id)
        })?;

        let episode_labels = db.with_conn(|conn| op_ed::episode_display_labels(conn, anime_id))?;

        let mut episode_job_ids: Vec<String> = Vec::new();
        for ep in &episodes {
            let episode_label = episode_labels
                .get(&ep.id)
                .map(String::as_str)
                .unwrap_or("Episode ?");
            if let Some(job_id) = self.enqueue_manual_op_ed_rematch_episode(
                db,
                anime_id,
                ep,
                &title,
                episode_label,
                chroma_priority,
                anime_title,
            )? {
                episode_job_ids.push(job_id);
            }
        }

        if episode_job_ids.is_empty() {
            if flush_scheduling {
                self.finish_op_ed_enqueue_batch();
            }
            return Ok(EnqueueJobResult::skipped());
        }

        let summary_id =
            self.enqueue_manual_op_ed_rematch_summary(anime_id, &title, episode_job_ids)?;
        if flush_scheduling {
            self.finish_op_ed_enqueue_batch();
        }
        Ok(EnqueueJobResult::queued(Some(summary_id)))
    }

    fn enqueue_manual_op_ed_rematch_summary(
        &mut self,
        anime_id: i64,
        anime_title: &str,
        episode_job_ids: Vec<String>,
    ) -> Result<String, String> {
        let identity = op_ed::manual_op_ed_rematch_summary_job_identity(anime_id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.contains_key(&existing_id) {
                return Ok(existing_id);
            }
        }

        let episode_count = episode_job_ids.len();
        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            short_id,
            name: format!("Rematch manual skip ({episode_count} episodes)"),
            desc: anime_title.to_string(),
            identity: identity.clone(),
            job_type: "manual_op_ed_rematch_summary".to_string(),
            resource_type: JobResourceType::None,
            priority: JobPriority::Medium,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps: 1,
            },
            step_label: "Waiting for episode jobs".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            waiting_for: Vec::new(),
            prerequisite_total: episode_job_ids.len() as u32,
            prerequisite_pending: 0,
            prerequisite_progress_current: 0,
            prerequisite_progress_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdManualRematchSummary { anime_id },
            follow_ups: Vec::new(),
            prerequisite_job_ids: episode_job_ids,
        };
        self.records.insert(id.clone(), record);
        self.identity_to_id.insert(identity, id.clone());
        self.try_start_or_queue(&id, false);
        self.queued_ids.push(id.clone());
        Ok(id)
    }

    /// One rematch job per episode; prerequisite is that episode's chroma job only.
    fn enqueue_manual_op_ed_rematch_episode(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        ep: &op_ed::OpEdEpisode,
        anime_title: &str,
        episode_label: &str,
        chroma_priority: JobPriority,
        anime_title_for_chroma: Option<&str>,
    ) -> Result<Option<String>, String> {
        let identity = op_ed::manual_op_ed_rematch_episode_job_identity(anime_id, ep.id);
        if let Some(existing_id) = self.identity_to_id.get(&identity).cloned() {
            if self.records.contains_key(&existing_id) {
                return Ok(Some(existing_id));
            }
        }

        let mut prerequisite_job_ids: Vec<String> = Vec::new();
        if !op_ed::op_ed_chroma_cache_satisfied(ep, op_ed::OpEdChromaCacheRequirement::FullEpisode)?
        {
            let chroma = self.enqueue_op_ed_chroma_episode(
                db,
                ep,
                chroma_priority,
                anime_title_for_chroma,
                false,
                op_ed::OpEdChromaCacheRequirement::FullEpisode,
            )?;
            if let Some(job_id) = chroma.job_id {
                prerequisite_job_ids.push(job_id);
            }
        }

        let desc = format!("{anime_title} — {episode_label}");

        let (id, short_id) = alloc_job_ids();
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: id.clone(),
            short_id,
            name: "Rematch manual skip".to_string(),
            desc,
            identity: identity.clone(),
            job_type: "manual_op_ed_rematch".to_string(),
            resource_type: JobResourceType::None,
            priority: JobPriority::Medium,
            status: JobStatus::Queued,
            cancelable: true,
            progress: JobProgress {
                current_step: 0,
                total_steps: 3,
            },
            step_label: "Queued".to_string(),
            completion_message: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            waiting_for: Vec::new(),
            prerequisite_total: prerequisite_job_ids.len() as u32,
            prerequisite_pending: 0,
            prerequisite_progress_current: 0,
            prerequisite_progress_total: 0,
        };
        let record = JobRecord {
            view,
            cancel,
            kind: JobKind::OpEdManualRematch {
                anime_id,
                episode_id: ep.id,
            },
            follow_ups: Vec::new(),
            prerequisite_job_ids,
        };
        self.records.insert(id.clone(), record);
        self.identity_to_id.insert(identity, id.clone());
        self.try_start_or_queue(&id, false);
        self.queued_ids.push(id.clone());
        Ok(Some(id))
    }

    pub fn prepare_manual_op_ed_rematch(
        &mut self,
        db: &AppDatabase,
        anime_id: i64,
        anime_title: Option<String>,
    ) -> Result<op_ed::PrepareManualOpEdRematchResult, String> {
        let has_manual = db.with_conn(|conn| op_ed::has_manual_templates(conn, anime_id))?;
        let title = anime_title.as_deref();
        if has_manual {
            if let Some(job_id) = self.active_manual_op_ed_rematch_job_id(anime_id) {
                return Ok(op_ed::PrepareManualOpEdRematchResult {
                    job_id: Some(job_id),
                    used_manual_templates: true,
                });
            }
            db.with_conn(|conn| op_ed::validate_manual_templates_for_rematch(conn, anime_id))?;
            db.with_conn(|conn| op_ed::clear_episode_op_ed_segments_for_anime(conn, anime_id))?;
            let result =
                self.enqueue_manual_op_ed_rematch(db, anime_id, title, JobPriority::Medium)?;
            Ok(op_ed::PrepareManualOpEdRematchResult {
                job_id: result.job_id,
                used_manual_templates: true,
            })
        } else {
            if let Some(job_id) = self.active_manual_op_ed_rematch_job_id(anime_id) {
                return Ok(op_ed::PrepareManualOpEdRematchResult {
                    job_id: Some(job_id),
                    used_manual_templates: false,
                });
            }
            db.with_conn(|conn| op_ed::clear_episode_op_ed_segments_for_anime(conn, anime_id))?;
            let result = self.enqueue_op_ed_auto_rematch(db, anime_id, title)?;
            Ok(op_ed::PrepareManualOpEdRematchResult {
                job_id: result.job_id,
                used_manual_templates: false,
            })
        }
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
            .filter(|r| r.view.status == JobStatus::Running && r.view.resource_type.as_str() == key)
            .count() as u32
    }

    fn running_low_priority_count(&self) -> u32 {
        self.records
            .values()
            .filter(|record| {
                record.view.status == JobStatus::Running && record.view.priority == JobPriority::Low
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
            JobKind::OpEdChroma { episode, .. } => Some(episode.path.as_str()),
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
        disk_volume::chroma_start_deferred(path, self.last_chroma_start_ms_for_path(path), now_ms())
    }

    fn record_chroma_volume_start(&mut self, path: &str) {
        if !disk_volume::path_requires_chroma_stagger(path) {
            return;
        }
        if let Some(volume) = disk_volume::volume_key_for_path(path) {
            self.last_chroma_start_on_volume.insert(volume, now_ms());
        }
    }

    /// Let the next chroma job on this volume start without waiting for the HDD gap.
    fn release_chroma_volume_stagger(&mut self, path: &str) {
        if !disk_volume::path_requires_chroma_stagger(path) {
            return;
        }
        let Some(volume) = disk_volume::volume_key_for_path(path) else {
            return;
        };
        let backdated = now_ms().saturating_sub(disk_volume::CHROMA_HDD_MIN_GAP_MS);
        self.last_chroma_start_on_volume.insert(volume, backdated);
    }

    fn active_op_ed_detect_job_id(&self, anime_id: i64) -> Option<String> {
        let prefix = op_ed_detect_job_identity_prefix(anime_id);
        let legacy = op_ed::op_ed_job_identity(anime_id);
        self.records
            .iter()
            .find(|(_, r)| {
                (r.view.identity.starts_with(&prefix) || r.view.identity == legacy)
                    && matches!(r.view.status, JobStatus::Queued | JobStatus::Running)
            })
            .map(|(id, _)| id.clone())
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
        if rescan_throttle_blocks_low_priority(
            record.view.priority,
            now_ms(),
            self.rescan_throttle_until_ms,
            self.running_low_priority_count(),
        ) {
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

    fn schedule_rescan_throttle_wakeup_if_needed(&mut self) {
        let now = now_ms();
        if self.rescan_throttle_wakeup_armed
            || self.rescan_throttle_until_ms <= now
            || !self.queued_ids.iter().any(|id| {
                self.records.get(id).is_some_and(|record| {
                    record.view.status == JobStatus::Queued
                        && record.view.priority == JobPriority::Low
                })
            })
        {
            return;
        }
        self.rescan_throttle_wakeup_armed = true;
        super::schedule_job_pump_after_ms(
            &self.app,
            self.rescan_throttle_until_ms.saturating_sub(now),
        );
    }

    pub fn on_job_pump_wakeup(&mut self) {
        self.chroma_disk_poll_wakeup_armed = false;
        self.rescan_throttle_wakeup_armed = false;
        if self.rescan_throttle_until_ms > 0 && now_ms() >= self.rescan_throttle_until_ms {
            self.rescan_throttle_until_ms = 0;
            crate::crash_log::log("INFO", "rescan jobs: low-priority start limit released");
        }
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
        self.schedule_rescan_throttle_wakeup_if_needed();
    }

    fn pick_startable_queued_id(&self) -> Option<String> {
        pick_startable_from_queue(
            &self.queued_ids,
            |id| {
                self.records
                    .get(id)
                    .is_some_and(|r| r.view.status == JobStatus::Queued && self.can_start(id))
            },
            |id| {
                self.records
                    .get(id)
                    .map(|r| r.view.priority)
                    .unwrap_or(JobPriority::Low)
            },
        )
    }

    fn start_job(&mut self, job_id: &str) {
        let chroma_path = self.chroma_episode_path(job_id).map(str::to_string);
        let Some(record) = self.records.get_mut(job_id) else {
            return;
        };
        if record.view.status != JobStatus::Queued {
            return;
        }
        let chroma_cache_hit = matches!(&record.kind, JobKind::OpEdChroma { episode, cache_requirement, .. } if {
            op_ed::op_ed_chroma_cache_satisfied(episode, *cache_requirement).unwrap_or(false)
        });
        if let JobKind::OpEdChroma {
            skip_volume_stagger,
            ..
        } = &mut record.kind
        {
            *skip_volume_stagger = chroma_cache_hit;
        }
        record.view.status = JobStatus::Running;
        record.view.started_at = Some(now_ms());
        record.view.step_label = if chroma_cache_hit {
            "Cached fingerprint".to_string()
        } else {
            "Starting".to_string()
        };
        let job_identity = record.view.identity.clone();
        let job_type = record.view.job_type.clone();
        let job_short = record.view.short_id;
        self.queued_ids.retain(|id| id != job_id);

        let cancel = record.cancel.clone();
        let kind = record.kind.clone();
        let job_id_owned = job_id.to_string();
        let app = self.app.clone();

        if let Some(path) = chroma_path.as_deref() {
            if chroma_cache_hit {
                self.release_chroma_volume_stagger(path);
                self.complete_worker(
                    job_id,
                    WorkerOutcome::Done("Cached fingerprint".to_string(), None),
                );
                return;
            }
            self.record_chroma_volume_start(path);
        }

        if !matches!(job_type.as_str(), "scrub_sprite" | "op_ed_chroma") {
            crate::crash_log::log(
                "INFO",
                &format!("job started: #{job_short} {job_type} ({job_identity})"),
            );
        }

        match kind {
            JobKind::ScrubSprite { path } => {
                super::spawn_scrub_worker(app, job_id_owned, path, cancel);
            }
            JobKind::OpEdChroma { episode, .. } => {
                super::spawn_op_ed_chroma_worker(app, job_id_owned, episode, cancel);
            }
            JobKind::OpEdDetect {
                anime_id,
                episode_ids,
                options,
            } => {
                super::spawn_op_ed_worker(
                    app,
                    job_id_owned,
                    anime_id,
                    episode_ids,
                    options,
                    cancel,
                );
            }
            JobKind::OpEdManualRematch {
                anime_id,
                episode_id,
            } => {
                super::spawn_manual_op_ed_rematch_worker(
                    app,
                    job_id_owned,
                    anime_id,
                    episode_id,
                    cancel,
                );
            }
            JobKind::OpEdManualRematchSummary { .. } => {
                self.complete_worker(
                    job_id,
                    WorkerOutcome::Done("Manual skip rematch complete".to_string(), None),
                );
                return;
            }
        }
        self.emit_snapshot();
    }

    pub fn complete_worker(&mut self, job_id: &str, outcome: WorkerOutcome) {
        let chroma_release_stagger = self.records.get(job_id).and_then(|r| match &r.kind {
            JobKind::OpEdChroma {
                episode,
                skip_volume_stagger: true,
                ..
            } => Some(episode.path.clone()),
            _ => None,
        });

        let scrub_path_for_emit = self.records.get(job_id).and_then(|r| match &r.kind {
            JobKind::ScrubSprite { path } => Some(path.clone()),
            JobKind::OpEdChroma { .. }
            | JobKind::OpEdDetect { .. }
            | JobKind::OpEdManualRematch { .. }
            | JobKind::OpEdManualRematchSummary { .. } => None,
        });

        let op_ed_updated_anime_id = self
            .records
            .get(job_id)
            .and_then(|r| Self::op_ed_job_anime_id(&r.kind));

        match outcome {
            WorkerOutcome::Done(message, scrub_ready) => {
                if let Some(ready) = scrub_ready {
                    emit_scrub_sprite_status(
                        &self.app,
                        scrub_preview::ScrubSpriteStatus::Ready(ready),
                    );
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
                if let Some(anime_id) = op_ed_updated_anime_id {
                    self.schedule_op_ed_analysis_updated(anime_id);
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
        if let Some(path) = chroma_release_stagger {
            self.release_chroma_volume_stagger(&path);
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
        let job_type = record.view.job_type.clone();
        let job_identity = record.view.identity.clone();

        let mut view = record.view;
        view.status = status;
        view.completion_message = completion_message.clone();
        view.finished_at = Some(now_ms());
        if view.progress.total_steps > 0 && status == JobStatus::Done {
            view.progress.current_step = view.progress.total_steps;
        }

        let detail = completion_message
            .as_deref()
            .filter(|message| !message.is_empty())
            .map(|message| format!(": {message}"))
            .unwrap_or_default();
        match status {
            JobStatus::Failed => {
                crate::crash_log::log(
                    "ERROR",
                    &format!("job failed: #{prereq_short} {job_type} ({job_identity}){detail}"),
                );
            }
            JobStatus::Canceled => {
                crate::crash_log::log(
                    "WARN",
                    &format!("job canceled: #{prereq_short} {job_type} ({job_identity}){detail}"),
                );
            }
            JobStatus::Done if !matches!(job_type.as_str(), "scrub_sprite" | "op_ed_chroma") => {
                crate::crash_log::log(
                    "INFO",
                    &format!("job finished: #{prereq_short} {job_type} ({job_identity}){detail}"),
                );
            }
            _ => {}
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
    match run_scrub_sprite_job(path, cancel, |step, total, label| {
        on_step(step, total, label)
    }) {
        Ok(ready) => WorkerOutcome::Done("Sprite sheet ready".to_string(), Some(ready)),
        Err(e) if e.contains("cancelled") => WorkerOutcome::Canceled,
        Err(e) => WorkerOutcome::Failed(e),
    }
}

pub fn run_op_ed_job_worker(
    app: &AppHandle,
    anime_id: i64,
    episode_ids: Vec<i64>,
    options: OpEdDetectJobOptions,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled;
    }
    let db = app.state::<AppDatabase>();
    match op_ed::run_op_ed_detect_job(&db, anime_id, &episode_ids, options, cancel, on_step) {
        Ok(()) => WorkerOutcome::Done("OP/ED batch complete".to_string(), None),
        Err(e) if e.contains("cancelled") => WorkerOutcome::Canceled,
        Err(e) => WorkerOutcome::Failed(e),
    }
}

pub fn run_manual_op_ed_rematch_worker(
    app: &AppHandle,
    anime_id: i64,
    episode_id: i64,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::Canceled;
    }
    let db = app.state::<AppDatabase>();
    match op_ed::run_manual_op_ed_rematch_episode(&db, anime_id, episode_id, cancel, on_step) {
        Ok(()) => WorkerOutcome::Done("Manual skip rematch complete".to_string(), None),
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
    let block_low = queued_ids
        .iter()
        .any(|id| job_priority(id) == JobPriority::Medium && can_start(id));
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

    #[test]
    fn rescan_throttle_only_blocks_low_priority_at_its_cap() {
        let now = 1_000;
        let until = 2_000;
        assert!(!rescan_throttle_blocks_low_priority(
            JobPriority::Low,
            now,
            until,
            RESCAN_LOW_PRIORITY_CAP - 1,
        ));
        assert!(rescan_throttle_blocks_low_priority(
            JobPriority::Low,
            now,
            until,
            RESCAN_LOW_PRIORITY_CAP,
        ));
        assert!(!rescan_throttle_blocks_low_priority(
            JobPriority::Medium,
            now,
            until,
            RESCAN_LOW_PRIORITY_CAP,
        ));
        assert!(!rescan_throttle_blocks_low_priority(
            JobPriority::Low,
            until,
            until,
            RESCAN_LOW_PRIORITY_CAP,
        ));
    }
}
