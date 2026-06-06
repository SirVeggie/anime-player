//! OP/ED detection via repeated audio fingerprints across episodes.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::AppDatabase;
use crate::media_tools::{
    cache_key, extract_pcm_range, fpcalc_path, hidden_command, normalized_video_path,
    portable_data_dir, probe_duration, probe_video_fps,
};

pub const ANALYSIS_VERSION: i32 = 2;
pub const SKIP_OP_ED_SETTING_KEY: &str = "skip_op_ed";
pub const AUTO_OP_ED_DETECT_SETTING_KEY: &str = "auto_op_ed_detect";
pub const DONT_SKIP_FIRST_EPISODE_OP_ED_SETTING_KEY: &str = "dont_skip_first_episode_op_ed";
/// Parent folder under portable `data/` for all OP/ED artifacts.
pub const OP_ED_DATA_DIR: &str = "op-ed";
const FINGERPRINTS_SUBDIR: &str = "fingerprints";
const JOB_NAME: &str = "op_ed_detect";
const MANUAL_REMATCH_JOB_NAME: &str = "manual_op_ed_rematch";
pub const MANUAL_TEMPLATE_MIN_SEC: f64 = 5.0;
pub const MANUAL_TEMPLATE_MAX_SEC: f64 = 180.0;

pub const SAMPLE_RATE: u32 = 11025;
const CHROMAPRINT_FRAME_SEC: f64 = 0.1238;
/// Discovery/template window length in seconds (each window gets its own ffmpeg + fpcalc pass).
const SEED_WINDOW_SEC: f64 = 15.0;
/// Frame equivalents for tests and in-memory trim/slice helpers only — not for discovery extraction.
const SEED_WINDOW_FRAMES: usize = 120;
/// Minimum search region before discovery skips an episode (~15s window + 5s margin).
const SEED_MIN_REGION_SEC: f64 = SEED_WINDOW_SEC + 5.0;
const SEGMENT_DURATION_SEC: f64 = 90.0;
const OP_SEARCH_SEC: f64 = 180.0;
const ED_TAIL_SEC: f64 = 180.0;
const MATCH_AVERAGE_THRESHOLD: f32 = 0.84;
const MATCH_STRONG_FRAME_THRESHOLD: f32 = 0.84;
const MATCH_MIN_STRONG_FRAME_RATIO: f32 = 0.60;
const MATCH_MIN_LOWER_QUARTILE: f32 = 0.78;
const SEED_MATCH_THRESHOLD: f32 = 0.82;
/// After coarse discovery, slide the 90s template ± this many seconds in unison.
const SEED_REFINE_OFFSET_SEC: i32 = 8;
const SEED_REFINE_OFFSET_STEP_SEC: f64 = 0.5;
/// Minimum sliding score when anchoring the 90s template on a seed episode for refinement.
const SEED_REFINE_ANCHOR_THRESHOLD: f32 = 0.82;
const MAX_SEED_EPISODES: usize = 10;
const MIN_EPISODES_FOR_NO_OP_ED: usize = 3;
/// Consecutive per-kind match misses that indicate a new OP/ED block (e.g. season change).
const FULL_PASS_FAIL_STREAK_FOR_NO_OP_ED: usize = 3;
/// Upper bound on discovery/match blocks per kind (avoids unbounded re-scan on bad data).
const MAX_OP_ED_BLOCKS: u32 = 8;
/// Lead seconds trimmed from the template on per-episode match retry (bad file-start frames).
const MATCH_FALLBACK_LEAD_TRIM_SEC: f64 = 3.0;
/// Tail seconds trimmed from the ED template on bidirectional match retry (fade/credits noise).
const MATCH_FALLBACK_TAIL_TRIM_SEC: f64 = 3.0;
/// Minimum remaining template length after lead trim for a fallback attempt.
const MATCH_FALLBACK_MIN_TEMPLATE_SEC: f64 = 45.0;
/// Near-miss fallback: best offset must be within this many seconds of the search region start.
const MATCH_EDGE_NEAR_OFFSET_SEC: f64 = 2.5;
const MATCH_NEAR_MISS_AVERAGE_THRESHOLD: f32 = 0.82;
const MATCH_NEAR_MISS_MIN_STRONG_FRAME_RATIO: f32 = 0.55;
const MATCH_NEAR_MISS_MIN_LOWER_QUARTILE: f32 = 0.74;

/// First-batch episode count for long seasons (preview pass before a full-title pass).
pub const OP_ED_DETECT_BATCH_SIZE: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Op,
    Ed,
}

impl SegmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Op => "op",
            Self::Ed => "ed",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Op => "OP",
            Self::Ed => "ED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "op" => Some(Self::Op),
            "ed" => Some(Self::Ed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpEdSegmentStatus {
    Pending,
    Analyzing,
    Matched,
    NotFound,
    Failed,
    Skipped,
}

impl OpEdSegmentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Analyzing => "analyzing",
            Self::Matched => "matched",
            Self::NotFound => "not_found",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn from_db(s: &str) -> Self {
        match s {
            "analyzing" => Self::Analyzing,
            "matched" => Self::Matched,
            "not_found" => Self::NotFound,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpEdSegmentInfo {
    pub kind: String,
    pub status: String,
    pub start_sec: Option<f64>,
    pub end_sec: Option<f64>,
    pub confidence: Option<f64>,
    pub search_pass: String,
    pub error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimeOpEdAnalysisSummary {
    pub anime_id: i64,
    pub no_op_ed: bool,
    pub analysis_version: i32,
    pub analyzed_at: Option<String>,
    pub episode_count: i64,
    pub op_matched: i64,
    pub op_pending: i64,
    pub ed_matched: i64,
    pub ed_pending: i64,
    pub templates_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualOpEdTemplate {
    pub id: i64,
    pub kind: String,
    pub kind_index: i32,
    pub start_sec: f64,
    pub duration_sec: f64,
    pub source_episode_id: i64,
    pub source_episode_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareManualOpEdRematchResult {
    pub job_id: Option<String>,
    pub used_manual_templates: bool,
}

#[derive(Debug, Clone)]
struct ManualTemplateRow {
    id: i64,
    kind: SegmentKind,
    duration_sec: f64,
    fingerprint_cache_key: String,
}

#[derive(Debug, Clone)]
struct EpisodeRow {
    id: i64,
    path: String,
    duration_seconds: f64,
}

#[derive(Debug, Clone)]
struct Fingerprint {
    /// Raw signed Chromaprint subfingerprints from `fpcalc -raw -signed`.
    values: Vec<i32>,
}

impl Fingerprint {
    fn frame_count(&self) -> usize {
        self.values.len()
    }

    fn frames_for_duration(duration_sec: f64) -> usize {
        (duration_sec / CHROMAPRINT_FRAME_SEC).floor() as usize
    }
}

fn op_ed_data_dir() -> Result<PathBuf, String> {
    let dir = portable_data_dir()?.join(OP_ED_DATA_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create {dir:?}: {e}"))?;
    Ok(dir)
}

fn fingerprint_cache_dir() -> Result<PathBuf, String> {
    let dir = op_ed_data_dir()?.join(FINGERPRINTS_SUBDIR);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create {dir:?}: {e}"))?;
    Ok(dir)
}

fn fingerprint_path(cache_key: &str) -> Result<PathBuf, String> {
    Ok(fingerprint_cache_dir()?.join(format!("{cache_key}.fp")))
}

fn directory_size(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

pub fn op_ed_job_identity(anime_id: i64) -> String {
    format!("{JOB_NAME}:{anime_id}")
}

/// Batched detect jobs use `op_ed_detect:{anime_id}:{batch_index}`.
pub fn op_ed_detect_batch_job_identity(anime_id: i64, batch_index: usize) -> String {
    format!("{JOB_NAME}:{anime_id}:{batch_index}")
}

pub fn op_ed_detect_job_identity_prefix(anime_id: i64) -> String {
    format!("{JOB_NAME}:{anime_id}:")
}

#[derive(Debug, Clone, Copy)]
pub struct OpEdDetectJobOptions {
    /// Reuse templates already in SQLite (batch 2+), falling back to discovery when needed.
    pub continue_templates: bool,
    /// Reset `no_op_ed` and ensure pending segment rows for every episode in the title.
    pub init_anime_state: bool,
    /// Set `op_ed_analyzed_at` when this job finishes.
    pub mark_analyzed: bool,
    /// Re-run matching on episodes already `matched` instead of skipping them in the match loop.
    pub rematch_matched: bool,
    /// Clear `matched` to `pending` before detect (keeps times via SQL) so block detection sees the
    /// full episode list. Used after a preview batch left early episodes matched.
    pub demote_matched_for_blocks: bool,
}

#[derive(Debug, Clone)]
pub struct OpEdDetectJobPlan {
    pub episode_ids: Vec<i64>,
    pub options: OpEdDetectJobOptions,
    pub batch_name: String,
}

/// True when the title was fully analyzed before and we only need a single all-episode pass
/// (e.g. new episodes imported).
pub fn anime_redetect_full_pass_only(conn: &Connection, anime_id: i64) -> Result<bool, String> {
    let (version, analyzed_at): (i32, Option<String>) = conn
        .query_row(
            "SELECT op_ed_analysis_version, op_ed_analyzed_at FROM anime WHERE id = ?1",
            params![anime_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if version != ANALYSIS_VERSION {
        return Ok(false);
    }
    Ok(analyzed_at
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty()))
}

/// Build detect jobs: one job for short seasons; preview (first 12) + full (all eps) when longer.
pub fn plan_op_ed_detect_jobs(
    all_episode_ids: &[i64],
    full_pass_only: bool,
) -> Vec<OpEdDetectJobPlan> {
    if all_episode_ids.len() < 2 {
        return Vec::new();
    }
    if full_pass_only || all_episode_ids.len() <= OP_ED_DETECT_BATCH_SIZE {
        return vec![OpEdDetectJobPlan {
            episode_ids: all_episode_ids.to_vec(),
            options: OpEdDetectJobOptions {
                init_anime_state: !full_pass_only,
                continue_templates: false,
                mark_analyzed: true,
                rematch_matched: false,
                demote_matched_for_blocks: false,
            },
            batch_name: "Detect OP/ED".to_string(),
        }];
    }
    let preview_ids: Vec<i64> = all_episode_ids
        .iter()
        .take(OP_ED_DETECT_BATCH_SIZE)
        .copied()
        .collect();
    vec![
        OpEdDetectJobPlan {
            episode_ids: preview_ids,
            options: OpEdDetectJobOptions {
                init_anime_state: true,
                continue_templates: false,
                mark_analyzed: false,
                rematch_matched: false,
                demote_matched_for_blocks: false,
            },
            batch_name: "Detect OP/ED (1/2)".to_string(),
        },
        OpEdDetectJobPlan {
            episode_ids: all_episode_ids.to_vec(),
            options: OpEdDetectJobOptions {
                init_anime_state: false,
                continue_templates: false,
                mark_analyzed: true,
                rematch_matched: false,
                demote_matched_for_blocks: true,
            },
            batch_name: "Detect OP/ED (2/2)".to_string(),
        },
    ]
}

const CHROMA_JOB_NAME: &str = "op_ed_chroma";

pub fn op_ed_chroma_job_identity(episode_id: i64) -> String {
    format!("{CHROMA_JOB_NAME}:{episode_id}")
}

/// Episode row used when enqueueing OP/ED fingerprint jobs.
#[derive(Debug, Clone)]
pub struct OpEdEpisode {
    pub id: i64,
    pub path: String,
    pub duration_seconds: f64,
}

pub fn count_non_missing_episodes(conn: &Connection, anime_id: i64) -> Result<usize, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes WHERE anime_id = ?1 AND missing = 0",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count as usize)
}

/// Whether this title should be queued for OP/ED detection (episode page / small rescan).
pub fn anime_needs_op_ed_detect(conn: &Connection, anime_id: i64) -> Result<bool, String> {
    if count_non_missing_episodes(conn, anime_id)? < 2 {
        return Ok(false);
    }
    let (version, analyzed_at): (i32, Option<String>) = conn
        .query_row(
            "SELECT op_ed_analysis_version, op_ed_analyzed_at FROM anime WHERE id = ?1",
            params![anime_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if version != ANALYSIS_VERSION {
        return Ok(true);
    }
    let analyzed = analyzed_at
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if !analyzed {
        return Ok(true);
    }
    let gaps: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes e
             WHERE e.anime_id = ?1 AND e.missing = 0
             AND (
               NOT EXISTS (
                 SELECT 1 FROM episode_op_ed_segments s
                 WHERE s.episode_id = e.id AND s.kind = 'op'
               )
               OR NOT EXISTS (
                 SELECT 1 FROM episode_op_ed_segments s
                 WHERE s.episode_id = e.id AND s.kind = 'ed'
               )
             )",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(gaps > 0)
}

pub fn list_anime_episodes(conn: &Connection, anime_id: i64) -> Result<Vec<OpEdEpisode>, String> {
    let rows = load_episodes(conn, anime_id)?;
    Ok(rows
        .into_iter()
        .map(|ep| OpEdEpisode {
            id: ep.id,
            path: ep.path,
            duration_seconds: ep.duration_seconds,
        })
        .collect())
}

pub fn load_episode_by_id(conn: &Connection, episode_id: i64) -> Result<Option<OpEdEpisode>, String> {
    conn.query_row(
        "SELECT id, path, duration_seconds FROM episodes WHERE id = ?1 AND missing = 0",
        params![episode_id],
        |row| {
            Ok(OpEdEpisode {
                id: row.get(0)?,
                path: row.get(1)?,
                duration_seconds: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn episode_duration_seconds(ep: &OpEdEpisode) -> Result<f64, String> {
    if ep.duration_seconds > 0.0 {
        return Ok(ep.duration_seconds);
    }
    let path = normalized_video_path(&ep.path)?;
    probe_duration(&path)
}

fn full_episode_fingerprint_cache_key(ep: &OpEdEpisode) -> Result<String, String> {
    let duration = episode_duration_seconds(ep)?;
    let extract_len = full_episode_extract_len(duration);
    let path_buf = normalized_video_path(&ep.path)?;
    Ok(format!(
        "cp{}_{}_{}_{}",
        ANALYSIS_VERSION,
        cache_key(&path_buf)?,
        0_i64,
        (extract_len * 1000.0) as i64
    ))
}

/// Whether the full-episode fingerprint used during OP/ED matching is already on disk.
pub fn full_episode_fingerprint_cached(ep: &OpEdEpisode) -> Result<bool, String> {
    let key = full_episode_fingerprint_cache_key(ep)?;
    Ok(load_fingerprint(&key)?.is_some())
}

/// Full episode plus phase-1 discovery windows (isolated segment fpcalc, one ffmpeg per band).
pub fn episode_chroma_cache_complete(ep: &OpEdEpisode) -> Result<bool, String> {
    if !full_episode_fingerprint_cached(ep)? {
        return Ok(false);
    }
    discovery_fingerprints_cached(ep)
}

/// Used before queueing chroma jobs.
pub fn full_episode_fingerprint_cached_for_enqueue(ep: &OpEdEpisode) -> Result<bool, String> {
    episode_chroma_cache_complete(ep)
}

/// Pre-compute full-episode + phase-1 discovery fingerprints for one episode.
pub fn run_episode_chroma_fingerprint(
    ep: &OpEdEpisode,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Fingerprinting cancelled".to_string());
    }
    let duration = episode_duration_seconds(ep)?;
    let extract_len = full_episode_extract_len(duration);
    let regions = discovery_regions(duration);
    let total_steps = 1 + regions.len() as u32;
    let mut step = 0u32;

    on_step(step, total_steps, "Full episode");
    ensure_episode_fingerprint(&ep.path, 0.0, extract_len)?;
    step += 1;

    for (region_start, region_len) in regions {
        if cancel.load(Ordering::Relaxed) {
            return Err("Fingerprinting cancelled".to_string());
        }
        let label = if region_start.abs() < f64::EPSILON {
            "OP discovery windows"
        } else {
            "ED discovery windows"
        };
        on_step(step, total_steps, label);
        ensure_discovery_region_fingerprints(&ep.path, region_start, region_len)?;
        step += 1;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct FpcalcOutput {
    fingerprint: Vec<i32>,
}

fn samples_to_temp_raw_file(samples: &[i16], cache_key: &str) -> Result<PathBuf, String> {
    let mut path = fingerprint_cache_dir()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    path.push(format!("{cache_key}_{}_{}.s16le", std::process::id(), now));

    let mut file =
        fs::File::create(&path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes())
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    Ok(path)
}

fn pcm_to_chromaprint(samples: &[i16], cache_key: &str) -> Result<Fingerprint, String> {
    if samples.is_empty() {
        return Ok(Fingerprint { values: Vec::new() });
    }

    let fpcalc = fpcalc_path()?;
    let raw_path = samples_to_temp_raw_file(samples, cache_key)?;
    let sample_rate = SAMPLE_RATE.to_string();
    let output = hidden_command(&fpcalc)
        .args([
            "-raw",
            "-json",
            "-signed",
            "-length",
            "0",
            "-format",
            "s16le",
            "-rate",
            sample_rate.as_str(),
            "-channels",
            "1",
        ])
        .arg(&raw_path)
        .output()
        .map_err(|e| format!("failed to run fpcalc: {e}"));
    let _ = fs::remove_file(&raw_path);
    let output = output?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Empty fingerprint") {
            return Ok(Fingerprint { values: Vec::new() });
        }
        return Err(format!("fpcalc failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: FpcalcOutput = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("failed to parse fpcalc JSON: {e}: {stdout}"))?;
    Ok(Fingerprint {
        values: parsed.fingerprint,
    })
}

fn fingerprint_item_similarity(a: i32, b: i32) -> f32 {
    let distance = ((a as u32) ^ (b as u32)).count_ones();
    1.0 - distance as f32 / 32.0
}

fn fingerprint_similarity(a: &Fingerprint, a_frame: usize, b: &Fingerprint, b_frame: usize) -> f32 {
    if a_frame >= a.frame_count() || b_frame >= b.frame_count() {
        return 0.0;
    }
    fingerprint_item_similarity(a.values[a_frame], b.values[b_frame])
}

fn segment_fingerprint_similarity(template: &Fingerprint, candidate: &Fingerprint) -> f32 {
    let frames = template.frame_count().min(candidate.frame_count());
    if frames == 0 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for f in 0..frames {
        sum += fingerprint_similarity(template, f, candidate, f);
    }
    sum / frames as f32
}

fn sliding_match_score(
    template: &Fingerprint,
    candidate: &Fingerprint,
    offset_frames: usize,
) -> f32 {
    let overlap = template
        .frame_count()
        .min(candidate.frame_count().saturating_sub(offset_frames));
    if overlap == 0 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for f in 0..overlap {
        sum += fingerprint_similarity(template, f, candidate, offset_frames + f);
    }
    sum / overlap as f32
}

#[derive(Debug, Clone, Copy)]
struct MatchQuality {
    average: f32,
    strong_frame_ratio: f32,
    lower_quartile: f32,
}

fn match_quality_at_offset(
    template: &Fingerprint,
    candidate: &Fingerprint,
    offset_frames: usize,
) -> Option<MatchQuality> {
    let overlap = template
        .frame_count()
        .min(candidate.frame_count().saturating_sub(offset_frames));
    if overlap == 0 {
        return None;
    }

    let mut scores = Vec::with_capacity(overlap);
    let mut sum = 0.0f32;
    let mut strong = 0usize;
    for f in 0..overlap {
        let score = fingerprint_similarity(template, f, candidate, offset_frames + f);
        sum += score;
        if score >= MATCH_STRONG_FRAME_THRESHOLD {
            strong += 1;
        }
        scores.push(score);
    }
    scores.sort_by(|a, b| a.total_cmp(b));
    let lower_quartile = scores[overlap / 4];

    Some(MatchQuality {
        average: sum / overlap as f32,
        strong_frame_ratio: strong as f32 / overlap as f32,
        lower_quartile,
    })
}

fn match_quality_is_accepted(quality: MatchQuality) -> bool {
    match_quality_meets_thresholds(
        quality,
        MATCH_AVERAGE_THRESHOLD,
        MATCH_MIN_STRONG_FRAME_RATIO,
        MATCH_MIN_LOWER_QUARTILE,
    )
}

fn match_quality_meets_thresholds(
    quality: MatchQuality,
    average: f32,
    strong_ratio: f32,
    lower_quartile: f32,
) -> bool {
    quality.average >= average
        && quality.strong_frame_ratio >= strong_ratio
        && quality.lower_quartile >= lower_quartile
}

fn match_quality_near_miss_accepted(quality: MatchQuality) -> bool {
    match_quality_meets_thresholds(
        quality,
        MATCH_NEAR_MISS_AVERAGE_THRESHOLD,
        MATCH_NEAR_MISS_MIN_STRONG_FRAME_RATIO,
        MATCH_NEAR_MISS_MIN_LOWER_QUARTILE,
    )
}

fn with_center_trimmed_template<R>(
    template: &Fingerprint,
    lead_trim_frames: usize,
    tail_trim_frames: usize,
    f: impl FnOnce(&Fingerprint) -> Option<R>,
) -> Option<R> {
    if lead_trim_frames == 0 && tail_trim_frames == 0 {
        return f(template);
    }
    let remaining = template
        .frame_count()
        .saturating_sub(lead_trim_frames)
        .saturating_sub(tail_trim_frames);
    let min_frames = frames_for_seconds(MATCH_FALLBACK_MIN_TEMPLATE_SEC);
    if remaining < min_frames {
        return None;
    }
    let trimmed = slice_fingerprint(template, lead_trim_frames, remaining)?;
    f(&trimmed)
}

fn with_trimmed_template<R>(
    template: &Fingerprint,
    lead_trim_frames: usize,
    f: impl FnOnce(&Fingerprint) -> Option<R>,
) -> Option<R> {
    with_center_trimmed_template(template, lead_trim_frames, 0, f)
}

fn find_best_offset_and_quality(
    template: &Fingerprint,
    candidate: &Fingerprint,
    search_start_frame: usize,
    search_end_frame: usize,
) -> Option<(usize, MatchQuality)> {
    let template_frames = template.frame_count();
    if template_frames == 0 || candidate.frame_count() < template_frames {
        return None;
    }
    let end = search_end_frame.min(candidate.frame_count().saturating_sub(template_frames));
    if search_start_frame > end {
        return None;
    }
    let mut best_score = 0.0f32;
    let mut best_offset = search_start_frame;
    for offset in search_start_frame..=end {
        let score = sliding_match_score(template, candidate, offset);
        if score > best_score {
            best_score = score;
            best_offset = offset;
        }
    }
    let quality = match_quality_at_offset(template, candidate, best_offset)?;
    Some((best_offset, quality))
}

#[derive(Debug, Clone)]
struct TemplateMatch {
    start_sec: f64,
    end_sec: f64,
    confidence: f32,
}

fn template_match_from_offset(
    _template: &Fingerprint,
    offset_frames: usize,
    quality: MatchQuality,
) -> TemplateMatch {
    let start_sec = seconds_for_frames(offset_frames);
    TemplateMatch {
        start_sec,
        end_sec: start_sec + SEGMENT_DURATION_SEC,
        confidence: quality.average,
    }
}

fn find_best_match_in_candidate(
    template: &Fingerprint,
    candidate: &Fingerprint,
    search_start_frame: usize,
    search_end_frame: usize,
) -> Option<TemplateMatch> {
    let (offset, quality) =
        find_best_offset_and_quality(template, candidate, search_start_frame, search_end_frame)?;
    if !match_quality_is_accepted(quality) {
        return None;
    }
    Some(template_match_from_offset(template, offset, quality))
}

fn bridge_search_pass_label(pass: &str) -> &'static str {
    match pass {
        "optimistic" => "bridge_optimistic",
        "full" => "bridge_full",
        "trim_optimistic" => "bridge_trim_optimistic",
        "trim_full" => "bridge_trim_full",
        "trim_both_optimistic" => "bridge_trim_both_optimistic",
        "trim_both_full" => "bridge_trim_both_full",
        "edge_near" => "bridge_edge_near",
        _ => "bridge",
    }
}

/// Optimistic then full match; lead-trim retry; ED bidirectional trim; OP edge near-miss.
fn match_episode_against_template(
    template_fp: &Fingerprint,
    candidate_fp: &Fingerprint,
    kind: SegmentKind,
    optimistic_range: (usize, usize),
    full_range: (usize, usize),
) -> Option<(&'static str, TemplateMatch)> {
    let lead_trim = frames_for_seconds(MATCH_FALLBACK_LEAD_TRIM_SEC);
    let attempts: [(&str, (usize, usize), usize); 4] = [
        ("optimistic", optimistic_range, 0),
        ("full", full_range, 0),
        ("trim_optimistic", optimistic_range, lead_trim),
        ("trim_full", full_range, lead_trim),
    ];

    for (pass, (search_start, search_end), trim_frames) in attempts {
        if let Some(matched) = with_trimmed_template(template_fp, trim_frames, |work_template| {
            find_best_match_in_candidate(work_template, candidate_fp, search_start, search_end)
        }) {
            return Some((pass, matched));
        }
    }

    if kind == SegmentKind::Ed {
        let tail_trim = frames_for_seconds(MATCH_FALLBACK_TAIL_TRIM_SEC);
        let both_attempts: [(&str, (usize, usize)); 2] = [
            ("trim_both_optimistic", optimistic_range),
            ("trim_both_full", full_range),
        ];
        for (pass, (search_start, search_end)) in both_attempts {
            if let Some(matched) =
                with_center_trimmed_template(template_fp, lead_trim, tail_trim, |work_template| {
                    find_best_match_in_candidate(
                        work_template,
                        candidate_fp,
                        search_start,
                        search_end,
                    )
                })
            {
                return Some((pass, matched));
            }
        }
        return None;
    }

    let edge_max_offset = optimistic_range.0 + frames_for_seconds(MATCH_EDGE_NEAR_OFFSET_SEC);

    with_trimmed_template(template_fp, lead_trim, |work_template| {
        let (offset, quality) = find_best_offset_and_quality(
            work_template,
            candidate_fp,
            optimistic_range.0,
            optimistic_range.1,
        )
        .or_else(|| {
            find_best_offset_and_quality(
                work_template,
                candidate_fp,
                full_range.0,
                full_range.1,
            )
        })?;

        if offset > edge_max_offset || !match_quality_near_miss_accepted(quality) {
            return None;
        }

        Some((
            "edge_near",
            template_match_from_offset(work_template, offset, quality),
        ))
    })
}

fn frames_for_seconds(sec: f64) -> usize {
    Fingerprint::frames_for_duration(sec)
}

fn seconds_for_frames(frames: usize) -> f64 {
    frames as f64 * CHROMAPRINT_FRAME_SEC
}

fn full_episode_extract_len(duration_sec: f64) -> f64 {
    duration_sec.max(SEGMENT_DURATION_SEC + 1.0)
}

fn sample_count_for_seconds(sec: f64) -> usize {
    (sec * SAMPLE_RATE as f64).round() as usize
}

fn segment_fingerprint_cache_key(
    path_buf: &Path,
    start_sec: f64,
    duration_sec: f64,
) -> Result<String, String> {
    Ok(format!(
        "cp{}_{}_{}_{}",
        ANALYSIS_VERSION,
        cache_key(path_buf)?,
        (start_sec * 1000.0) as i64,
        (duration_sec * 1000.0) as i64
    ))
}

fn discovery_regions(duration_sec: f64) -> Vec<(f64, f64)> {
    let mut regions = Vec::new();
    let op_len = OP_SEARCH_SEC.min(duration_sec);
    if op_len >= SEED_MIN_REGION_SEC {
        regions.push((0.0, op_len));
    }
    let ed_tail = ED_TAIL_SEC.min(duration_sec);
    if ed_tail >= SEED_MIN_REGION_SEC {
        regions.push(((duration_sec - ed_tail).max(0.0), ed_tail));
    }
    regions
}

fn discovery_window_starts(region_start_sec: f64, region_len_sec: f64) -> Vec<f64> {
    let mut starts = Vec::new();
    let mut offset = 0.0f64;
    while offset + SEED_WINDOW_SEC <= region_len_sec {
        starts.push(region_start_sec + offset);
        offset += SEED_WINDOW_SEC / 2.0;
    }
    starts
}

fn discovery_fingerprints_cached(ep: &OpEdEpisode) -> Result<bool, String> {
    let duration = episode_duration_seconds(ep)?;
    let path_buf = normalized_video_path(&ep.path)?;
    for (region_start, region_len) in discovery_regions(duration) {
        for window_start in discovery_window_starts(region_start, region_len) {
            let key = segment_fingerprint_cache_key(&path_buf, window_start, SEED_WINDOW_SEC)?;
            if load_fingerprint(&key)?.is_none() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// One ffmpeg decode per region; each discovery window still gets isolated fpcalc on its PCM slice.
fn ensure_discovery_region_fingerprints(
    path: &str,
    region_start_sec: f64,
    region_len_sec: f64,
) -> Result<(), String> {
    if region_len_sec < SEED_MIN_REGION_SEC {
        return Ok(());
    }
    let path_buf = normalized_video_path(path)?;
    let window_starts = discovery_window_starts(region_start_sec, region_len_sec);
    if window_starts.is_empty() {
        return Ok(());
    }

    let needs_decode = window_starts.iter().any(|&window_start| {
        segment_fingerprint_cache_key(&path_buf, window_start, SEED_WINDOW_SEC)
            .ok()
            .and_then(|key| load_fingerprint(&key).ok().flatten())
            .is_none()
    });
    if !needs_decode {
        return Ok(());
    }

    let samples = extract_pcm_range(&path_buf, region_start_sec, region_len_sec, SAMPLE_RATE)?;
    let window_samples = sample_count_for_seconds(SEED_WINDOW_SEC);
    for window_start in window_starts {
        let offset_in_region = window_start - region_start_sec;
        let offset_samples = sample_count_for_seconds(offset_in_region);
        if offset_samples + window_samples > samples.len() {
            continue;
        }
        let key = segment_fingerprint_cache_key(&path_buf, window_start, SEED_WINDOW_SEC)?;
        if load_fingerprint(&key)?.is_some() {
            continue;
        }
        let window_pcm = &samples[offset_samples..offset_samples + window_samples];
        let fp = pcm_to_chromaprint(window_pcm, &key)?;
        save_fingerprint(&key, &fp)?;
    }
    Ok(())
}

fn slice_fingerprint(
    src: &Fingerprint,
    start_frame: usize,
    window_frames: usize,
) -> Option<Fingerprint> {
    let end = start_frame.saturating_add(window_frames);
    if window_frames == 0 || end > src.frame_count() {
        return None;
    }
    Some(Fingerprint {
        values: src.values[start_frame..end].to_vec(),
    })
}

fn save_fingerprint(cache_key: &str, fp: &Fingerprint) -> Result<(), String> {
    let path = fingerprint_path(cache_key)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&(fp.values.len() as u32).to_le_bytes());
    for value in &fp.values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(&path, payload).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

fn load_fingerprint(cache_key: &str) -> Result<Option<Fingerprint>, String> {
    let path = fingerprint_path(cache_key)?;
    if !path.is_file() {
        return Ok(None);
    }
    let payload = fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    if payload.len() < 4 {
        return Ok(None);
    }
    let frame_count = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let expected_len = frame_count * 4;
    if payload.len() < 4 + expected_len {
        return Ok(None);
    }
    let mut values = Vec::with_capacity(frame_count);
    for chunk in payload[4..4 + expected_len].chunks_exact(4) {
        values.push(i32::from_le_bytes(chunk.try_into().unwrap()));
    }
    Ok(Some(Fingerprint { values }))
}

fn extract_fingerprint_from_file(
    path: &Path,
    start_sec: f64,
    duration_sec: f64,
    cache_key: &str,
) -> Result<Fingerprint, String> {
    let samples = extract_pcm_range(path, start_sec, duration_sec, SAMPLE_RATE)?;
    pcm_to_chromaprint(&samples, cache_key)
}

fn ensure_episode_fingerprint(
    path: &str,
    start_sec: f64,
    duration_sec: f64,
) -> Result<(String, Fingerprint), String> {
    let path_buf = normalized_video_path(path)?;
    let key = segment_fingerprint_cache_key(&path_buf, start_sec, duration_sec)?;
    if let Some(fp) = load_fingerprint(&key)? {
        return Ok((key, fp));
    }
    let fp = extract_fingerprint_from_file(&path_buf, start_sec, duration_sec, &key)?;
    save_fingerprint(&key, &fp)?;
    Ok((key, fp))
}

#[derive(Debug, Clone)]
struct SeedCandidate {
    start_sec: f64,
    episode_id: i64,
    fingerprint: Fingerprint,
}

fn kind_optimistic_search_range(kind: SegmentKind, duration: f64) -> (usize, usize) {
    match kind {
        SegmentKind::Op => {
            let end_sec = OP_SEARCH_SEC.min(duration);
            (0, frames_for_seconds(end_sec))
        }
        SegmentKind::Ed => {
            let tail = ED_TAIL_SEC.min(duration);
            let start = (duration - tail).max(0.0);
            (frames_for_seconds(start), frames_for_seconds(duration))
        }
    }
}

/// Best template offset in a search region (no consistency gates — used for seed refinement).
fn find_anchor_offset(
    template: &Fingerprint,
    candidate: &Fingerprint,
    search_start_frame: usize,
    search_end_frame: usize,
) -> Option<usize> {
    let template_frames = template.frame_count();
    if template_frames == 0 || candidate.frame_count() < template_frames {
        return None;
    }
    let end = search_end_frame.min(candidate.frame_count().saturating_sub(template_frames));
    if search_start_frame > end {
        return None;
    }
    let mut best_score = 0.0f32;
    let mut best_offset = search_start_frame;
    for offset in search_start_frame..=end {
        let score = sliding_match_score(template, candidate, offset);
        if score > best_score {
            best_score = score;
            best_offset = offset;
        }
    }
    if best_score >= SEED_REFINE_ANCHOR_THRESHOLD {
        Some(best_offset)
    } else {
        None
    }
}

/// Steps reserved per segment kind: scan each seed episode, compare, refine, then build or bail.
fn discovery_steps_per_kind(episode_count: usize) -> u32 {
    if episode_count < 2 {
        return 0;
    }
    episode_count.min(MAX_SEED_EPISODES) as u32 + 4
}

fn max_detection_blocks(episode_count: usize) -> u32 {
    if episode_count < 2 {
        return 1;
    }
    let by_streak =
        (episode_count / FULL_PASS_FAIL_STREAK_FOR_NO_OP_ED).saturating_add(1) as u32;
    by_streak.min(MAX_OP_ED_BLOCKS)
}

fn op_ed_detect_total_steps(episode_count: usize) -> u32 {
    if episode_count < 2 {
        // Starting + OP skip + ED skip + Done
        return 4;
    }
    let blocks = max_detection_blocks(episode_count);
    let per_kind_block = discovery_steps_per_kind(episode_count) + episode_count as u32;
    let per_kind = per_kind_block * blocks;
    // Starting + (discovery + per-episode match) × 2 kinds + Done
    1 + per_kind * 2 + 1
}

fn discover_repeated_seed(
    episodes: &[EpisodeRow],
    kind: SegmentKind,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&str),
) -> Result<Option<(SeedCandidate, Vec<i64>)>, String> {
    let pool: Vec<_> = episodes.iter().take(MAX_SEED_EPISODES).collect();
    if pool.len() < 2 {
        return Ok(None);
    }

    let mut seeds: Vec<SeedCandidate> = Vec::new();
    for (index, ep) in pool.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }
        report(&format!(
            "{} discovery: scanning episode {}/{}",
            kind.display_name(),
            index + 1,
            pool.len()
        ));
        let duration = if ep.duration_seconds > 0.0 {
            ep.duration_seconds
        } else {
            let path = normalized_video_path(&ep.path)?;
            probe_duration(&path)?
        };
        let (region_start_sec, region_len_sec) = match kind {
            SegmentKind::Op => (0.0, OP_SEARCH_SEC.min(duration)),
            SegmentKind::Ed => {
                let tail = ED_TAIL_SEC.min(duration);
                ((duration - tail).max(0.0), tail)
            }
        };
        if region_len_sec < SEED_MIN_REGION_SEC {
            continue;
        }

        let mut offset = 0.0f64;
        while offset + SEED_WINDOW_SEC <= region_len_sec {
            if cancel.load(Ordering::Relaxed) {
                return Err("OP/ED detection cancelled".to_string());
            }
            let window_start = region_start_sec + offset;
            let (_, fp) =
                ensure_episode_fingerprint(&ep.path, window_start, SEED_WINDOW_SEC)?;
            seeds.push(SeedCandidate {
                start_sec: window_start,
                episode_id: ep.id,
                fingerprint: fp,
            });
            offset += SEED_WINDOW_SEC / 2.0;
        }
    }

    report(&format!(
        "{} discovery: comparing fingerprints",
        kind.display_name()
    ));

    Ok(cluster_seed_candidates(&seeds))
}

fn cluster_seed_candidates(seeds: &[SeedCandidate]) -> Option<(SeedCandidate, Vec<i64>)> {
    let mut best_cluster: Option<(SeedCandidate, Vec<i64>, usize)> = None;
    for (i, seed_a) in seeds.iter().enumerate() {
        let mut matching_eps = vec![seed_a.episode_id];
        for (j, seed_b) in seeds.iter().enumerate() {
            if i == j || seed_a.episode_id == seed_b.episode_id {
                continue;
            }
            let sim = segment_fingerprint_similarity(&seed_a.fingerprint, &seed_b.fingerprint);
            if sim >= SEED_MATCH_THRESHOLD {
                if !matching_eps.contains(&seed_b.episode_id) {
                    matching_eps.push(seed_b.episode_id);
                }
            }
        }
        if best_cluster
            .as_ref()
            .is_none_or(|(_, _, count)| matching_eps.len() > *count)
        {
            let count = matching_eps.len();
            best_cluster = Some((seed_a.clone(), matching_eps, count));
        }
    }
    let (seed, episode_ids, count) = best_cluster?;
    if count < 2 {
        return None;
    }
    Some((seed, episode_ids))
}

/// Refined template must match at least as many episodes as coarse, and coarse must already work.
fn refinement_beats_coarse(coarse_hits: usize, refined_hits: usize) -> bool {
    coarse_hits >= 2 && refined_hits >= coarse_hits
}

fn count_episode_template_matches(
    template_fp: &Fingerprint,
    episode_ids: &[i64],
    episodes: &[EpisodeRow],
    kind: SegmentKind,
) -> Result<usize, String> {
    let mut hits = 0usize;
    for ep_id in episode_ids {
        let Some(ep) = episodes.iter().find(|e| e.id == *ep_id) else {
            continue;
        };
        let duration = if ep.duration_seconds > 0.0 {
            ep.duration_seconds
        } else {
            let path = normalized_video_path(&ep.path)?;
            probe_duration(&path)?
        };
        let extract_len = full_episode_extract_len(duration);
        let (_, candidate_fp) = ensure_episode_fingerprint(&ep.path, 0.0, extract_len)?;
        let optimistic = kind_optimistic_search_range(kind, duration);
        let full = (0, frames_for_seconds(duration));
        if match_episode_against_template(
            template_fp,
            &candidate_fp,
            kind,
            optimistic,
            full,
        )
        .is_some()
        {
            hits += 1;
        }
    }
    Ok(hits)
}

struct AnchoredSeedEpisode {
    episode_id: i64,
    anchor_frame: usize,
    full: Fingerprint,
}

/// Second discovery pass: anchor the coarse 90s template on each phase-1 seed episode and
/// drop failures, then slide the template in unison ±`SEED_REFINE_OFFSET_SEC` at
/// `SEED_REFINE_OFFSET_STEP_SEC` steps by re-slicing the reference fingerprint (drop left /
/// add right). Pick the shift with the highest average sliding score across survivors.
fn refine_discovered_seed(
    initial_seed: &SeedCandidate,
    source_ids: &[i64],
    episodes: &[EpisodeRow],
    template_fp: &Fingerprint,
    kind: SegmentKind,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&str),
) -> Result<(SeedCandidate, Vec<i64>), String> {
    report(&format!(
        "{} discovery: refining seed alignment",
        kind.display_name()
    ));

    let Some(ref_ep) = episodes.iter().find(|e| e.id == initial_seed.episode_id) else {
        return Ok((initial_seed.clone(), source_ids.to_vec()));
    };
    let ref_duration = if ref_ep.duration_seconds > 0.0 {
        ref_ep.duration_seconds
    } else {
        let path = normalized_video_path(&ref_ep.path)?;
        probe_duration(&path)?
    };
    let ref_extract_len = full_episode_extract_len(ref_duration);
    let (_, ref_full) = ensure_episode_fingerprint(&ref_ep.path, 0.0, ref_extract_len)?;

    let template_frames = template_fp.frame_count();
    let coarse_start_sec = initial_seed.start_sec;

    let mut anchored: Vec<AnchoredSeedEpisode> = Vec::new();
    for ep_id in source_ids {
        if cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }
        let Some(ep) = episodes.iter().find(|e| e.id == *ep_id) else {
            continue;
        };
        let duration = if ep.duration_seconds > 0.0 {
            ep.duration_seconds
        } else {
            let path = normalized_video_path(&ep.path)?;
            probe_duration(&path)?
        };
        let full_extract_len = full_episode_extract_len(duration);
        let (_, full) = ensure_episode_fingerprint(&ep.path, 0.0, full_extract_len)?;
        let (search_start, search_end) = kind_optimistic_search_range(kind, duration);
        let Some(anchor_frame) =
            find_anchor_offset(template_fp, &full, search_start, search_end)
        else {
            continue;
        };
        anchored.push(AnchoredSeedEpisode {
            episode_id: ep.id,
            anchor_frame,
            full,
        });
    }

    let refined_ids: Vec<i64> = anchored.iter().map(|a| a.episode_id).collect();
    if anchored.len() < 2 {
        return Ok((initial_seed.clone(), refined_ids));
    }

    report(&format!(
        "{} discovery: sliding template over {} episodes",
        kind.display_name(),
        anchored.len()
    ));

    let mut best_delta_sec = 0.0f64;
    let mut best_avg_score = f32::NEG_INFINITY;

    let mut offset_sec = -(SEED_REFINE_OFFSET_SEC as f64);
    while offset_sec <= SEED_REFINE_OFFSET_SEC as f64 + f64::EPSILON {
        if cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }
        let shifted_start_sec = coarse_start_sec + offset_sec;
        if shifted_start_sec < 0.0 {
            offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
            continue;
        }
        let shifted_start_frame = frames_for_seconds(shifted_start_sec);
        let Some(shifted_template) =
            slice_fingerprint(&ref_full, shifted_start_frame, template_frames)
        else {
            offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
            continue;
        };

        let mut score_sum = 0.0f32;
        let mut valid = true;
        for ep in &anchored {
            let match_sec = seconds_for_frames(ep.anchor_frame) + offset_sec;
            if match_sec < 0.0 {
                valid = false;
                break;
            }
            let match_frame = frames_for_seconds(match_sec);
            if match_frame + template_frames > ep.full.frame_count() {
                valid = false;
                break;
            }
            score_sum += sliding_match_score(&shifted_template, &ep.full, match_frame);
        }
        if valid {
            let avg = score_sum / anchored.len() as f32;
            if avg > best_avg_score
                || (avg == best_avg_score && offset_sec.abs() < best_delta_sec.abs())
            {
                best_avg_score = avg;
                best_delta_sec = offset_sec;
            }
        }
        offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
    }

    if !best_avg_score.is_finite() {
        return Ok((initial_seed.clone(), refined_ids));
    }

    Ok((
        SeedCandidate {
            start_sec: coarse_start_sec + best_delta_sec,
            episode_id: initial_seed.episode_id,
            fingerprint: initial_seed.fingerprint.clone(),
        },
        refined_ids,
    ))
}

fn build_template_fingerprint(
    episode_path: &str,
    start_sec: f64,
    cancel: &AtomicBool,
) -> Result<(Fingerprint, String), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("OP/ED detection cancelled".to_string());
    }
    // Isolated segment fpcalc — must match discovery windows; do not slice from full-episode cache.
    let (key, fp) = ensure_episode_fingerprint(episode_path, start_sec, SEGMENT_DURATION_SEC)?;
    Ok((fp, key))
}

fn upsert_segment_status_conn(
    conn: &Connection,
    episode_id: i64,
    kind: SegmentKind,
    status: OpEdSegmentStatus,
    start_sec: Option<f64>,
    end_sec: Option<f64>,
    confidence: Option<f64>,
    template_id: Option<i64>,
    search_pass: &str,
    fp_key: Option<&str>,
    error_text: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO episode_op_ed_segments
            (episode_id, kind, status, start_sec, end_sec, confidence, template_id,
             search_pass, fingerprint_cache_key, error_text, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, CURRENT_TIMESTAMP)
         ON CONFLICT(episode_id, kind) DO UPDATE SET
            status = excluded.status,
            start_sec = COALESCE(excluded.start_sec, start_sec),
            end_sec = COALESCE(excluded.end_sec, end_sec),
            confidence = COALESCE(excluded.confidence, confidence),
            template_id = excluded.template_id,
            search_pass = excluded.search_pass,
            fingerprint_cache_key = COALESCE(excluded.fingerprint_cache_key, fingerprint_cache_key),
            error_text = excluded.error_text,
            updated_at = CURRENT_TIMESTAMP",
        params![
            episode_id,
            kind.as_str(),
            status.as_str(),
            start_sec,
            end_sec,
            confidence,
            template_id,
            search_pass,
            fp_key,
            error_text,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct DetectContext<'a> {
    db: &'a AppDatabase,
    anime_id: i64,
    episodes: &'a [EpisodeRow],
    continue_templates: bool,
    rematch_matched: bool,
    cancel: &'a AtomicBool,
    on_progress: &'a dyn Fn(u32, u32, &str),
}

struct ExistingKindTemplate {
    template_id: i64,
    template_fp: Fingerprint,
    block_index: i32,
}

impl DetectContext<'_> {
    fn upsert_segment_status(
        &self,
        episode_id: i64,
        kind: SegmentKind,
        status: OpEdSegmentStatus,
        start_sec: Option<f64>,
        end_sec: Option<f64>,
        confidence: Option<f64>,
        template_id: Option<i64>,
        search_pass: &str,
        fp_key: Option<&str>,
        error_text: Option<&str>,
    ) -> Result<(), String> {
        self.db.with_conn(|conn| {
            upsert_segment_status_conn(
                conn,
                episode_id,
                kind,
                status,
                start_sec,
                end_sec,
                confidence,
                template_id,
                search_pass,
                fp_key,
                error_text,
            )
        })
    }

    fn segment_is_matched(&self, episode_id: i64, kind: SegmentKind) -> Result<bool, String> {
        self.db
            .with_conn(|conn| segment_has_status(conn, episode_id, kind, "matched"))
    }

    fn segment_skip_in_match_pass(
        &self,
        episode_id: i64,
        kind: SegmentKind,
    ) -> Result<bool, String> {
        if self.rematch_matched {
            return Ok(self.db.with_conn(|conn| {
                segment_has_status(conn, episode_id, kind, "skipped")
            })?);
        }
        self.db.with_conn(|conn| {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM episode_op_ed_segments WHERE episode_id = ?1 AND kind = ?2",
                    params![episode_id, kind.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            Ok(matches!(
                status.as_deref(),
                Some("matched") | Some("skipped")
            ))
        })
    }

    /// True when exactly one of OP/ED is matched on this episode (season-bridge edge case).
    fn episode_has_single_kind_match(&self, episode_id: i64) -> Result<bool, String> {
        let count: i64 = self.db.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM episode_op_ed_segments
                 WHERE episode_id = ?1 AND status = 'matched'",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })?;
        Ok(count == 1)
    }

    fn insert_template(
        &self,
        kind: SegmentKind,
        block_index: i32,
        start_sec: f64,
        duration_sec: f64,
        confidence: f32,
        fp_key: &str,
        source_ids: &[i64],
    ) -> Result<i64, String> {
        self.db.with_conn(|conn| {
            insert_template(
                conn,
                self.anime_id,
                kind,
                block_index,
                start_sec,
                duration_sec,
                confidence,
                fp_key,
                source_ids,
                "auto",
            )
        })
    }

    fn mark_no_op_ed(&self) -> Result<(), String> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE anime SET no_op_ed = 1 WHERE id = ?1",
                params![self.anime_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn episode_duration(&self, ep: &EpisodeRow) -> Result<f64, String> {
        if ep.duration_seconds > 0.0 {
            return Ok(ep.duration_seconds);
        }
        let path = normalized_video_path(&ep.path)?;
        Ok(probe_duration(&path)?)
    }

    fn optimistic_search_range(&self, kind: SegmentKind, duration: f64) -> (usize, usize) {
        kind_optimistic_search_range(kind, duration)
    }

    fn full_search_range(&self, duration: f64) -> (usize, usize) {
        (0, frames_for_seconds(duration))
    }
}

fn load_latest_kind_template(
    ctx: &DetectContext<'_>,
    kind: SegmentKind,
) -> Result<Option<ExistingKindTemplate>, String> {
    ctx.db.with_conn(|conn| {
        let row: Option<(i64, i32, String)> = conn
            .query_row(
                "SELECT id, block_index, fingerprint_cache_key FROM op_ed_templates
                 WHERE anime_id = ?1 AND kind = ?2 AND source = 'auto'
                 ORDER BY block_index DESC, id DESC
                 LIMIT 1",
                params![ctx.anime_id, kind.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((template_id, block_index, fp_key)) = row else {
            return Ok(None);
        };
        let Some(template_fp) = load_fingerprint(&fp_key)? else {
            return Ok(None);
        };
        Ok(Some(ExistingKindTemplate {
            template_id,
            template_fp,
            block_index,
        }))
    })
}

fn load_episodes(conn: &Connection, anime_id: i64) -> Result<Vec<EpisodeRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, duration_seconds FROM episodes
             WHERE anime_id = ?1 AND missing = 0
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| {
            Ok(EpisodeRow {
                id: row.get(0)?,
                path: row.get(1)?,
                duration_seconds: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Subset of a title's episodes (in library order) for one detect batch.
pub fn load_episodes_for_detect(
    conn: &Connection,
    anime_id: i64,
    episode_ids: &[i64],
) -> Result<Vec<EpisodeRow>, String> {
    let all = load_episodes(conn, anime_id)?;
    if episode_ids.is_empty() {
        return Ok(all);
    }
    let wanted: std::collections::HashSet<i64> = episode_ids.iter().copied().collect();
    Ok(all
        .into_iter()
        .filter(|ep| wanted.contains(&ep.id))
        .collect())
}

fn ensure_pending_segments_for_anime(conn: &Connection, anime_id: i64) -> Result<(), String> {
    let episodes = load_episodes(conn, anime_id)?;
    for ep in &episodes {
        for kind in [SegmentKind::Op, SegmentKind::Ed] {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM episode_op_ed_segments WHERE episode_id = ?1 AND kind = ?2",
                    params![ep.id, kind.as_str()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .is_some();
            if !exists {
                conn.execute(
                    "INSERT INTO episode_op_ed_segments (episode_id, kind, status)
                     VALUES (?1, ?2, 'pending')",
                    params![ep.id, kind.as_str()],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn insert_template(
    conn: &Connection,
    anime_id: i64,
    kind: SegmentKind,
    block_index: i32,
    start_sec: f64,
    duration_sec: f64,
    confidence: f32,
    fp_key: &str,
    source_ids: &[i64],
    source: &str,
) -> Result<i64, String> {
    let source_json = serde_json::to_string(source_ids).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO op_ed_templates
            (anime_id, kind, block_index, start_sec, duration_sec, confidence,
             fingerprint_cache_key, source_episode_ids, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            anime_id,
            kind.as_str(),
            block_index,
            start_sec,
            duration_sec,
            confidence,
            fp_key,
            source_json,
            source,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Episodes used for repeated-segment discovery in the current block.
fn discovery_seed_episodes(
    ctx: &DetectContext<'_>,
    episodes: &[EpisodeRow],
    kind: SegmentKind,
    block_index: i32,
) -> Result<Vec<EpisodeRow>, String> {
    if ctx.rematch_matched && block_index == 0 {
        return Ok(episodes.to_vec());
    }
    Ok(episodes
        .iter()
        .filter(|ep| {
            ctx.segment_is_matched(ep.id, kind)
                .map(|matched| !matched)
                .unwrap_or(true)
        })
        .cloned()
        .collect())
}

fn demote_matched_segments_for_rematch(
    db: &AppDatabase,
    episode_ids: &[i64],
) -> Result<(), String> {
    if episode_ids.is_empty() {
        return Ok(());
    }
    db.with_conn(|conn| {
        for episode_id in episode_ids {
            for kind in [SegmentKind::Op, SegmentKind::Ed] {
                let matched = segment_has_status(conn, *episode_id, kind, "matched")?;
                if matched {
                    upsert_segment_status_conn(
                        conn,
                        *episode_id,
                        kind,
                        OpEdSegmentStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                        "rematch",
                        None,
                        None,
                    )?;
                }
            }
        }
        Ok(())
    })
}

fn segment_has_status(
    conn: &Connection,
    episode_id: i64,
    kind: SegmentKind,
    expected: &str,
) -> Result<bool, String> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM episode_op_ed_segments WHERE episode_id = ?1 AND kind = ?2",
            params![episode_id, kind.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(status.as_deref() == Some(expected))
}

fn record_episode_match_result(
    ctx: &DetectContext<'_>,
    ep: &EpisodeRow,
    kind: SegmentKind,
    template_id: i64,
    candidate_key: &str,
    matched: Option<TemplateMatch>,
    search_pass: &str,
) -> Result<(), String> {
    if let Some(m) = matched {
        ctx.upsert_segment_status(
            ep.id,
            kind,
            OpEdSegmentStatus::Matched,
            Some(m.start_sec),
            Some(m.end_sec),
            Some(f64::from(m.confidence)),
            Some(template_id),
            search_pass,
            Some(candidate_key),
            None,
        )?;
    } else if ctx.rematch_matched && ctx.segment_is_matched(ep.id, kind)? {
        // Rare after demote_rematch; keep times, clear matched so later blocks can discover.
        ctx.upsert_segment_status(
            ep.id,
            kind,
            OpEdSegmentStatus::Pending,
            None,
            None,
            None,
            Some(template_id),
            search_pass,
            None,
            None,
        )?;
    } else {
        ctx.upsert_segment_status(
            ep.id,
            kind,
            OpEdSegmentStatus::NotFound,
            None,
            None,
            None,
            Some(template_id),
            search_pass,
            None,
            None,
        )?;
    }
    Ok(())
}

fn run_kind_detection(
    ctx: &DetectContext<'_>,
    kind: SegmentKind,
    step: &mut u32,
    step_total: u32,
) -> Result<(), String> {
    let episodes = ctx.episodes;
    let mut tick = |label: &str| {
        (ctx.on_progress)(*step, step_total, label);
        *step += 1;
    };

    if episodes.len() < 2 {
        tick(&format!(
            "{}: skipped (need at least 2 episodes)",
            kind.display_name()
        ));
        for ep in episodes {
            ctx.upsert_segment_status(
                ep.id,
                kind,
                OpEdSegmentStatus::Skipped,
                None,
                None,
                None,
                None,
                "none",
                None,
                Some("insufficient episodes"),
            )?;
        }
        return Ok(());
    }

    let max_blocks = max_detection_blocks(episodes.len());
    let mut block_index = 0i32;
    let mut bridge_episode_id: Option<i64> = None;
    let mut try_reuse_templates = ctx.continue_templates;

    loop {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }

        let seed_pool: Vec<EpisodeRow> =
            discovery_seed_episodes(ctx, episodes, kind, block_index)?;

        let can_match_with_saved_template = try_reuse_templates
            && block_index == 0
            && load_latest_kind_template(ctx, kind)?.is_some();

        if seed_pool.len() < 2 && !can_match_with_saved_template {
            if block_index == 0 {
                tick(&format!(
                    "{} discovery: no repeated segment found",
                    kind.display_name()
                ));
                if episodes.len() >= MIN_EPISODES_FOR_NO_OP_ED {
                    ctx.mark_no_op_ed()?;
                }
                for ep in episodes {
                    if !ctx.segment_is_matched(ep.id, kind)? {
                        ctx.upsert_segment_status(
                            ep.id,
                            kind,
                            OpEdSegmentStatus::NotFound,
                            None,
                            None,
                            None,
                            None,
                            "seed",
                            None,
                            None,
                        )?;
                    }
                }
            }
            break;
        }

        let template_bundle: Option<(Fingerprint, i64)> = 'template: {
            if try_reuse_templates && block_index == 0 {
                try_reuse_templates = false;
                if let Some(existing) = load_latest_kind_template(ctx, kind)? {
                    tick(&format!(
                        "{} block {}: reusing saved template",
                        kind.display_name(),
                        existing.block_index + 1
                    ));
                    block_index = existing.block_index;
                    break 'template Some((existing.template_fp, existing.template_id));
                }
            }

            if block_index > 0 {
                tick(&format!(
                    "{} block {}: discovering new segment",
                    kind.display_name(),
                    block_index + 1
                ));
            }

            let seed_result =
                discover_repeated_seed(&seed_pool, kind, ctx.cancel, &mut |label| tick(label))?;
            let Some((seed, source_ids)) = seed_result else {
                tick(&format!(
                    "{} block {}: no repeated segment in remaining episodes",
                    kind.display_name(),
                    block_index + 1
                ));
                if block_index == 0 && episodes.len() >= MIN_EPISODES_FOR_NO_OP_ED {
                    ctx.mark_no_op_ed()?;
                }
                for ep in &seed_pool {
                    if !ctx.segment_is_matched(ep.id, kind)? {
                        ctx.upsert_segment_status(
                            ep.id,
                            kind,
                            OpEdSegmentStatus::NotFound,
                            None,
                            None,
                            None,
                            None,
                            "seed",
                            None,
                            None,
                        )?;
                    }
                }
                break 'template None;
            };

            tick(&format!(
                "{} block {}: building template",
                kind.display_name(),
                block_index + 1
            ));

            let coarse_path = episodes
                .iter()
                .find(|e| e.id == seed.episode_id)
                .map(|e| e.path.as_str())
                .ok_or("seed episode not found")?;
            let (coarse_template_fp, coarse_template_key) = build_template_fingerprint(
                coarse_path,
                seed.start_sec,
                ctx.cancel,
            )?;
            let validate_ids: Vec<i64> = episodes.iter().map(|e| e.id).collect();
            let coarse_hits = count_episode_template_matches(
                &coarse_template_fp,
                &validate_ids,
                episodes,
                kind,
            )?;
            let (refined_seed, refined_source_ids) = refine_discovered_seed(
                &seed,
                &source_ids,
                episodes,
                &coarse_template_fp,
                kind,
                ctx.cancel,
                &mut |label| tick(label),
            )?;

            let refinement_changed = (refined_seed.start_sec - seed.start_sec).abs() > 0.05
                || refined_seed.episode_id != seed.episode_id
                || refined_source_ids != source_ids;

            let (final_seed, final_source_ids, template_fp, template_key) =
                if refinement_changed {
                    let (refined_template_fp, refined_template_key) = build_template_fingerprint(
                        coarse_path,
                        refined_seed.start_sec,
                        ctx.cancel,
                    )?;
                    let refined_hits = count_episode_template_matches(
                        &refined_template_fp,
                        &validate_ids,
                        episodes,
                        kind,
                    )?;
                    if refinement_beats_coarse(coarse_hits, refined_hits) {
                        (
                            refined_seed,
                            refined_source_ids,
                            refined_template_fp,
                            refined_template_key,
                        )
                    } else {
                        tick(&format!(
                            "{} block {}: keeping coarse seed (refined {}/{} vs coarse {}/{})",
                            kind.display_name(),
                            block_index + 1,
                            refined_hits,
                            validate_ids.len(),
                            coarse_hits,
                            validate_ids.len(),
                        ));
                        (
                            seed.clone(),
                            source_ids.clone(),
                            coarse_template_fp,
                            coarse_template_key,
                        )
                    }
                } else {
                    (
                        seed.clone(),
                        source_ids.clone(),
                        coarse_template_fp,
                        coarse_template_key,
                    )
                };

            let template_id = ctx.insert_template(
                kind,
                block_index,
                final_seed.start_sec,
                SEGMENT_DURATION_SEC,
                1.0,
                &template_key,
                &final_source_ids,
            )?;
            break 'template Some((template_fp, template_id));
        };
        let Some((template_fp, template_id)) = template_bundle else {
            break;
        };

        if let Some(bridge_id) = bridge_episode_id.take() {
            if ctx.episode_has_single_kind_match(bridge_id)?
                && !ctx.segment_is_matched(bridge_id, kind)?
            {
                if let Some(bridge_ep) = episodes.iter().find(|e| e.id == bridge_id) {
                    tick(&format!(
                        "{} block {}: bridge episode retro match",
                        kind.display_name(),
                        block_index + 1
                    ));
                    ctx.upsert_segment_status(
                        bridge_id,
                        kind,
                        OpEdSegmentStatus::Analyzing,
                        None,
                        None,
                        None,
                        Some(template_id),
                        "bridge",
                        None,
                        None,
                    )?;
                    let duration = ctx.episode_duration(bridge_ep)?;
                    let optimistic = ctx.optimistic_search_range(kind, duration);
                    let full = ctx.full_search_range(duration);
                    let extract_len = full_episode_extract_len(duration);
                    let (candidate_key, candidate_fp) =
                        ensure_episode_fingerprint(&bridge_ep.path, 0.0, extract_len)?;
                    let bridge_match = match_episode_against_template(
                        &template_fp,
                        &candidate_fp,
                        kind,
                        optimistic,
                        full,
                    );
                    let (search_pass, matched) = bridge_match
                        .map(|(pass, m)| (bridge_search_pass_label(pass), Some(m)))
                        .unwrap_or(("bridge", None));
                    record_episode_match_result(
                        ctx,
                        bridge_ep,
                        kind,
                        template_id,
                        &candidate_key,
                        matched,
                        search_pass,
                    )?;
                }
            }
        }

        let mut fail_streak = 0usize;
        let mut fail_streak_start_id: Option<i64> = None;
        let mut done = 0u32;
        let total = episodes.len() as u32;
        let mut block_transition = false;

        for ep in episodes {
            if ctx.cancel.load(Ordering::Relaxed) {
                return Err("OP/ED detection cancelled".to_string());
            }
            if ctx.segment_skip_in_match_pass(ep.id, kind)? {
                done += 1;
                continue;
            }

            if !(ctx.rematch_matched && ctx.segment_is_matched(ep.id, kind)?) {
                ctx.upsert_segment_status(
                    ep.id,
                    kind,
                    OpEdSegmentStatus::Analyzing,
                    None,
                    None,
                    None,
                    Some(template_id),
                    "optimistic",
                    None,
                    None,
                )?;
            }

            let duration = ctx.episode_duration(ep)?;
            let extract_len = full_episode_extract_len(duration);
            let (candidate_key, candidate_fp) =
                ensure_episode_fingerprint(&ep.path, 0.0, extract_len)?;

            let optimistic = ctx.optimistic_search_range(kind, duration);
            let full = ctx.full_search_range(duration);
            let (search_pass, matched) = match_episode_against_template(
                &template_fp,
                &candidate_fp,
                kind,
                optimistic,
                full,
            )
            .map(|(pass, m)| (pass, Some(m)))
            .unwrap_or(("full", None));

            done += 1;
            tick(&format!(
                "{} block {} match: episode {}/{}",
                kind.display_name(),
                block_index + 1,
                done,
                total
            ));

            if let Some(m) = matched {
                fail_streak = 0;
                fail_streak_start_id = None;
                record_episode_match_result(
                    ctx,
                    ep,
                    kind,
                    template_id,
                    &candidate_key,
                    Some(m),
                    search_pass,
                )?;
            } else {
                if fail_streak == 0 {
                    fail_streak_start_id = Some(ep.id);
                }
                fail_streak += 1;
                record_episode_match_result(
                    ctx,
                    ep,
                    kind,
                    template_id,
                    &candidate_key,
                    None,
                    search_pass,
                )?;
                if fail_streak >= FULL_PASS_FAIL_STREAK_FOR_NO_OP_ED
                    && episodes.len() >= MIN_EPISODES_FOR_NO_OP_ED
                {
                    if let Some(first_fail_id) = fail_streak_start_id {
                        bridge_episode_id = episodes
                            .iter()
                            .position(|e| e.id == first_fail_id)
                            .and_then(|pos| pos.checked_sub(1))
                            .map(|pos| episodes[pos].id);
                    }
                    block_transition = true;
                    break;
                }
            }
        }

        if block_transition {
            let still_unmatched = episodes
                .iter()
                .filter(|ep| {
                    ctx.segment_is_matched(ep.id, kind)
                        .map(|m| !m)
                        .unwrap_or(true)
                })
                .count();
            if still_unmatched >= 2 && (block_index + 1) < max_blocks as i32 {
                block_index += 1;
                continue;
            }
        }
        break;
    }

    Ok(())
}

pub fn run_op_ed_detect_job(
    db: &AppDatabase,
    anime_id: i64,
    episode_ids: &[i64],
    options: OpEdDetectJobOptions,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> Result<(), String> {
    let episodes = db.with_conn(|conn| load_episodes_for_detect(conn, anime_id, episode_ids))?;
    let total_steps = op_ed_detect_total_steps(episodes.len());
    let mut step = 1u32;
    let mut tick = |label: &str| {
        on_step(step, total_steps, label);
        step += 1;
    };

    if options.init_anime_state {
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE anime SET no_op_ed = 0, op_ed_analysis_version = ?2 WHERE id = ?1",
                params![anime_id, ANALYSIS_VERSION],
            )
            .map_err(|e| e.to_string())?;
            ensure_pending_segments_for_anime(conn, anime_id)
        })?;
    }

    if options.demote_matched_for_blocks {
        demote_matched_segments_for_rematch(db, episode_ids)?;
    }

    let ctx = DetectContext {
        db,
        anime_id,
        episodes: &episodes,
        continue_templates: options.continue_templates,
        rematch_matched: options.rematch_matched,
        cancel,
        on_progress: &on_step,
    };

    tick("Starting OP/ED detection");
    run_kind_detection(&ctx, SegmentKind::Op, &mut step, total_steps)?;

    if cancel.load(Ordering::Relaxed) {
        return Err("OP/ED detection cancelled".to_string());
    }

    run_kind_detection(&ctx, SegmentKind::Ed, &mut step, total_steps)?;

    if options.mark_analyzed {
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE anime SET op_ed_analyzed_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![anime_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })?;
    }

    on_step(total_steps, total_steps, "Done");
    Ok(())
}

/// Clears OP/ED templates and per-episode segment rows for a title. Chromaprint cache
/// files under `data/op-ed/fingerprints/` are kept so re-detection can reuse them.
pub fn reset_anime_op_ed_analysis(conn: &Connection, anime_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM episode_op_ed_segments
         WHERE episode_id IN (SELECT id FROM episodes WHERE anime_id = ?1)",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM op_ed_templates WHERE anime_id = ?1",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE anime SET no_op_ed = 0, op_ed_analyzed_at = NULL, op_ed_analysis_version = 0
         WHERE id = ?1",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_skip_op_ed(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SKIP_OP_ED_SETTING_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(value.as_deref(), Some("1" | "true" | "yes")))
}

pub fn write_skip_op_ed(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SKIP_OP_ED_SETTING_KEY, if enabled { "1" } else { "0" }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_auto_op_ed_detect(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![AUTO_OP_ED_DETECT_SETTING_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(value.as_deref(), Some("1" | "true" | "yes")))
}

pub fn write_auto_op_ed_detect(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![AUTO_OP_ED_DETECT_SETTING_KEY, if enabled { "1" } else { "0" }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_dont_skip_first_episode_op_ed(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![DONT_SKIP_FIRST_EPISODE_OP_ED_SETTING_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(value.as_deref(), Some("1" | "true" | "yes")))
}

pub fn write_dont_skip_first_episode_op_ed(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![
            DONT_SKIP_FIRST_EPISODE_OP_ED_SETTING_KEY,
            if enabled { "1" } else { "0" }
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_anime_op_ed_summary(
    conn: &Connection,
    anime_id: i64,
) -> Result<AnimeOpEdAnalysisSummary, String> {
    let (no_op_ed, version, analyzed_at): (i64, i32, Option<String>) = conn
        .query_row(
            "SELECT no_op_ed, op_ed_analysis_version, op_ed_analyzed_at FROM anime WHERE id = ?1",
            params![anime_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    let episode_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes WHERE anime_id = ?1 AND missing = 0",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let count_status = |kind: &str, status: &str| -> Result<i64, String> {
        conn.query_row(
            "SELECT COUNT(*) FROM episode_op_ed_segments s
             JOIN episodes e ON e.id = s.episode_id
             WHERE e.anime_id = ?1 AND e.missing = 0 AND s.kind = ?2 AND s.status = ?3",
            params![anime_id, kind, status],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    };

    let op_matched = count_status("op", "matched")?;
    let op_pending = episode_count - op_matched;
    let ed_matched = count_status("ed", "matched")?;
    let ed_pending = episode_count - ed_matched;

    let templates_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM op_ed_templates WHERE anime_id = ?1",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(AnimeOpEdAnalysisSummary {
        anime_id,
        no_op_ed: no_op_ed != 0,
        analysis_version: version,
        analyzed_at,
        episode_count,
        op_matched,
        op_pending,
        ed_matched,
        ed_pending,
        templates_count,
    })
}

pub fn load_episode_op_ed_segments(
    conn: &Connection,
    episode_id: i64,
) -> Result<Vec<OpEdSegmentInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, status, start_sec, end_sec, confidence, search_pass, error_text
             FROM episode_op_ed_segments WHERE episode_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![episode_id], |row| {
            Ok(OpEdSegmentInfo {
                kind: row.get(0)?,
                status: row.get(1)?,
                start_sec: row.get(2)?,
                end_sec: row.get(3)?,
                confidence: row.get(4)?,
                search_pass: row.get(5)?,
                error_text: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn remove_op_ed_cache_for_anime(anime_id: i64, conn: &Connection) -> Result<u64, String> {
    reset_anime_op_ed_analysis(conn, anime_id)?;
    Ok(0)
}

pub fn delete_unreferenced_op_ed_fingerprints(
    referenced_keys: &HashSet<String>,
) -> Result<(usize, u64), String> {
    let cache_dir = fingerprint_cache_dir()?;
    if !cache_dir.is_dir() {
        return Ok((0, 0));
    }
    let mut removed = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(&cache_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("fp") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if referenced_keys.contains(stem) {
            continue;
        }
        bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        removed += 1;
    }
    Ok((removed, bytes))
}

pub fn list_referenced_op_ed_fingerprint_keys(
    conn: &Connection,
) -> Result<HashSet<String>, String> {
    let mut keys = HashSet::new();
    let mut stmt = conn
        .prepare(
            "SELECT fingerprint_cache_key FROM episode_op_ed_segments
             WHERE fingerprint_cache_key IS NOT NULL
             UNION
             SELECT fingerprint_cache_key FROM op_ed_templates",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        if let Ok(k) = row {
            keys.insert(k);
        }
    }
    Ok(keys)
}

pub fn op_ed_cache_directory_size() -> Result<u64, String> {
    let dir = portable_data_dir()?.join(OP_ED_DATA_DIR);
    directory_size(&dir)
}

pub fn manual_op_ed_rematch_job_identity(anime_id: i64) -> String {
    format!("{MANUAL_REMATCH_JOB_NAME}:{anime_id}")
}

fn clamp_manual_template_duration(duration_sec: f64) -> f64 {
    duration_sec.clamp(MANUAL_TEMPLATE_MIN_SEC, MANUAL_TEMPLATE_MAX_SEC)
}

fn episode_display_label(
    conn: &Connection,
    episode_id: i64,
    tracker_offset: i64,
) -> Result<String, String> {
    let episode_number: Option<f64> = conn
        .query_row(
            "SELECT episode_number FROM episodes WHERE id = ?1",
            params![episode_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(match episode_number {
        Some(n) => format!("Episode {}", (n.floor() as i64) - tracker_offset),
        None => "Episode ?".to_string(),
    })
}

pub fn has_manual_templates(conn: &Connection, anime_id: i64) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM op_ed_templates WHERE anime_id = ?1 AND source = 'manual'",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

pub fn count_manual_templates(conn: &Connection, anime_id: i64) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM op_ed_templates WHERE anime_id = ?1 AND source = 'manual'",
        params![anime_id],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

pub fn clear_episode_op_ed_segments_for_anime(conn: &Connection, anime_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM episode_op_ed_segments
         WHERE episode_id IN (SELECT id FROM episodes WHERE anime_id = ?1)",
        params![anime_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn load_manual_templates(conn: &Connection, anime_id: i64) -> Result<Vec<ManualTemplateRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, duration_sec, fingerprint_cache_key
             FROM op_ed_templates
             WHERE anime_id = ?1 AND source = 'manual'
             ORDER BY kind, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| {
            let kind_str: String = row.get(1)?;
            Ok(ManualTemplateRow {
                id: row.get(0)?,
                kind: SegmentKind::parse(&kind_str).unwrap_or(SegmentKind::Op),
                duration_sec: row.get(2)?,
                fingerprint_cache_key: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn list_manual_op_ed_templates(
    conn: &Connection,
    anime_id: i64,
) -> Result<Vec<ManualOpEdTemplate>, String> {
    let tracker_offset: i64 = conn
        .query_row(
            "SELECT tracker_offset FROM anime WHERE id = ?1",
            params![anime_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, kind, start_sec, duration_sec, source_episode_ids
             FROM op_ed_templates
             WHERE anime_id = ?1 AND source = 'manual'
             ORDER BY kind, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    let mut kind_counters: std::collections::HashMap<String, i32> =
        std::collections::HashMap::new();
    for row in rows {
        let (id, kind, start_sec, duration_sec, source_json) = row.map_err(|e| e.to_string())?;
        let kind_index = {
            let counter = kind_counters.entry(kind.clone()).or_insert(0);
            *counter += 1;
            *counter
        };
        let source_episode_id = source_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<i64>>(json).ok())
            .and_then(|ids| ids.first().copied())
            .unwrap_or(0);
        let source_episode_label = if source_episode_id > 0 {
            episode_display_label(conn, source_episode_id, tracker_offset)?
        } else {
            "Episode ?".to_string()
        };
        out.push(ManualOpEdTemplate {
            id,
            kind,
            kind_index,
            start_sec,
            duration_sec,
            source_episode_id,
            source_episode_label,
        });
    }
    Ok(out)
}

fn save_manual_template_fingerprint(
    episode_path: &str,
    start_sec: f64,
    duration_sec: f64,
) -> Result<(Fingerprint, String), String> {
    let duration_sec = clamp_manual_template_duration(duration_sec);
    let (key, fp) = ensure_episode_fingerprint(episode_path, start_sec, duration_sec)?;
    Ok((fp, key))
}

pub fn save_manual_op_ed_template(
    conn: &Connection,
    anime_id: i64,
    kind: &str,
    episode_id: i64,
    start_sec: f64,
    duration_sec: f64,
) -> Result<i64, String> {
    let segment_kind =
        SegmentKind::parse(kind).ok_or_else(|| format!("invalid template kind: {kind}"))?;
    let duration_sec = clamp_manual_template_duration(duration_sec);
    let ep: EpisodeRow = conn
        .query_row(
            "SELECT id, path, duration_seconds FROM episodes
             WHERE id = ?1 AND anime_id = ?2 AND missing = 0",
            params![episode_id, anime_id],
            |row| {
                Ok(EpisodeRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    duration_seconds: row.get(2)?,
                })
            },
        )
        .map_err(|e| format!("episode not found: {e}"))?;
    let (_, fp_key) = save_manual_template_fingerprint(&ep.path, start_sec, duration_sec)?;
    insert_template(
        conn,
        anime_id,
        segment_kind,
        0,
        start_sec,
        duration_sec,
        1.0,
        &fp_key,
        &[episode_id],
        "manual",
    )
}

pub fn update_manual_op_ed_template(
    conn: &Connection,
    template_id: i64,
    start_sec: f64,
    duration_sec: f64,
) -> Result<(), String> {
    let (anime_id, kind_str, source_json): (i64, String, Option<String>) = conn
        .query_row(
            "SELECT anime_id, kind, source_episode_ids FROM op_ed_templates
             WHERE id = ?1 AND source = 'manual'",
            params![template_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| format!("manual template not found: {e}"))?;
    let episode_id = source_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<i64>>(json).ok())
        .and_then(|ids| ids.first().copied())
        .ok_or("manual template has no source episode")?;
    let duration_sec = clamp_manual_template_duration(duration_sec);
    let ep: EpisodeRow = conn
        .query_row(
            "SELECT id, path, duration_seconds FROM episodes WHERE id = ?1 AND anime_id = ?2",
            params![episode_id, anime_id],
            |row| {
                Ok(EpisodeRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    duration_seconds: row.get(2)?,
                })
            },
        )
        .map_err(|e| format!("source episode not found: {e}"))?;
    let (_, fp_key) = save_manual_template_fingerprint(&ep.path, start_sec, duration_sec)?;
    conn.execute(
        "UPDATE op_ed_templates
         SET start_sec = ?2, duration_sec = ?3, confidence = 1.0, fingerprint_cache_key = ?4
         WHERE id = ?1",
        params![template_id, start_sec, duration_sec, fp_key],
    )
    .map_err(|e| e.to_string())?;
    let _ = kind_str;
    Ok(())
}

pub fn delete_manual_op_ed_template(conn: &Connection, template_id: i64) -> Result<(), String> {
    let changed = conn
        .execute(
            "DELETE FROM op_ed_templates WHERE id = ?1 AND source = 'manual'",
            params![template_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("manual template not found".to_string());
    }
    Ok(())
}

fn template_match_from_offset_variable(
    duration_sec: f64,
    offset_frames: usize,
    quality: MatchQuality,
) -> TemplateMatch {
    let start_sec = seconds_for_frames(offset_frames);
    TemplateMatch {
        start_sec,
        end_sec: start_sec + duration_sec,
        confidence: quality.average,
    }
}

fn find_best_match_variable_in_range(
    template: &Fingerprint,
    candidate: &Fingerprint,
    template_duration_sec: f64,
    search_start_frame: usize,
    search_end_frame: usize,
) -> Option<TemplateMatch> {
    let (offset, quality) = find_best_offset_and_quality(
        template,
        candidate,
        search_start_frame,
        search_end_frame,
    )?;
    if !match_quality_is_accepted(quality) {
        return None;
    }
    Some(template_match_from_offset_variable(
        template_duration_sec,
        offset,
        quality,
    ))
}

/// Band-hinted full-episode search for a variable-length manual template.
fn match_template_variable_duration(
    template_fp: &Fingerprint,
    candidate_fp: &Fingerprint,
    kind: SegmentKind,
    template_duration_sec: f64,
    episode_duration: f64,
) -> Option<TemplateMatch> {
    let optimistic = kind_optimistic_search_range(kind, episode_duration);
    let full_end = frames_for_seconds(episode_duration);
    if let Some(matched) = find_best_match_variable_in_range(
        template_fp,
        candidate_fp,
        template_duration_sec,
        optimistic.0,
        optimistic.1,
    ) {
        return Some(matched);
    }
    let before = (0, optimistic.0.saturating_sub(1));
    if before.0 <= before.1 {
        if let Some(matched) = find_best_match_variable_in_range(
            template_fp,
            candidate_fp,
            template_duration_sec,
            before.0,
            before.1,
        ) {
            return Some(matched);
        }
    }
    let after = (optimistic.1.saturating_add(1), full_end);
    if after.0 <= after.1 {
        return find_best_match_variable_in_range(
            template_fp,
            candidate_fp,
            template_duration_sec,
            after.0,
            after.1,
        );
    }
    None
}

pub fn run_manual_op_ed_rematch(
    db: &AppDatabase,
    anime_id: i64,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> Result<(), String> {
    let episodes = db.with_conn(|conn| load_episodes(conn, anime_id))?;
    let templates = db.with_conn(|conn| load_manual_templates(conn, anime_id))?;
    if templates.is_empty() {
        return Err("no manual templates to rematch".to_string());
    }

    db.with_conn(|conn| ensure_pending_segments_for_anime(conn, anime_id))?;

    let total_steps = (episodes.len() as u32).saturating_mul(2).max(1);
    let mut step = 1u32;

    for ep in &episodes {
        if cancel.load(Ordering::Relaxed) {
            return Err("Manual OP/ED rematch cancelled".to_string());
        }
        let duration = if ep.duration_seconds > 0.0 {
            ep.duration_seconds
        } else {
            let path = normalized_video_path(&ep.path)?;
            probe_duration(&path)?
        };
        let extract_len = full_episode_extract_len(duration);
        let (candidate_key, candidate_fp) =
            ensure_episode_fingerprint(&ep.path, 0.0, extract_len)?;

        for kind in [SegmentKind::Op, SegmentKind::Ed] {
            if cancel.load(Ordering::Relaxed) {
                return Err("Manual OP/ED rematch cancelled".to_string());
            }
            on_step(
                step,
                total_steps,
                &format!("{} {} rematch", kind.display_name(), ep.id),
            );
            step = step.saturating_add(1);

            let kind_templates: Vec<&ManualTemplateRow> = templates
                .iter()
                .filter(|t| t.kind == kind)
                .collect();
            if kind_templates.is_empty() {
                db.with_conn(|conn| {
                    upsert_segment_status_conn(
                        conn,
                        ep.id,
                        kind,
                        OpEdSegmentStatus::NotFound,
                        None,
                        None,
                        None,
                        None,
                        "manual",
                        None,
                        None,
                    )
                })?;
                continue;
            }

            let mut best: Option<(i64, TemplateMatch)> = None;
            for template in kind_templates {
                let Some(template_fp) = load_fingerprint(&template.fingerprint_cache_key)? else {
                    continue;
                };
                if let Some(matched) = match_template_variable_duration(
                    &template_fp,
                    &candidate_fp,
                    kind,
                    template.duration_sec,
                    duration,
                ) {
                    if best
                        .as_ref()
                        .is_none_or(|(_, prev)| matched.confidence > prev.confidence)
                    {
                        best = Some((template.id, matched));
                    }
                }
            }

            if let Some((template_id, matched)) = best {
                db.with_conn(|conn| {
                    upsert_segment_status_conn(
                        conn,
                        ep.id,
                        kind,
                        OpEdSegmentStatus::Matched,
                        Some(matched.start_sec),
                        Some(matched.end_sec),
                        Some(f64::from(matched.confidence)),
                        Some(template_id),
                        "manual",
                        Some(&candidate_key),
                        None,
                    )
                })?;
            } else {
                db.with_conn(|conn| {
                    upsert_segment_status_conn(
                        conn,
                        ep.id,
                        kind,
                        OpEdSegmentStatus::NotFound,
                        None,
                        None,
                        None,
                        None,
                        "manual",
                        None,
                        None,
                    )
                })?;
            }
        }
    }

    on_step(total_steps, total_steps, "Done");
    Ok(())
}

pub fn auto_rematch_job_options() -> OpEdDetectJobOptions {
    OpEdDetectJobOptions {
        continue_templates: true,
        init_anime_state: false,
        mark_analyzed: false,
        rematch_matched: true,
        demote_matched_for_blocks: false,
    }
}

#[tauri::command]
pub fn list_manual_op_ed_templates_cmd(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Vec<ManualOpEdTemplate>, String> {
    db.with_conn(|conn| list_manual_op_ed_templates(conn, anime_id))
}

#[tauri::command]
pub fn count_manual_op_ed_templates_cmd(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<i64, String> {
    db.with_conn(|conn| count_manual_templates(conn, anime_id))
}

#[tauri::command]
pub fn save_manual_op_ed_template_cmd(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    kind: String,
    episode_id: i64,
    start_sec: f64,
    duration_sec: f64,
) -> Result<i64, String> {
    db.with_conn(|conn| {
        save_manual_op_ed_template(conn, anime_id, &kind, episode_id, start_sec, duration_sec)
    })
}

#[tauri::command]
pub fn update_manual_op_ed_template_cmd(
    db: State<'_, AppDatabase>,
    template_id: i64,
    start_sec: f64,
    duration_sec: f64,
) -> Result<(), String> {
    db.with_conn(|conn| update_manual_op_ed_template(conn, template_id, start_sec, duration_sec))
}

#[tauri::command]
pub fn delete_manual_op_ed_template_cmd(
    db: State<'_, AppDatabase>,
    template_id: i64,
) -> Result<(), String> {
    db.with_conn(|conn| delete_manual_op_ed_template(conn, template_id))
}

#[tauri::command]
pub fn probe_video_fps_cmd(path: String) -> Result<f64, String> {
    let path_buf = normalized_video_path(&path)?;
    probe_video_fps(&path_buf)
}

#[tauri::command]
pub fn reset_anime_op_ed_analysis_cmd(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<(), String> {
    db.with_conn(|conn| reset_anime_op_ed_analysis(conn, anime_id))
}

#[tauri::command]
pub fn get_anime_op_ed_summary_cmd(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<AnimeOpEdAnalysisSummary, String> {
    db.with_conn(|conn| get_anime_op_ed_summary(conn, anime_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint_from_values(values: &[i32]) -> Fingerprint {
        Fingerprint {
            values: values.to_vec(),
        }
    }

    fn value_with_bit_diffs(count: u32) -> i32 {
        if count == 0 {
            0
        } else {
            ((1_u32 << count) - 1) as i32
        }
    }

    #[test]
    fn fingerprint_similarity_identical_items_score_high() {
        let fp = fingerprint_from_values(&[12345, -98765]);
        let score = fingerprint_similarity(&fp, 0, &fp, 0);
        assert!(score > 0.99, "score={score}");
    }

    #[test]
    fn discovery_window_starts_cover_op_search_band() {
        let starts = discovery_window_starts(0.0, 180.0);
        assert_eq!(starts.len(), 23);
        assert!((starts[0] - 0.0).abs() < f64::EPSILON);
        assert!((starts[1] - 7.5).abs() < f64::EPSILON);
        assert!((starts.last().copied().unwrap() - 165.0).abs() < f64::EPSILON);
    }

    #[test]
    fn discovery_slice_extracts_window_from_full() {
        let values: Vec<i32> = (0..200).collect();
        let full = fingerprint_from_values(&values);
        let slice = slice_fingerprint(&full, 60, SEED_WINDOW_FRAMES).expect("in range");
        assert_eq!(slice.frame_count(), SEED_WINDOW_FRAMES);
        assert_eq!(slice.values[0], 60);
        assert_eq!(slice.values[SEED_WINDOW_FRAMES - 1], (60 + SEED_WINDOW_FRAMES - 1) as i32);
    }

    #[test]
    fn cluster_seed_candidates_requires_cross_episode_agreement() {
        let near = value_with_bit_diffs(2);
        let window = fingerprint_from_values(&[near; SEED_WINDOW_FRAMES]);
        let seeds = vec![
            SeedCandidate {
                start_sec: 0.0,
                episode_id: 1,
                fingerprint: window.clone(),
            },
            SeedCandidate {
                start_sec: 1.0,
                episode_id: 2,
                fingerprint: window.clone(),
            },
            SeedCandidate {
                start_sec: 90.0,
                episode_id: 3,
                fingerprint: fingerprint_from_values(&[value_with_bit_diffs(16); SEED_WINDOW_FRAMES]),
            },
        ];
        let (seed, ids) = cluster_seed_candidates(&seeds).expect("cluster");
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&1) && ids.contains(&2));
        assert!(seed.episode_id == 1 || seed.episode_id == 2);
    }

    #[test]
    fn refine_slide_offset_step_count_covers_plus_minus_eight() {
        let mut count = 0usize;
        let mut offset_sec = -(SEED_REFINE_OFFSET_SEC as f64);
        while offset_sec <= SEED_REFINE_OFFSET_SEC as f64 + f64::EPSILON {
            count += 1;
            offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
        }
        assert_eq!(count, 33);
    }

    #[test]
    fn unison_slide_finds_strong_average_match() {
        let near = value_with_bit_diffs(2);
        let far = value_with_bit_diffs(16);
        let template_frames = 40usize;
        let coarse_start_frame = 10usize;
        let ref_vals: Vec<i32> = (0..200)
            .map(|i| if (coarse_start_frame..coarse_start_frame + template_frames).contains(&i) {
                near
            } else {
                far
            })
            .collect();
        let ref_full = fingerprint_from_values(&ref_vals);
        let coarse_template =
            slice_fingerprint(&ref_full, coarse_start_frame, template_frames).expect("template");

        let mut ep_a = vec![far; 8];
        ep_a.extend(vec![near; 80]);
        let full_a = fingerprint_from_values(&ep_a);
        let anchor_a =
            find_anchor_offset(&coarse_template, &full_a, 0, 20).expect("anchor a");

        let mut ep_b = vec![far; 13];
        ep_b.extend(vec![near; 80]);
        let full_b = fingerprint_from_values(&ep_b);
        let anchor_b =
            find_anchor_offset(&coarse_template, &full_b, 0, 25).expect("anchor b");

        let coarse_start_sec = seconds_for_frames(coarse_start_frame);
        let mut best_avg = f32::NEG_INFINITY;
        let mut offset_sec = -(SEED_REFINE_OFFSET_SEC as f64);
        while offset_sec <= SEED_REFINE_OFFSET_SEC as f64 + f64::EPSILON {
            let shifted_start_sec = coarse_start_sec + offset_sec;
            if shifted_start_sec < 0.0 {
                offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
                continue;
            }
            let shifted_start_frame = frames_for_seconds(shifted_start_sec);
            let Some(shifted_template) =
                slice_fingerprint(&ref_full, shifted_start_frame, template_frames)
            else {
                offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
                continue;
            };
            let match_a_sec = seconds_for_frames(anchor_a) + offset_sec;
            let match_b_sec = seconds_for_frames(anchor_b) + offset_sec;
            if match_a_sec < 0.0 || match_b_sec < 0.0 {
                offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
                continue;
            }
            let match_a = frames_for_seconds(match_a_sec);
            let match_b = frames_for_seconds(match_b_sec);
            if match_a + template_frames > full_a.frame_count()
                || match_b + template_frames > full_b.frame_count()
            {
                offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
                continue;
            }
            let score_a = sliding_match_score(&shifted_template, &full_a, match_a);
            let score_b = sliding_match_score(&shifted_template, &full_b, match_b);
            let avg = (score_a + score_b) / 2.0;
            if avg > best_avg {
                best_avg = avg;
            }
            offset_sec += SEED_REFINE_OFFSET_STEP_SEC;
        }
        assert!(best_avg.is_finite());
        assert!(best_avg > 0.9);
    }

    #[test]
    fn refinement_beats_coarse_requires_minimum_and_improvement() {
        assert!(refinement_beats_coarse(5, 5));
        assert!(refinement_beats_coarse(5, 6));
        assert!(!refinement_beats_coarse(5, 4));
        assert!(!refinement_beats_coarse(0, 2));
        assert!(!refinement_beats_coarse(3, 2));
    }

    #[test]
    fn find_anchor_offset_picks_best_sliding_match() {
        let near = value_with_bit_diffs(2);
        let far = value_with_bit_diffs(16);
        let template = fingerprint_from_values(&[near; 40]);
        let mut candidate_vals = vec![far; 8];
        candidate_vals.extend(vec![near; 40]);
        let candidate = fingerprint_from_values(&candidate_vals);
        let offset = find_anchor_offset(&template, &candidate, 0, 12).expect("anchor");
        assert_eq!(offset, 8);
    }

    #[test]
    fn insufficient_episode_count_skips_seed() {
        let eps = vec![EpisodeRow {
            id: 1,
            path: String::new(),
            duration_seconds: 1200.0,
        }];
        let cancel = AtomicBool::new(false);
        let result = discover_repeated_seed(&eps, SegmentKind::Op, &cancel, &mut |_| {}).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn match_requires_consistent_strong_frames() {
        let template = fingerprint_from_values(&[0; 8]);
        let weak = value_with_bit_diffs(12);
        let candidate = fingerprint_from_values(&[0, weak, 0, weak, 0, weak, 0, 0]);

        assert!(sliding_match_score(&template, &candidate, 0) >= MATCH_AVERAGE_THRESHOLD);
        assert!(find_best_match_in_candidate(&template, &candidate, 0, 0).is_none());
    }

    #[test]
    fn lead_trim_fallback_matches_when_template_lead_frames_are_weak() {
        let near = value_with_bit_diffs(2);
        let far = value_with_bit_diffs(16);
        let lead_frames = frames_for_seconds(MATCH_FALLBACK_LEAD_TRIM_SEC);
        let body_len = frames_for_seconds(SEGMENT_DURATION_SEC) + 40;
        let mut template_vals = vec![far; lead_frames];
        template_vals.extend(vec![near; body_len]);
        let template = fingerprint_from_values(&template_vals);

        let mut candidate_vals = vec![far; 8];
        candidate_vals.extend(vec![near; body_len + 8]);
        let candidate = fingerprint_from_values(&candidate_vals);

        let search_end = candidate.frame_count().saturating_sub(template.frame_count());
        assert!(
            find_best_match_in_candidate(&template, &candidate, 0, search_end).is_none(),
            "strict match should reject weak template lead at offset 0"
        );

        let trimmed_len = template.frame_count().saturating_sub(lead_frames);
        let trimmed = slice_fingerprint(&template, lead_frames, trimmed_len).expect("trim");
        let trimmed_end = candidate
            .frame_count()
            .saturating_sub(trimmed.frame_count());
        assert!(
            find_best_match_in_candidate(&trimmed, &candidate, 0, trimmed_end).is_some(),
            "trimmed template should match aligned candidate body"
        );
    }

    #[test]
    fn match_accepts_consistent_high_quality_segment() {
        let template = fingerprint_from_values(&[0; 8]);
        let near = value_with_bit_diffs(2);
        let far = value_with_bit_diffs(16);
        let candidate =
            fingerprint_from_values(&[far, far, near, near, near, near, near, near, near, near]);

        let matched = find_best_match_in_candidate(&template, &candidate, 0, 2)
            .expect("consistent high-quality segment should match");
        assert!((matched.start_sec - seconds_for_frames(2)).abs() < f64::EPSILON);
        assert!(matched.confidence >= MATCH_AVERAGE_THRESHOLD);
    }

    #[test]
    fn plan_detect_jobs_preview_then_full_for_long_seasons() {
        let ids: Vec<i64> = (1..=20).collect();
        let plans = plan_op_ed_detect_jobs(&ids, false);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].episode_ids.len(), OP_ED_DETECT_BATCH_SIZE);
        assert_eq!(plans[1].episode_ids.len(), 20);
        assert!(!plans[0].options.mark_analyzed);
        assert!(!plans[1].options.rematch_matched);
        assert!(plans[1].options.demote_matched_for_blocks);
        assert!(plans[1].options.mark_analyzed);

        let short = plan_op_ed_detect_jobs(&(1..=8).collect::<Vec<_>>(), false);
        assert_eq!(short.len(), 1);
        assert!(short[0].options.mark_analyzed);

        let rerun = plan_op_ed_detect_jobs(&ids, true);
        assert_eq!(rerun.len(), 1);
        assert_eq!(rerun[0].episode_ids.len(), 20);
        assert!(!rerun[0].options.rematch_matched);
        assert!(!rerun[0].options.demote_matched_for_blocks);
        assert!(!rerun[0].options.init_anime_state);
    }
}
