use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::media_tools::{
    cache_key, ffmpeg_path, hidden_command, normalized_video_path, portable_data_dir, probe_duration,
};

pub use crate::media_tools::normalized_video_path_key;

const SPRITE_DIR: &str = "scrub-sprites";
const THUMB_COLS: u32 = 10;
const THUMB_WIDTH: u32 = 160;
const THUMB_HEIGHT: u32 = 90;
const MIN_THUMBS: u32 = 20;
const MAX_THUMBS: u32 = 120;
const THUMB_INTERVAL_SEC: f64 = 5.0;
const SCRUB_JOB_NAME: &str = "scrub_sprite";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubSpriteReady {
    pub path: String,
    pub data_url: String,
    pub cols: u32,
    pub rows: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub thumb_count: u32,
    pub interval_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScrubSpriteStatus {
    Ready(ScrubSpriteReady),
    Unavailable { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScrubSpriteMeta {
    cols: u32,
    rows: u32,
    thumb_width: u32,
    thumb_height: u32,
    thumb_count: u32,
    interval_sec: f64,
    source_path: String,
}

fn sprite_cache_dir() -> Result<PathBuf, String> {
    let dir = portable_data_dir()?.join(SPRITE_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create {dir:?}: {e}"))?;
    Ok(dir)
}

pub fn scrub_sprite_identity(path: &str) -> Result<String, String> {
    let path_buf = normalized_video_path(path)?;
    let canonical = path_buf.to_string_lossy().to_ascii_lowercase();
    Ok(format!("{SCRUB_JOB_NAME}:{canonical}"))
}

pub fn scrub_sprite_is_cached(path: &str) -> Result<bool, String> {
    let path_buf = normalized_video_path(path)?;
    let key = cache_key(&path_buf)?;
    let cache_dir = sprite_cache_dir()?;
    let jpg_path = cache_dir.join(format!("{key}.jpg"));
    let json_path = cache_dir.join(format!("{key}.json"));
    Ok(jpg_path.is_file() && json_path.is_file())
}

fn normalize_path_for_match(path: &str) -> String {
    normalized_video_path_key(path)
}

fn remove_sprite_cache_files(key: &str) -> Result<u64, String> {
    let cache_dir = sprite_cache_dir()?;
    let mut bytes = 0_u64;
    for ext in ["jpg", "json", "tmp.jpg"] {
        let file_path = cache_dir.join(format!("{key}.{ext}"));
        if !file_path.is_file() {
            continue;
        }
        bytes += fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
        fs::remove_file(&file_path)
            .map_err(|e| format!("failed to remove {}: {e}", file_path.display()))?;
    }
    Ok(bytes)
}

fn remove_sprite_cache_by_source_path(path: &str) -> Result<u64, String> {
    let target = normalize_path_for_match(path);
    let cache_dir = sprite_cache_dir()?;
    if !cache_dir.is_dir() {
        return Ok(0);
    }

    let mut bytes = 0_u64;
    for entry in fs::read_dir(&cache_dir).map_err(|e| format!("failed to read {cache_dir:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let json_path = entry.path();
        if json_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(meta) = fs::read_to_string(&json_path)
            .ok()
            .and_then(|content| serde_json::from_str::<ScrubSpriteMeta>(&content).ok())
        else {
            continue;
        };
        if normalize_path_for_match(&meta.source_path) != target {
            continue;
        }
        let Some(key) = json_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        bytes += remove_sprite_cache_files(key)?;
    }
    Ok(bytes)
}

/// Deletes scrub sprite cache entries whose `source_path` is not in `referenced_paths`.
pub fn delete_unreferenced_scrub_sprites(
    referenced_paths: &HashSet<String>,
) -> Result<(usize, u64), String> {
    let cache_dir = sprite_cache_dir()?;
    if !cache_dir.is_dir() {
        return Ok((0, 0));
    }

    let mut removed_count = 0_usize;
    let mut removed_bytes = 0_u64;

    for entry in fs::read_dir(&cache_dir).map_err(|e| format!("failed to read {cache_dir:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_path = entry.path();
        if file_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(meta) = fs::read_to_string(&file_path)
            .ok()
            .and_then(|content| serde_json::from_str::<ScrubSpriteMeta>(&content).ok())
        else {
            continue;
        };
        if referenced_paths.contains(&normalize_path_for_match(&meta.source_path)) {
            continue;
        }
        let Some(key) = file_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        removed_bytes += remove_sprite_cache_files(key)?;
        removed_count += 1;
    }

    for entry in fs::read_dir(&cache_dir).map_err(|e| format!("failed to read {cache_dir:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_path = entry.path();
        let Some(ext) = file_path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "jpg" && ext != "tmp" {
            continue;
        }
        let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let key = stem.strip_suffix(".tmp").unwrap_or(stem);
        let json_path = cache_dir.join(format!("{key}.json"));
        if json_path.is_file() {
            continue;
        }
        if file_path.is_file() {
            removed_bytes += fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
            let _ = fs::remove_file(&file_path);
        }
    }

    Ok((removed_count, removed_bytes))
}

/// Deletes scrub sprite cache entries for the given episode paths.
pub fn delete_scrub_sprites_for_paths(paths: &[String]) -> Result<(usize, u64), String> {
    let mut removed_count = 0_usize;
    let mut removed_bytes = 0_u64;
    for path in paths {
        let bytes = remove_scrub_sprite_cache(path)?;
        if bytes > 0 {
            removed_count += 1;
            removed_bytes += bytes;
        }
    }
    Ok((removed_count, removed_bytes))
}

/// Removes cached scrub sprite files for a video path. Best-effort when the file is gone.
pub fn remove_scrub_sprite_cache(path: &str) -> Result<u64, String> {
    let mut bytes = 0_u64;
    let path_buf = PathBuf::from(path);
    if path_buf.is_file() {
        if let Ok(canonical) = path_buf.canonicalize() {
            if let Ok(key) = cache_key(&canonical) {
                bytes += remove_sprite_cache_files(&key)?;
            }
            let canonical_str = canonical.to_string_lossy();
            bytes += remove_sprite_cache_by_source_path(&canonical_str)?;
        }
    }
    bytes += remove_sprite_cache_by_source_path(path)?;
    Ok(bytes)
}

fn thumb_count_for_duration(duration: f64) -> u32 {
    if duration <= 0.0 {
        return MIN_THUMBS;
    }
    let by_interval = (duration / THUMB_INTERVAL_SEC).ceil() as u32;
    by_interval.clamp(MIN_THUMBS, MAX_THUMBS)
}

fn read_data_url(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        BASE64.encode(bytes)
    ))
}

pub fn load_cached_ready(path: &str, key: &str) -> Result<Option<ScrubSpriteReady>, String> {
    let cache_dir = sprite_cache_dir()?;
    let jpg_path = cache_dir.join(format!("{key}.jpg"));
    let json_path = cache_dir.join(format!("{key}.json"));
    if !jpg_path.is_file() || !json_path.is_file() {
        return Ok(None);
    }
    let meta: ScrubSpriteMeta = serde_json::from_str(
        &fs::read_to_string(&json_path)
            .map_err(|e| format!("failed to read {}: {e}", json_path.display()))?,
    )
    .map_err(|e| format!("invalid scrub sprite metadata: {e}"))?;
    let data_url = read_data_url(&jpg_path)?;
    Ok(Some(ScrubSpriteReady {
        path: path.to_string(),
        data_url,
        cols: meta.cols,
        rows: meta.rows,
        thumb_width: meta.thumb_width,
        thumb_height: meta.thumb_height,
        thumb_count: meta.thumb_count,
        interval_sec: meta.interval_sec,
    }))
}

pub fn get_scrub_sprite_if_ready(path: &str) -> Result<Option<ScrubSpriteReady>, String> {
    let path_buf = normalized_video_path(path)?;
    let display_path = path_buf.to_string_lossy().into_owned();
    let key = cache_key(&path_buf)?;
    let mut ready = load_cached_ready(&display_path, &key)?;
    if let Some(sprite) = ready.as_mut() {
        sprite.path = display_path;
    }
    Ok(ready)
}

fn generate_sprite(
    path: &Path,
    key: &str,
    cancel: &AtomicBool,
) -> Result<ScrubSpriteReady, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("scrub sprite generation cancelled".to_string());
    }

    let duration = probe_duration(path)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("scrub sprite generation cancelled".to_string());
    }

    let thumb_count = thumb_count_for_duration(duration);
    let cols = THUMB_COLS;
    let rows = thumb_count.div_ceil(cols);
    let interval_sec = duration / f64::from(thumb_count);

    let cache_dir = sprite_cache_dir()?;
    let jpg_path = cache_dir.join(format!("{key}.jpg"));
    let json_path = cache_dir.join(format!("{key}.json"));
    let tmp_jpg = cache_dir.join(format!("{key}.tmp.jpg"));

    let interval_str = format!("{interval_sec:.6}");
    let vf = format!(
        "fps=1/{interval_str},scale={THUMB_WIDTH}:{THUMB_HEIGHT}:force_original_aspect_ratio=decrease,pad={THUMB_WIDTH}:{THUMB_HEIGHT}:(ow-iw)/2:(oh-ih)/2,tile={cols}x{rows}"
    );

    let ffmpeg = ffmpeg_path()?;
    let output = hidden_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-hwaccel", "auto"])
        .args(["-skip_frame", "nokey"])
        .arg("-i")
        .arg(path)
        .args(["-an", "-vf", &vf, "-vsync", "0", "-frames:v", "1", "-q:v", "5"])
        .arg(&tmp_jpg)
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;

    if cancel.load(Ordering::Relaxed) {
        let _ = fs::remove_file(&tmp_jpg);
        return Err("scrub sprite generation cancelled".to_string());
    }

    if !output.status.success() {
        let _ = fs::remove_file(&tmp_jpg);
        return Err(format!(
            "ffmpeg sprite generation failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    if !tmp_jpg.is_file() {
        return Err(format!(
            "ffmpeg did not produce scrub sprite for {}",
            path.display()
        ));
    }

    fs::rename(&tmp_jpg, &jpg_path).map_err(|e| {
        format!(
            "failed to move scrub sprite into cache {}: {e}",
            jpg_path.display()
        )
    })?;

    let meta = ScrubSpriteMeta {
        cols,
        rows,
        thumb_width: THUMB_WIDTH,
        thumb_height: THUMB_HEIGHT,
        thumb_count,
        interval_sec,
        source_path: path.to_string_lossy().replace('\\', "/"),
    };
    fs::write(
        &json_path,
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("failed to write {}: {e}", json_path.display()))?;

    let data_url = read_data_url(&jpg_path)?;
    Ok(ScrubSpriteReady {
        path: path.to_string_lossy().into_owned(),
        data_url,
        cols: meta.cols,
        rows: meta.rows,
        thumb_width: meta.thumb_width,
        thumb_height: meta.thumb_height,
        thumb_count: meta.thumb_count,
        interval_sec: meta.interval_sec,
    })
}

/// Runs scrub sprite generation with step callbacks (used by the background job worker).
pub fn run_scrub_sprite_job(
    path: &str,
    cancel: &AtomicBool,
    on_step: impl Fn(u32, u32, &str),
) -> Result<ScrubSpriteReady, String> {
    let path_buf = normalized_video_path(path)?;
    let key = cache_key(&path_buf)?;
    if let Some(ready) = load_cached_ready(path, &key)? {
        return Ok(ready);
    }

    on_step(1, 2, "Probing duration");
    let _duration = probe_duration(&path_buf)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("scrub sprite generation cancelled".to_string());
    }

    on_step(2, 2, "Generating sprite sheet");
    generate_sprite(&path_buf, &key, cancel)
}

pub fn emit_scrub_sprite_status(app: &AppHandle, status: ScrubSpriteStatus) {
    let _ = app.emit("scrub-sprite-ready", status);
}

#[tauri::command]
pub async fn get_scrub_sprite_if_ready_cmd(path: String) -> Result<Option<ScrubSpriteReady>, String> {
    tauri::async_runtime::spawn_blocking(move || get_scrub_sprite_if_ready(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn scrub_sprite_is_cached_cmd(path: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || scrub_sprite_is_cached(&path))
        .await
        .map_err(|e| e.to_string())?
}
