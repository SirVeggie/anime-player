use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

/// Portable `data/` directory beside the executable.
pub fn portable_data_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("failed to resolve parent directory for {exe:?}"))?;
    Ok(dir.join("data"))
}

/// GUI release builds have no console; without this, ffmpeg/ffprobe flash CMD windows.
pub fn hidden_command(program: &Path) -> Command {
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

pub fn ffmpeg_path() -> Result<PathBuf, String> {
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

pub fn ffprobe_path() -> Result<PathBuf, String> {
    let ffmpeg = ffmpeg_path()?;
    let beside_ffmpeg = ffmpeg.with_file_name("ffprobe.exe");
    if beside_ffmpeg.is_file() {
        return Ok(beside_ffmpeg);
    }
    if let Some(path_probe) = find_on_path("ffprobe.exe") {
        return Ok(path_probe);
    }
    Err("ffprobe.exe not found beside ffmpeg or on PATH — \
         run: npm run setup:ffmpeg, or install ffmpeg and add it to PATH"
        .to_string())
}

pub fn fpcalc_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "failed to resolve exe directory".to_string())?;
    let beside = dir.join("fpcalc.exe");
    if beside.is_file() {
        return Ok(beside);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev = manifest.join("libs").join("chromaprint").join("fpcalc.exe");
    if dev.is_file() {
        return Ok(dev);
    }
    if let Some(path_fpcalc) = find_on_path("fpcalc.exe") {
        return Ok(path_fpcalc);
    }
    Err(
        "fpcalc.exe not found beside the app, in the dev tree, or on PATH — \
         run: npm run setup:chromaprint, or install Chromaprint fpcalc and add it to PATH"
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

pub fn normalized_video_path_key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// Average frame rate from the first video stream (`r_frame_rate`), or 24 when unknown.
pub fn probe_video_fps(path: &Path) -> Result<f64, String> {
    let ffprobe = ffprobe_path()?;
    let output = hidden_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        return Ok(24.0);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    if let Some((num, den)) = trimmed.split_once('/') {
        let n: f64 = num.trim().parse().unwrap_or(0.0);
        let d: f64 = den.trim().parse().unwrap_or(1.0);
        if n > 0.0 && d > 0.0 {
            return Ok(n / d);
        }
    } else if let Ok(fps) = trimmed.parse::<f64>() {
        if fps > 0.0 {
            return Ok(fps);
        }
    }
    Ok(24.0)
}

pub fn probe_duration(path: &Path) -> Result<f64, String> {
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

/// Decode mono PCM (`s16le`) for a time range. Returns samples at `sample_rate` Hz.
pub fn extract_pcm_range(
    path: &Path,
    start_sec: f64,
    duration_sec: f64,
    sample_rate: u32,
) -> Result<Vec<i16>, String> {
    if duration_sec <= 0.0 {
        return Ok(Vec::new());
    }
    let ffmpeg = ffmpeg_path()?;
    let output = hidden_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error"])
        .args([
            "-ss",
            &format!("{start_sec:.3}"),
            "-t",
            &format!("{duration_sec:.3}"),
        ])
        .arg("-i")
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &sample_rate.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg pcm extract failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let bytes = output.stdout;
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(samples)
}
