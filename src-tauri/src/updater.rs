use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use url::Url;

use crate::crash_log;

const MANIFEST_URL: &str =
    "https://github.com/SirVeggie/anime-player/releases/latest/download/manifest.json";
const USER_AGENT: &str = "anime-player-updater";
const PENDING_DIR: &str = "_pending";
const APPLY_FILE: &str = "apply.json";
const APPLY_FAILED_FILE: &str = "apply.failed.json";
const ERROR_FILE: &str = "error.txt";
const VERSION_FILE: &str = "VERSION.txt";
const EXE_NAME: &str = "anime-player.exe";
const ALLOWED_FILES: &[&str] = &[
    "anime-player.exe",
    "libmpv-2.dll",
    "ffmpeg.exe",
    "ffprobe.exe",
    "fpcalc.exe",
    "update.bat",
    "_update.ps1",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub schema: u32,
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyPlan {
    version: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Idle,
    Checking,
    Downloading,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    pub phase: UpdatePhase,
    pub file: Option<String>,
    pub downloaded: u64,
    pub total: u64,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub notes: String,
    pub available: bool,
    pub pending_apply: bool,
    pub downloading: bool,
    pub updates_supported: bool,
    pub download_bytes: u64,
    pub files_to_download: Vec<String>,
    pub last_error: Option<String>,
    pub progress: UpdateProgress,
}

impl UpdateStatus {
    fn initial() -> Self {
        Self {
            current_version: read_local_version().ok().flatten(),
            latest_version: None,
            notes: String::new(),
            available: false,
            pending_apply: pending_apply_exists(),
            downloading: false,
            updates_supported: updates_supported(),
            download_bytes: 0,
            files_to_download: Vec::new(),
            last_error: read_apply_error(),
            progress: UpdateProgress {
                phase: UpdatePhase::Idle,
                file: None,
                downloaded: 0,
                total: 0,
                bytes_downloaded: 0,
                bytes_total: 0,
            },
        }
    }
}

pub struct UpdaterState {
    inner: Mutex<UpdaterInner>,
}

struct UpdaterInner {
    status: UpdateStatus,
    manifest: Option<UpdateManifest>,
}

impl UpdaterState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(UpdaterInner {
                status: UpdateStatus::initial(),
                manifest: None,
            }),
        }
    }
}

pub fn try_apply_pending_on_startup() {
    if !updates_supported() {
        return;
    }
    if pending_apply_exists() {
        match spawn_apply_helper() {
            Ok(()) => {
                crash_log::log("INFO", "exiting so a pending update can be applied");
                std::process::exit(0);
            }
            Err(error) => {
                crash_log::log(
                    "ERROR",
                    &format!("failed to start pending update apply: {error}"),
                );
            }
        }
        return;
    }
    cleanup_stale_pending();
}

fn cleanup_stale_pending() {
    let Ok(pending) = pending_dir() else {
        return;
    };
    if !pending.is_dir() || pending.join(APPLY_FILE).is_file() {
        return;
    }
    if let Err(error) = fs::remove_dir_all(&pending) {
        crash_log::log(
            "WARN",
            &format!("could not remove leftover update files: {error}"),
        );
    }
}

pub fn apply_pending_update() -> i32 {
    crash_log::init();
    match apply_pending_update_inner() {
        Ok(()) => 0,
        Err(error) => {
            crash_log::log("ERROR", &format!("apply update failed: {error}"));
            let _ = write_apply_error(&error);
            let _ = mark_apply_failed();
            let _ = relaunch_installed_exe();
            1
        }
    }
}

fn updates_supported() -> bool {
    cfg!(all(windows, not(debug_assertions)))
}

fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "executable has no parent directory".to_string())
}

fn pending_dir() -> Result<PathBuf, String> {
    Ok(install_dir()?.join(PENDING_DIR))
}

fn pending_apply_path() -> Result<PathBuf, String> {
    Ok(pending_dir()?.join(APPLY_FILE))
}

fn pending_apply_exists() -> bool {
    pending_apply_path()
        .map(|path| path.is_file())
        .unwrap_or(false)
}

fn read_local_version() -> Result<Option<String>, String> {
    let path = install_dir()?.join(VERSION_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("failed to read {VERSION_FILE}: {e}"))?;
    let version = text.trim();
    if version.is_empty() {
        Ok(None)
    } else {
        Ok(Some(version.to_string()))
    }
}

fn read_apply_error() -> Option<String> {
    let path = pending_dir().ok()?.join(ERROR_FILE);
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn write_apply_error(message: &str) -> Result<(), String> {
    let dir = pending_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create pending directory: {e}"))?;
    fs::write(dir.join(ERROR_FILE), format!("{message}\n"))
        .map_err(|e| format!("failed to write update error: {e}"))
}

fn mark_apply_failed() -> Result<(), String> {
    let pending = pending_dir()?;
    let apply = pending.join(APPLY_FILE);
    if apply.is_file() {
        let failed = pending.join(APPLY_FAILED_FILE);
        let _ = fs::remove_file(&failed);
        fs::rename(&apply, failed).map_err(|e| format!("failed to mark apply as failed: {e}"))?;
    }
    Ok(())
}

fn allowed_file_name(name: &str) -> bool {
    ALLOWED_FILES.contains(&name)
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
}

fn normalize_sha256(value: &str) -> Result<String, String> {
    let hash = value.trim().to_ascii_lowercase();
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("manifest file hash is not a SHA256 hex digest".to_string());
    }
    Ok(hash)
}

fn validate_manifest(manifest: &UpdateManifest) -> Result<(), String> {
    if manifest.schema != 1 {
        return Err(format!("unsupported updater manifest schema {}", manifest.schema));
    }
    if manifest.version.trim().is_empty() {
        return Err("manifest is missing a version".to_string());
    }
    if manifest.files.is_empty() {
        return Err("manifest does not list any files".to_string());
    }
    for file in &manifest.files {
        if !allowed_file_name(&file.name) {
            return Err(format!("manifest lists a disallowed file: {}", file.name));
        }
        normalize_sha256(&file.sha256)?;
        let url = Url::parse(&file.url).map_err(|e| format!("invalid file url for {}: {e}", file.name))?;
        if url.scheme() != "https" {
            return Err(format!("file url for {} must be https", file.name));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn local_file_needs_update(install: &Path, file: &ManifestFile) -> Result<bool, String> {
    let path = install.join(&file.name);
    if !path.is_file() {
        return Ok(true);
    }
    let actual = sha256_file(&path)?;
    Ok(actual != normalize_sha256(&file.sha256)?)
}

fn parse_version_parts(tag: &str) -> Option<(u32, u32)> {
    let trimmed = tag.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

fn remote_is_newer(remote: &str, local: Option<&str>) -> bool {
    let Some(local) = local else {
        return true;
    };
    if local == remote {
        return false;
    }
    match (parse_version_parts(remote), parse_version_parts(local)) {
        (Some(remote_parts), Some(local_parts)) => remote_parts > local_parts,
        _ => remote != local,
    }
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))
}

async fn fetch_manifest() -> Result<UpdateManifest, String> {
    let response = http_client()?
        .get(MANIFEST_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("failed to download update manifest: {e}"))?
        .error_for_status()
        .map_err(|e| format!("failed to download update manifest: {e}"))?;
    let manifest = response
        .json::<UpdateManifest>()
        .await
        .map_err(|e| format!("failed to parse update manifest: {e}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn files_to_download(
    install: &Path,
    manifest: &UpdateManifest,
) -> Result<(Vec<ManifestFile>, u64), String> {
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for file in &manifest.files {
        if local_file_needs_update(install, file)? {
            bytes = bytes.saturating_add(file.size);
            files.push(file.clone());
        }
    }
    Ok((files, bytes))
}

fn emit_status(app: &AppHandle, status: &UpdateStatus) {
    let _ = app.emit("update://status", status);
}

fn emit_progress(app: &AppHandle, progress: &UpdateProgress) {
    let _ = app.emit("update://progress", progress);
}

fn with_state<T>(state: &UpdaterState, f: impl FnOnce(&mut UpdaterInner) -> T) -> Result<T, String> {
    let mut guard = state
        .inner
        .lock()
        .map_err(|_| "updater state is poisoned".to_string())?;
    Ok(f(&mut guard))
}

fn refresh_status_from_manifest(
    inner: &mut UpdaterInner,
    manifest: UpdateManifest,
) -> Result<(), String> {
    let install = install_dir()?;
    let (needed, download_bytes) = files_to_download(&install, &manifest)?;
    let current_version = read_local_version()?.or_else(|| inner.status.current_version.clone());
    let available = remote_is_newer(&manifest.version, current_version.as_deref()) || !needed.is_empty();
    inner.manifest = Some(manifest.clone());
    inner.status.current_version = current_version;
    inner.status.latest_version = Some(manifest.version);
    inner.status.notes = manifest.notes;
    inner.status.available = available;
    inner.status.pending_apply = pending_apply_exists();
    inner.status.download_bytes = download_bytes;
    inner.status.files_to_download = needed.into_iter().map(|file| file.name).collect();
    inner.status.last_error = read_apply_error();
    inner.status.updates_supported = updates_supported();
    if !inner.status.downloading {
        inner.status.progress.phase = if inner.status.pending_apply {
            UpdatePhase::Ready
        } else {
            UpdatePhase::Idle
        };
    }
    Ok(())
}

#[tauri::command]
pub fn updater_get_status(state: State<'_, UpdaterState>) -> Result<UpdateStatus, String> {
    with_state(&state, |inner| {
        inner.status.current_version = read_local_version().ok().flatten();
        inner.status.pending_apply = pending_apply_exists();
        inner.status.last_error = read_apply_error();
        inner.status.updates_supported = updates_supported();
        inner.status.clone()
    })
}

#[tauri::command]
pub async fn updater_check(
    app: AppHandle,
    state: State<'_, UpdaterState>,
) -> Result<UpdateStatus, String> {
    {
        let snapshot = with_state(&state, |inner| {
            inner.status.progress.phase = UpdatePhase::Checking;
            inner.status.last_error = None;
            inner.status.clone()
        })?;
        emit_status(&app, &snapshot);
    }

    let result = fetch_manifest().await;
    with_state(&state, |inner| match result {
        Ok(manifest) => {
            refresh_status_from_manifest(inner, manifest)?;
            emit_status(&app, &inner.status);
            Ok(inner.status.clone())
        }
        Err(error) => {
            inner.status.progress.phase = UpdatePhase::Error;
            inner.status.last_error = Some(error.clone());
            emit_status(&app, &inner.status);
            Err(error)
        }
    })?
}

#[tauri::command]
pub async fn updater_start_download(
    app: AppHandle,
    state: State<'_, UpdaterState>,
) -> Result<UpdateStatus, String> {
    if !updates_supported() {
        return Err("Updates are disabled in development builds.".to_string());
    }

    let already_downloading = with_state(&state, |inner| inner.status.downloading)?;
    if already_downloading {
        return Err("An update is already downloading.".to_string());
    }

    let manifest = match with_state(&state, |inner| inner.manifest.clone())? {
        Some(manifest) => manifest,
        None => fetch_manifest().await?,
    };
    validate_manifest(&manifest)?;

    let install = install_dir()?;
    let (needed, bytes_total) = files_to_download(&install, &manifest)?;
    if needed.is_empty() {
        return with_state(&state, |inner| {
            refresh_status_from_manifest(inner, manifest.clone())?;
            inner.status.available = false;
            emit_status(&app, &inner.status);
            Ok(inner.status.clone())
        })?;
    }

    with_state(&state, |inner| {
        inner.manifest = Some(manifest.clone());
        inner.status.downloading = true;
        inner.status.last_error = None;
        inner.status.progress = UpdateProgress {
            phase: UpdatePhase::Downloading,
            file: needed.first().map(|file| file.name.clone()),
            downloaded: 0,
            total: needed.len() as u64,
            bytes_downloaded: 0,
            bytes_total,
        };
        emit_status(&app, &inner.status);
        emit_progress(&app, &inner.status.progress);
    })?;

    let download_result = download_files(&app, &state, &needed).await;
    match download_result {
        Ok(()) => {
            write_apply_plan(&manifest.version, &needed)?;
            let _ = fs::remove_file(pending_dir()?.join(ERROR_FILE));
            with_state(&state, |inner| {
                inner.status.downloading = false;
                inner.status.pending_apply = true;
                inner.status.available = true;
                inner.status.progress.phase = UpdatePhase::Ready;
                inner.status.progress.file = None;
                inner.status.progress.downloaded = needed.len() as u64;
                inner.status.progress.bytes_downloaded = bytes_total;
                emit_status(&app, &inner.status);
                inner.status.clone()
            })
        }
        Err(error) => {
            let _ = write_apply_error(&error);
            with_state(&state, |inner| {
                inner.status.downloading = false;
                inner.status.progress.phase = UpdatePhase::Error;
                inner.status.last_error = Some(error.clone());
                emit_status(&app, &inner.status);
            })?;
            Err(error)
        }
    }
}

async fn download_files(
    app: &AppHandle,
    state: &State<'_, UpdaterState>,
    files: &[ManifestFile],
) -> Result<(), String> {
    let pending = pending_dir()?;
    fs::create_dir_all(&pending).map_err(|e| format!("failed to create pending directory: {e}"))?;
    let client = http_client()?;
    let mut bytes_downloaded = 0u64;
    let bytes_total: u64 = files.iter().map(|file| file.size).sum();

    for (index, file) in files.iter().enumerate() {
        let dest = pending.join(&file.name);
        download_one_file(
            app,
            state,
            &client,
            file,
            &dest,
            index as u64,
            files.len() as u64,
            bytes_downloaded,
            bytes_total,
        )
        .await?;
        bytes_downloaded = bytes_downloaded.saturating_add(file.size);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_one_file(
    app: &AppHandle,
    state: &State<'_, UpdaterState>,
    client: &Client,
    file: &ManifestFile,
    dest: &Path,
    file_index: u64,
    file_total: u64,
    bytes_already: u64,
    bytes_total: u64,
) -> Result<(), String> {
    let expected = normalize_sha256(&file.sha256)?;
    if dest.is_file() && sha256_file(dest)? == expected {
        return Ok(());
    }

    let tmp = dest.with_extension(format!(
        "{}.part",
        dest.extension().and_then(|ext| ext.to_str()).unwrap_or("download")
    ));
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }

    let mut response = client
        .get(&file.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|e| format!("failed to download {}: {e}", file.name))?
        .error_for_status()
        .map_err(|e| format!("failed to download {}: {e}", file.name))?;

    let mut output =
        File::create(&tmp).map_err(|e| format!("failed to create {}: {e}", tmp.display()))?;
    let mut hasher = Sha256::new();
    let mut file_downloaded = 0u64;

    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|e| format!("failed to read {}: {e}", file.name))?;
        let Some(chunk) = chunk else {
            break;
        };
        hasher.update(&chunk);
        output
            .write_all(&chunk)
            .map_err(|e| format!("failed to write {}: {e}", file.name))?;
        file_downloaded += chunk.len() as u64;

        let progress = UpdateProgress {
            phase: UpdatePhase::Downloading,
            file: Some(file.name.clone()),
            downloaded: file_index,
            total: file_total,
            bytes_downloaded: bytes_already.saturating_add(file_downloaded),
            bytes_total,
        };
        let _ = with_state(state, |inner| {
            inner.status.progress = progress.clone();
        });
        emit_progress(app, &progress);
    }

    drop(output);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "SHA256 mismatch for {}. Expected {expected} but got {actual}.",
            file.name
        ));
    }
    fs::rename(&tmp, dest).map_err(|e| format!("failed to finalize {}: {e}", file.name))?;
    Ok(())
}

fn write_apply_plan(version: &str, files: &[ManifestFile]) -> Result<(), String> {
    let pending = pending_dir()?;
    let plan = ApplyPlan {
        version: version.to_string(),
        files: files.iter().map(|file| file.name.clone()).collect(),
    };
    let text = serde_json::to_string_pretty(&plan).map_err(|e| format!("failed to write apply plan: {e}"))?;
    fs::write(pending.join(APPLY_FILE), text).map_err(|e| format!("failed to write apply plan: {e}"))?;
    let _ = fs::remove_file(pending.join(APPLY_FAILED_FILE));
    Ok(())
}

#[tauri::command]
pub fn updater_apply_and_restart(app: AppHandle) -> Result<(), String> {
    if !updates_supported() {
        return Err("Updates are disabled in development builds.".to_string());
    }
    if !pending_apply_exists() {
        return Err("No downloaded update is ready to apply.".to_string());
    }
    spawn_apply_helper()?;
    crash_log::log("INFO", "restarting to apply downloaded update");
    crate::app_lifecycle::confirm_quit(app)
}

fn spawn_apply_helper() -> Result<(), String> {
    let install = install_dir()?;
    let pending_exe = install.join(PENDING_DIR).join(EXE_NAME);
    let helper = if pending_exe.is_file() {
        pending_exe
    } else {
        std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?
    };
    let parent_pid = std::process::id();

    let mut command = Command::new(&helper);
    command
        .arg("--apply-update")
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .current_dir(&install)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    command
        .spawn()
        .map_err(|e| format!("failed to start update helper: {e}"))?;
    Ok(())
}

fn apply_pending_update_inner() -> Result<(), String> {
    let parent_pid = parse_parent_pid();
    if let Some(pid) = parent_pid {
        wait_for_pid(pid);
        std::thread::sleep(Duration::from_millis(400));
    }

    let install = installed_dir_from_helper()?;
    let pending = install.join(PENDING_DIR);
    let apply_path = pending.join(APPLY_FILE);
    if !apply_path.is_file() {
        return Err("no pending update plan was found".to_string());
    }
    let plan: ApplyPlan = serde_json::from_str(
        &fs::read_to_string(&apply_path).map_err(|e| format!("failed to read apply plan: {e}"))?,
    )
    .map_err(|e| format!("failed to parse apply plan: {e}"))?;

    let self_exe = std::env::current_exe().ok();
    for name in &plan.files {
        if !allowed_file_name(name) {
            return Err(format!("pending update listed a disallowed file: {name}"));
        }
        let source = pending.join(name);
        if !source.is_file() {
            return Err(format!("pending update is missing {name}"));
        }
        let dest = install.join(name);
        if let Some(self_exe) = &self_exe {
            if paths_equal(self_exe, &dest) {
                continue;
            }
        }
        fs::copy(&source, &dest).map_err(|e| format!("failed to install {name}: {e}"))?;
    }

    fs::write(install.join(VERSION_FILE), plan.version.as_bytes())
        .map_err(|e| format!("failed to write {VERSION_FILE}: {e}"))?;

    let running_from_pending = self_exe
        .as_ref()
        .and_then(|path| path.parent())
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        == Some(PENDING_DIR);
    if running_from_pending {
        let _ = fs::remove_file(&apply_path);
        let _ = fs::remove_file(pending.join(ERROR_FILE));
        for name in &plan.files {
            let staged = pending.join(name);
            if self_exe
                .as_ref()
                .is_some_and(|exe| paths_equal(exe, &staged))
            {
                continue;
            }
            let _ = fs::remove_file(staged);
        }
    } else {
        fs::remove_dir_all(&pending)
            .map_err(|e| format!("failed to remove pending update files: {e}"))?;
    }

    crash_log::log("INFO", &format!("applied update {}", plan.version));
    relaunch_exe(&install.join(EXE_NAME), &install)
}

fn installed_dir_from_helper() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "executable has no parent directory".to_string())?;
    if dir.file_name().and_then(|name| name.to_str()) == Some(PENDING_DIR) {
        dir.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "pending update folder has no install directory".to_string())
    } else {
        Ok(dir.to_path_buf())
    }
}

fn relaunch_installed_exe() -> Result<(), String> {
    let install = installed_dir_from_helper()?;
    relaunch_exe(&install.join(EXE_NAME), &install)
}

fn relaunch_exe(exe: &Path, install: &Path) -> Result<(), String> {
    let mut command = Command::new(exe);
    command
        .current_dir(install)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .spawn()
        .map_err(|e| format!("failed to relaunch Anime Player: {e}"))?;
    Ok(())
}

fn parse_parent_pid() -> Option<u32> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == "--parent-pid" {
            return args.next()?.parse().ok();
        }
    }
    None
}

fn wait_for_pid(pid: u32) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
                return;
            };
            let _ = WaitForSingleObject(handle, u32::MAX);
            let _ = CloseHandle(handle);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}
