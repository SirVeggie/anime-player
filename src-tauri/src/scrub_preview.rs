use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    Generating { path: String },
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

struct ActiveGeneration {
    path: String,
    cancel: Arc<AtomicBool>,
}

static ACTIVE_GENERATION: Mutex<Option<ActiveGeneration>> = Mutex::new(None);

fn portable_data_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("failed to resolve parent directory for {exe:?}"))?;
    Ok(dir.join("data"))
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
    Err("ffmpeg.exe not found ÔÇö run: npm run setup:ffmpeg".to_string())
}

fn ffprobe_path() -> Result<PathBuf, String> {
    let ffmpeg = ffmpeg_path()?;
    let probe = ffmpeg.with_file_name("ffprobe.exe");
    if probe.is_file() {
        return Ok(probe);
    }
    Err("ffprobe.exe not found ÔÇö run: npm run setup:ffmpeg".to_string())
}

fn normalized_video_path(path: &str) -> Result<PathBuf, String> {
    let path_buf = PathBuf::from(path);
    if !path_buf.is_file() {
        return Err(format!("video file not found: {path}"));
    }
    path_buf
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize {path}: {e}"))
}

fn cache_key(path: &Path) -> Result<String, String> {
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

fn probe_duration(path: &Path) -> Result<f64, String> {
    let ffprobe = ffprobe_path()?;
    let output = Command::new(&ffprobe)
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

fn load_cached_ready(path: &str, key: &str) -> Result<Option<ScrubSpriteReady>, String> {
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
    let output = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .arg("-i")
        .arg(path)
        .args(["-an", "-vf", &vf, "-frames:v", "1", "-q:v", "5"])
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

fn cancel_active_generation() {
    let mut guard = ACTIVE_GENERATION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = guard.take() {
        active.cancel.store(true, Ordering::Relaxed);
    }
}

fn start_generation(app: AppHandle, path: String, key: String) {
    cancel_active_generation();

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut guard = ACTIVE_GENERATION.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(ActiveGeneration {
            path: path.clone(),
            cancel: Arc::clone(&cancel),
        });
    }

    tauri::async_runtime::spawn(async move {
        let path_for_job = path.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let path_buf = PathBuf::from(&path_for_job);
            generate_sprite(&path_buf, &key, &cancel)
        })
        .await;

        let mut still_active = false;
        {
            let mut guard = ACTIVE_GENERATION.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(active) = guard.as_ref() {
                still_active = active.path == path && !active.cancel.load(Ordering::Relaxed);
            }
            if still_active {
                *guard = None;
            }
        }

        if !still_active {
            return;
        }

        let emit_path = path.clone();
        match result {
            Ok(Ok(ready)) => {
                let _ = app.emit(
                    "scrub-sprite-ready",
                    ScrubSpriteStatus::Ready(ready),
                );
            }
            Ok(Err(_)) | Err(_) => {
                let _ = app.emit(
                    "scrub-sprite-ready",
                    ScrubSpriteStatus::Unavailable { path: emit_path },
                );
            }
        }
    });
}

fn ensure_scrub_sprite_blocking(app: &AppHandle, path: String) -> Result<ScrubSpriteStatus, String> {
    let path_buf = match normalized_video_path(&path) {
        Ok(path_buf) => path_buf,
        Err(_) => return Ok(ScrubSpriteStatus::Unavailable { path }),
    };
    let path = path_buf.to_string_lossy().into_owned();

    let key = cache_key(&path_buf)?;
    if let Some(mut ready) = load_cached_ready(&path, &key)? {
        ready.path = path.clone();
        return Ok(ScrubSpriteStatus::Ready(ready));
    }
    {
        let guard = ACTIVE_GENERATION.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = guard.as_ref() {
            if active.path == path {
                return Ok(ScrubSpriteStatus::Generating { path: path.clone() });
            }
        }
    }

    start_generation(app.clone(), path.clone(), key);
    Ok(ScrubSpriteStatus::Generating { path })
}

#[tauri::command]
pub async fn ensure_scrub_sprite(app: AppHandle, path: String) -> Result<ScrubSpriteStatus, String> {
    tauri::async_runtime::spawn_blocking(move || ensure_scrub_sprite_blocking(&app, path))
        .await
        .map_err(|e| e.to_string())?
}
