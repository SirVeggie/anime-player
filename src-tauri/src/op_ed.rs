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
    portable_data_dir, probe_duration,
};

pub const ANALYSIS_VERSION: i32 = 2;
pub const SKIP_OP_ED_SETTING_KEY: &str = "skip_op_ed";
/// Parent folder under portable `data/` for all OP/ED artifacts.
pub const OP_ED_DATA_DIR: &str = "op-ed";
const FINGERPRINTS_SUBDIR: &str = "fingerprints";
const JOB_NAME: &str = "op_ed_detect";

pub const SAMPLE_RATE: u32 = 11025;
const CHROMAPRINT_FRAME_SEC: f64 = 0.1238;
const SEED_WINDOW_SEC: f64 = 15.0;
const SEGMENT_DURATION_SEC: f64 = 90.0;
const OP_SEARCH_SEC: f64 = 180.0;
const ED_TAIL_SEC: f64 = 180.0;
const MATCH_AVERAGE_THRESHOLD: f32 = 0.84;
const MATCH_STRONG_FRAME_THRESHOLD: f32 = 0.84;
const MATCH_MIN_STRONG_FRAME_RATIO: f32 = 0.60;
const MATCH_MIN_LOWER_QUARTILE: f32 = 0.78;
const SEED_MATCH_THRESHOLD: f32 = 0.82;
const MAX_SEED_EPISODES: usize = 10;
const MIN_EPISODES_FOR_NO_OP_ED: usize = 3;
const FULL_PASS_FAIL_STREAK_FOR_NO_OP_ED: usize = 3;

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

fn episode_path_cache_keys(conn: &Connection, anime_id: i64) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM episodes WHERE anime_id = ?1 AND missing = 0")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut keys = HashSet::new();
    for row in rows {
        let path = row.map_err(|e| e.to_string())?;
        let path_buf = normalized_video_path(&path)?;
        keys.insert(cache_key(&path_buf)?);
    }
    Ok(keys)
}

/// Remove cached `.fp` files for this anime's episodes (including keys never stored in SQLite).
fn purge_fingerprint_cache_for_anime(conn: &Connection, anime_id: i64) -> Result<(), String> {
    let path_keys = episode_path_cache_keys(conn, anime_id)?;
    if path_keys.is_empty() {
        return Ok(());
    }
    let cache_dir = fingerprint_cache_dir()?;
    if !cache_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&cache_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("fp") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let belongs_to_anime = path_keys
            .iter()
            .any(|key| stem == key.as_str() || stem.starts_with(&format!("{key}_")));
        if belongs_to_anime {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

pub fn op_ed_job_identity(anime_id: i64) -> String {
    format!("{JOB_NAME}:{anime_id}")
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
    quality.average >= MATCH_AVERAGE_THRESHOLD
        && quality.strong_frame_ratio >= MATCH_MIN_STRONG_FRAME_RATIO
        && quality.lower_quartile >= MATCH_MIN_LOWER_QUARTILE
}

fn frames_for_seconds(sec: f64) -> usize {
    Fingerprint::frames_for_duration(sec)
}

fn seconds_for_frames(frames: usize) -> f64 {
    frames as f64 * CHROMAPRINT_FRAME_SEC
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
    let key = format!(
        "cp{}_{}_{}_{}",
        ANALYSIS_VERSION,
        cache_key(&path_buf)?,
        (start_sec * 1000.0) as i64,
        (duration_sec * 1000.0) as i64
    );
    if let Some(fp) = load_fingerprint(&key)? {
        return Ok((key, fp));
    }
    let fp = extract_fingerprint_from_file(&path_buf, start_sec, duration_sec, &key)?;
    save_fingerprint(&key, &fp)?;
    Ok((key, fp))
}

#[derive(Debug, Clone)]
struct TemplateMatch {
    start_sec: f64,
    end_sec: f64,
    confidence: f32,
}

fn find_best_match_in_candidate(
    template: &Fingerprint,
    candidate: &Fingerprint,
    search_start_frame: usize,
    search_end_frame: usize,
) -> Option<TemplateMatch> {
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
    if !match_quality_is_accepted(quality) {
        return None;
    }
    let start_sec = seconds_for_frames(best_offset);
    let end_sec = start_sec + SEGMENT_DURATION_SEC;
    Some(TemplateMatch {
        start_sec,
        end_sec,
        confidence: quality.average,
    })
}

#[derive(Debug, Clone)]
struct SeedCandidate {
    start_sec: f64,
    episode_id: i64,
    fingerprint: Fingerprint,
}

/// Steps reserved per segment kind: scan each seed episode, compare, then build or bail.
fn discovery_steps_per_kind(episode_count: usize) -> u32 {
    if episode_count < 2 {
        return 0;
    }
    episode_count.min(MAX_SEED_EPISODES) as u32 + 2
}

fn op_ed_detect_total_steps(episode_count: usize) -> u32 {
    if episode_count < 2 {
        // Starting + OP skip + ED skip + Done
        return 4;
    }
    let per_kind = discovery_steps_per_kind(episode_count) + episode_count as u32;
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
        let (start_sec, extract_len) = match kind {
            SegmentKind::Op => (0.0, OP_SEARCH_SEC.min(duration)),
            SegmentKind::Ed => {
                let tail = ED_TAIL_SEC.min(duration);
                ((duration - tail).max(0.0), tail)
            }
        };
        if extract_len < SEED_WINDOW_SEC + 5.0 {
            continue;
        }
        let mut offset = 0.0f64;
        while offset + SEED_WINDOW_SEC <= extract_len {
            if cancel.load(Ordering::Relaxed) {
                return Err("OP/ED detection cancelled".to_string());
            }
            let (_, fp) =
                ensure_episode_fingerprint(&ep.path, start_sec + offset, SEED_WINDOW_SEC)?;
            seeds.push(SeedCandidate {
                start_sec: start_sec + offset,
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

    let Some((seed, episode_ids, count)) = best_cluster else {
        return Ok(None);
    };
    if count < 2 {
        return Ok(None);
    }
    Ok(Some((seed, episode_ids)))
}

fn build_template_fingerprint(
    episode_path: &str,
    start_sec: f64,
    cancel: &AtomicBool,
) -> Result<(Fingerprint, String), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("OP/ED detection cancelled".to_string());
    }
    let (key, fp) = ensure_episode_fingerprint(episode_path, start_sec, SEGMENT_DURATION_SEC)?;
    Ok((fp, key))
}

struct DetectContext<'a> {
    conn: &'a Connection,
    anime_id: i64,
    episodes: &'a [EpisodeRow],
    cancel: &'a AtomicBool,
    on_progress: &'a dyn Fn(u32, u32, &str),
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
        self.conn
            .execute(
                "INSERT INTO episode_op_ed_segments
                    (episode_id, kind, status, start_sec, end_sec, confidence, template_id,
                     search_pass, fingerprint_cache_key, error_text, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, CURRENT_TIMESTAMP)
                 ON CONFLICT(episode_id, kind) DO UPDATE SET
                    status = excluded.status,
                    start_sec = excluded.start_sec,
                    end_sec = excluded.end_sec,
                    confidence = excluded.confidence,
                    template_id = excluded.template_id,
                    search_pass = excluded.search_pass,
                    fingerprint_cache_key = excluded.fingerprint_cache_key,
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

    fn episode_duration(&self, ep: &EpisodeRow) -> Result<f64, String> {
        if ep.duration_seconds > 0.0 {
            return Ok(ep.duration_seconds);
        }
        let path = normalized_video_path(&ep.path)?;
        Ok(probe_duration(&path)?)
    }

    fn optimistic_search_range(&self, kind: SegmentKind, duration: f64) -> (usize, usize) {
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

    fn full_search_range(&self, duration: f64) -> (usize, usize) {
        (0, frames_for_seconds(duration))
    }
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
) -> Result<i64, String> {
    let source_json = serde_json::to_string(source_ids).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO op_ed_templates
            (anime_id, kind, block_index, start_sec, duration_sec, confidence,
             fingerprint_cache_key, source_episode_ids)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            anime_id,
            kind.as_str(),
            block_index,
            start_sec,
            duration_sec,
            confidence,
            fp_key,
            source_json,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn segment_is_terminal(
    conn: &Connection,
    episode_id: i64,
    kind: SegmentKind,
) -> Result<bool, String> {
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
        Some("matched") | Some("not_found") | Some("skipped")
    ))
}

fn run_kind_detection(
    ctx: &DetectContext<'_>,
    kind: SegmentKind,
    block_index: i32,
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

    let seed_result = discover_repeated_seed(episodes, kind, ctx.cancel, &mut |label| tick(label))?;
    let Some((seed, source_ids)) = seed_result else {
        tick(&format!(
            "{} discovery: no repeated segment found",
            kind.display_name()
        ));
        if episodes.len() >= MIN_EPISODES_FOR_NO_OP_ED {
            ctx.conn
                .execute(
                    "UPDATE anime SET no_op_ed = 1 WHERE id = ?1",
                    params![ctx.anime_id],
                )
                .map_err(|e| e.to_string())?;
        }
        for ep in episodes {
            if !segment_is_terminal(ctx.conn, ep.id, kind)? {
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
        return Ok(());
    };

    tick(&format!(
        "{} discovery: building template",
        kind.display_name()
    ));

    let source_path = episodes
        .iter()
        .find(|e| e.id == seed.episode_id)
        .map(|e| e.path.as_str())
        .ok_or("seed episode not found")?;
    let (template_fp, template_key) =
        build_template_fingerprint(source_path, seed.start_sec, ctx.cancel)?;
    let template_id = insert_template(
        ctx.conn,
        ctx.anime_id,
        kind,
        block_index,
        seed.start_sec,
        SEGMENT_DURATION_SEC,
        1.0,
        &template_key,
        &source_ids,
    )?;

    let mut fail_streak = 0usize;
    let mut done = 0u32;
    let total = episodes.len() as u32;

    for ep in episodes {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }
        if segment_is_terminal(ctx.conn, ep.id, kind)? {
            done += 1;
            continue;
        }

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

        let duration = ctx.episode_duration(ep)?;
        let (search_start, search_end) = ctx.optimistic_search_range(kind, duration);
        let (full_start, full_end) = ctx.full_search_range(duration);

        let extract_len = duration.max(SEGMENT_DURATION_SEC + 1.0);
        let (candidate_key, candidate_fp) = ensure_episode_fingerprint(&ep.path, 0.0, extract_len)?;

        let mut matched =
            find_best_match_in_candidate(&template_fp, &candidate_fp, search_start, search_end);
        let mut search_pass = "optimistic";

        if matched.is_none() {
            matched =
                find_best_match_in_candidate(&template_fp, &candidate_fp, full_start, full_end);
            search_pass = "full";
        }

        done += 1;
        tick(&format!(
            "{} match: episode {}/{}",
            kind.display_name(),
            done,
            total
        ));

        if let Some(m) = matched {
            fail_streak = 0;
            ctx.upsert_segment_status(
                ep.id,
                kind,
                OpEdSegmentStatus::Matched,
                Some(m.start_sec),
                Some(m.end_sec),
                Some(f64::from(m.confidence)),
                Some(template_id),
                search_pass,
                Some(&candidate_key),
                None,
            )?;
        } else {
            fail_streak += 1;
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
            if fail_streak >= FULL_PASS_FAIL_STREAK_FOR_NO_OP_ED
                && episodes.len() >= MIN_EPISODES_FOR_NO_OP_ED
            {
                break;
            }
        }
    }

    Ok(())
}

pub fn run_op_ed_detect_job(
    db: &AppDatabase,
    anime_id: i64,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> Result<(), String> {
    db.with_conn(|conn| {
        let episodes = load_episodes(conn, anime_id)?;
        let total_steps = op_ed_detect_total_steps(episodes.len());
        let mut step = 1u32;
        let mut tick = |label: &str| {
            on_step(step, total_steps, label);
            step += 1;
        };

        conn.execute(
            "UPDATE anime SET no_op_ed = 0, op_ed_analysis_version = ?2 WHERE id = ?1",
            params![anime_id, ANALYSIS_VERSION],
        )
        .map_err(|e| e.to_string())?;

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

        let ctx = DetectContext {
            conn,
            anime_id,
            episodes: &episodes,
            cancel,
            on_progress: &on_step,
        };

        tick("Starting OP/ED detection");
        run_kind_detection(&ctx, SegmentKind::Op, 0, &mut step, total_steps)?;

        if cancel.load(Ordering::Relaxed) {
            return Err("OP/ED detection cancelled".to_string());
        }

        run_kind_detection(&ctx, SegmentKind::Ed, 0, &mut step, total_steps)?;

        conn.execute(
            "UPDATE anime SET op_ed_analyzed_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;

        on_step(total_steps, total_steps, "Done");
        Ok(())
    })
}

pub fn reset_anime_op_ed_analysis(conn: &Connection, anime_id: i64) -> Result<(), String> {
    let keys: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT fingerprint_cache_key FROM episode_op_ed_segments
                 WHERE episode_id IN (SELECT id FROM episodes WHERE anime_id = ?1)
                   AND fingerprint_cache_key IS NOT NULL
                 UNION
                 SELECT fingerprint_cache_key FROM op_ed_templates WHERE anime_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![anime_id], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let mut keys = Vec::new();
        for row in rows {
            if let Ok(k) = row {
                keys.push(k);
            }
        }
        keys
    };

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

    for key in keys {
        if let Ok(path) = fingerprint_path(&key) {
            let _ = fs::remove_file(path);
        }
    }
    purge_fingerprint_cache_for_anime(conn, anime_id)?;
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
}
