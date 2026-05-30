use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

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

fn portable_data_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("failed to resolve parent directory for {exe:?}"))?;
    Ok(dir.join("data"))
}

/// GUI release builds have no console; without this, ffmpeg/ffprobe flash CMD windows.
fn hidden_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn find_on_path(file_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn ffmpeg_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "failed to resolve exe directory".to_string())?;
    let beside = dir.join("ffmpeg.exe");
    if beside.is_file() {
        return Ok(beside);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev = manifest.join("libs").join("ffmpeg").join("ffmpeg.exe");
    if dev.is_file() {
        return Ok(dev);
    }
    if let Some(path_ffmpeg) = find_on_path("ffmpeg.exe") {
        return Ok(path_ffmpeg);
    }
    Err(
        "ffmpeg.exe not found beside the app, in the dev tree, or on PATH — \
         run: npm run setup:ffmpeg, or install ffmpeg and add it to PATH"
            .to_string(),
    )
}

fn ffprobe_path() -> Result<PathBuf, String> {
    let ffmpeg = ffmpeg_path()?;
    let beside_ffmpeg = ffmpeg.with_file_name("ffprobe.exe");
    if beside_ffmpeg.is_file() {
        return Ok(beside_ffmpeg);
    }
    if let Some(path_probe) = find_on_path("ffprobe.exe") {
        return Ok(path_probe);
    }
    Err(
        "ffprobe.exe not found beside ffmpeg or on PATH — \
         run: npm run setup:ffmpeg, or install ffmpeg and add it to PATH"
            .to_string(),
    )
}

pub fn normalized_video_path(path: &str) -> Result<PathBuf, String> {
    let path_buf = PathBuf::from(path);
    if !path_buf.is_file() {
        return Err(format!("video file not found: {path}"));
    }
    path_buf
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize {path}: {e}"))
}

pub fn cache_key(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("failed to stat {}: {e}", path.display()))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let canonical = path.to_string_lossy().to_ascii_lowercase();
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
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
    path.replace('\\', "/").to_ascii_lowercase()
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

fn probe_duration(path: &Path) -> Result<f64, String> {
    let ffprobe = ffprobe_path()?;
    let output = hidden_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<f64>()
        .map_err(|e| format!("invalid duration from ffprobe: {e}"))
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
