//! Windows volume helpers for chroma job scheduling (HDD vs SSD detection).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

/// Bumped when detection logic changes so stale entries are not reused.
const ROTATIONAL_CACHE_PREFIX: &str = "r5:";

/// Chroma starts on rotational volumes wait until disk busy time is below this (0–100).
pub const CHROMA_HDD_BUSY_THRESHOLD_PERCENT: f64 = 50.0;

/// How often the job scheduler re-checks while chroma jobs are deferred on HDD.
pub const CHROMA_HDD_POLL_MS: u64 = 500;

/// Minimum gap between chroma starts on the same rotational volume (disk % lags).
pub const CHROMA_HDD_MIN_GAP_MS: u64 = 500;

/// When disk busy cannot be read, wait this long after the previous chroma start on that volume.
pub const CHROMA_HDD_WMI_FALLBACK_GAP_MS: u64 = 3000;

/// Reuse one WMI perf snapshot for this long (avoids blocking the UI thread on every scheduler check).
const DISK_BUSY_CACHE_TTL_MS: u64 = CHROMA_HDD_POLL_MS;

/// `true` when chroma jobs on this volume should use HDD deferral (rotational media).
static ROTATIONAL_CACHE: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct DiskBusyCache {
    sampled_at_ms: u64,
    by_letter: HashMap<char, f64>,
}

static DISK_BUSY_CACHE: LazyLock<Mutex<DiskBusyCache>> = LazyLock::new(|| {
    Mutex::new(DiskBusyCache {
        sampled_at_ms: 0,
        by_letter: HashMap::new(),
    })
});

fn rotational_cache_key(volume: &str) -> String {
    format!("{ROTATIONAL_CACHE_PREFIX}{volume}")
}

/// Canonical volume key (`G:/`) for a file path, if a drive letter is present.
pub fn volume_key_for_path(path: &str) -> Option<String> {
    volume_path_for_file(path)
}

/// Whether chroma HDD deferral should apply to files on this path.
/// Defaults to `true` when the path or disk cannot be classified (HDD-safe default).
pub fn path_requires_chroma_stagger(path: &str) -> bool {
    path_requires_chroma_stagger_impl(path).unwrap_or(true)
}

/// Refresh cached `PercentDiskTime` for all drives (at most once per [`DISK_BUSY_CACHE_TTL_MS`]).
pub fn refresh_disk_busy_cache(now_ms: u64) {
    #[cfg(not(windows))]
    let _ = now_ms;

    #[cfg(windows)]
    {
        if let Ok(cache) = DISK_BUSY_CACHE.lock() {
            if now_ms.saturating_sub(cache.sampled_at_ms) < DISK_BUSY_CACHE_TTL_MS {
                return;
            }
        }

        let snapshot = wmi_perf::all_percent_disk_times().unwrap_or_default();

        if let Ok(mut cache) = DISK_BUSY_CACHE.lock() {
            if now_ms.saturating_sub(cache.sampled_at_ms) < DISK_BUSY_CACHE_TTL_MS {
                return;
            }
            cache.sampled_at_ms = now_ms;
            cache.by_letter = snapshot;
        }
    }
}

fn cached_disk_busy_percent_for_path(path: &str, now_ms: u64) -> Option<f64> {
    let letter = drive_letter_from_path(Path::new(path))?;
    refresh_disk_busy_cache(now_ms);
    DISK_BUSY_CACHE
        .lock()
        .ok()?
        .by_letter
        .get(&letter)
        .copied()
}

/// `true` when a chroma job on this path should stay queued (rotational volumes only).
pub fn chroma_start_deferred(path: &str, last_chroma_start_ms: Option<u64>, now_ms: u64) -> bool {
    if !path_requires_chroma_stagger(path) {
        return false;
    }
    if let Some(last) = last_chroma_start_ms {
        if now_ms.saturating_sub(last) < CHROMA_HDD_MIN_GAP_MS {
            return true;
        }
    }
    let busy = cached_disk_busy_percent_for_path(path, now_ms);
    chroma_start_deferred_rotational(busy, last_chroma_start_ms, now_ms)
}

/// Milliseconds until the scheduler should poll again for this deferred chroma path.
pub fn chroma_defer_retry_ms(path: &str, last_chroma_start_ms: Option<u64>, now_ms: u64) -> u64 {
    if !path_requires_chroma_stagger(path) {
        return CHROMA_HDD_POLL_MS;
    }
    if let Some(last) = last_chroma_start_ms {
        let elapsed = now_ms.saturating_sub(last);
        if elapsed < CHROMA_HDD_MIN_GAP_MS {
            return CHROMA_HDD_MIN_GAP_MS.saturating_sub(elapsed).max(1);
        }
    }
    let busy = cached_disk_busy_percent_for_path(path, now_ms);
    if !chroma_start_deferred_rotational(busy, last_chroma_start_ms, now_ms) {
        return CHROMA_HDD_POLL_MS;
    }
    let mut wait = CHROMA_HDD_POLL_MS;
    if let Some(last) = last_chroma_start_ms {
        let elapsed = now_ms.saturating_sub(last);
        if busy.is_none() {
            wait = wait.min(CHROMA_HDD_WMI_FALLBACK_GAP_MS.saturating_sub(elapsed));
        }
    }
    wait.max(1)
}

fn chroma_start_deferred_rotational(
    disk_busy_percent: Option<f64>,
    last_chroma_start_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    if let Some(last) = last_chroma_start_ms {
        if now_ms.saturating_sub(last) < CHROMA_HDD_MIN_GAP_MS {
            return true;
        }
    }
    match disk_busy_percent {
        Some(pct) if disk_usage_blocks_chroma_start(pct) => true,
        Some(_) => false,
        None => last_chroma_start_ms.is_some_and(|last| {
            now_ms.saturating_sub(last) < CHROMA_HDD_WMI_FALLBACK_GAP_MS
        }),
    }
}

fn disk_usage_blocks_chroma_start(usage_percent: f64) -> bool {
    usage_percent >= CHROMA_HDD_BUSY_THRESHOLD_PERCENT
}

fn path_requires_chroma_stagger_impl(path: &str) -> Option<bool> {
    let volume = volume_path_for_file(path)?;
    let cache_key = rotational_cache_key(&volume);
    if let Ok(cache) = ROTATIONAL_CACHE.lock() {
        if let Some(&rotational) = cache.get(&cache_key) {
            return Some(rotational);
        }
    }

    let rotational = volume_is_rotational(&volume)?;
    if let Ok(mut cache) = ROTATIONAL_CACHE.lock() {
        cache.insert(cache_key, rotational);
    }
    Some(rotational)
}

fn normalize_volume_key(volume: &str) -> String {
    volume.replace('\\', "/").to_ascii_uppercase()
}

/// Drive letter from `S:\foo`, `\\?\S:\foo`, or normalized `//?/S:/`.
fn drive_letter_from_path(path: &Path) -> Option<char> {
    drive_letter_from_volume_key(&path.to_string_lossy())
}

fn drive_letter_from_volume_key(key: &str) -> Option<char> {
    let upper = normalize_volume_key(key);
    let colon = upper.find(":/")?;
    let before = &upper[..colon];
    before
        .chars()
        .rev()
        .find(|c| c.is_ascii_alphabetic())
}

#[cfg(windows)]
fn volume_path_for_file(path: &str) -> Option<String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return None;
    }
    let letter = drive_letter_from_path(path)?;
    Some(format!("{}:/", letter.to_ascii_uppercase()))
}

#[cfg(not(windows))]
fn volume_path_for_file(_path: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn volume_is_rotational(volume_key: &str) -> Option<bool> {
    let letter = drive_letter_from_volume_key(volume_key)?;
    media_type_is_hdd_via_wmi(letter).or_else(|| media_type_is_hdd_via_powershell(letter))
}

#[cfg(not(windows))]
fn volume_is_rotational(_volume_key: &str) -> Option<bool> {
    None
}

/// `MSFT_PhysicalDisk.MediaType`: 3 = HDD, 4 = SSD, 5 = SCM (treat as non-rotational).
#[cfg(windows)]
fn media_type_implies_rotational(media_type: u16) -> Option<bool> {
    match media_type {
        3 => Some(true),
        4 | 5 => Some(false),
        _ => None,
    }
}

#[cfg(windows)]
mod wmi_storage {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    const STORAGE_NAMESPACE: &str = "ROOT\\Microsoft\\Windows\\Storage";

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct PartitionRow {
        disk_number: u32,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct PhysicalDiskRow {
        media_type: u16,
    }

    pub fn media_type_for_drive_letter(letter: char) -> Option<u16> {
        let com = COMLibrary::new().ok()?;
        let conn = WMIConnection::with_namespace_path(STORAGE_NAMESPACE, com).ok()?;
        let letter_upper = letter.to_ascii_uppercase();
        let partition_query = format!(
            "SELECT DiskNumber FROM MSFT_Partition WHERE DriveLetter = '{letter_upper}'"
        );
        let partitions: Vec<PartitionRow> = conn.raw_query(&partition_query).ok()?;
        let disk_number = partitions.into_iter().next()?.disk_number;
        let disk_query = format!("SELECT MediaType FROM MSFT_PhysicalDisk WHERE DeviceId = {disk_number}");
        let disks: Vec<PhysicalDiskRow> = conn.raw_query(&disk_query).ok()?;
        disks.into_iter().next().map(|row| row.media_type)
    }
}

#[cfg(windows)]
fn media_type_is_hdd_via_wmi(letter: char) -> Option<bool> {
    let media_type = wmi_storage::media_type_for_drive_letter(letter)?;
    media_type_implies_rotational(media_type)
}

#[cfg(windows)]
mod wmi_perf {
    use std::collections::HashMap;

    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    use super::drive_letter_from_perf_token;

    #[derive(Debug, Deserialize)]
    struct DiskPerfRow {
        #[serde(rename = "PercentDiskTime")]
        percent_disk_time: serde_json::Value,
        #[serde(rename = "Name")]
        name: String,
    }

    fn parse_percent(value: &serde_json::Value) -> Option<f64> {
        match value {
            serde_json::Value::Number(n) => n.as_f64(),
            serde_json::Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// One WMI round-trip; highest `PercentDiskTime` per drive letter in the perf instance name.
    pub fn all_percent_disk_times() -> Option<HashMap<char, f64>> {
        let com = COMLibrary::new().ok()?;
        let conn = WMIConnection::new(com).ok()?;
        let rows: Vec<DiskPerfRow> = conn
            .raw_query(
                "SELECT PercentDiskTime, Name FROM Win32_PerfFormattedData_PerfDisk_PhysicalDisk",
            )
            .ok()?;
        let mut by_letter: HashMap<char, f64> = HashMap::new();
        for row in rows {
            if row.name == "_Total" || row.name.starts_with('_') {
                continue;
            }
            let Some(pct) = parse_percent(&row.percent_disk_time) else {
                continue;
            };
            for token in row.name.split_whitespace() {
                let Some(letter) = drive_letter_from_perf_token(token) else {
                    continue;
                };
                use std::collections::hash_map::Entry;
                match by_letter.entry(letter) {
                    Entry::Occupied(mut slot) => {
                        *slot.get_mut() = slot.get().max(pct);
                    }
                    Entry::Vacant(slot) => {
                        slot.insert(pct);
                    }
                }
            }
        }
        Some(by_letter)
    }
}

fn drive_letter_from_perf_token(token: &str) -> Option<char> {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            Some(drive.to_ascii_uppercase())
        }
        _ => None,
    }
}

#[cfg(windows)]
fn disk_busy_percent_via_powershell(letter: char) -> Option<f64> {
    use crate::media_tools::hidden_command;

    let script = format!(
        "$samples = (Get-Counter '\\PhysicalDisk(*)\\% Disk Time' -ErrorAction Stop).CounterSamples \
         | Where-Object {{ $_.InstanceName -match '(^|\\s){letter}:' }}; \
         if (-not $samples) {{ exit 1 }}; \
         [Math]::Round(($samples | Measure-Object -Property CookedValue -Maximum).Maximum, 2)"
    );
    let output = hidden_command(Path::new("powershell"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
}

/// Fallback when WMI is unavailable (same source as Storage settings).
#[cfg(windows)]
fn media_type_is_hdd_via_powershell(letter: char) -> Option<bool> {
    use crate::media_tools::hidden_command;

    let script = format!(
        "$p = Get-Partition -DriveLetter '{letter}' -ErrorAction Stop; \
         $m = (Get-PhysicalDisk -DeviceNumber $p.DiskNumber).MediaType; \
         if ($m -eq 'HDD') {{ 'true' }} else {{ 'false' }}"
    );
    let output = hidden_command(Path::new("powershell"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&output.stdout).trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_usage_threshold() {
        assert!(!disk_usage_blocks_chroma_start(49.0));
        assert!(disk_usage_blocks_chroma_start(50.0));
        assert!(disk_usage_blocks_chroma_start(100.0));
    }

    #[test]
    fn perf_name_drive_token_matching() {
        assert_eq!(drive_letter_from_perf_token("G:"), Some('G'));
        assert_eq!(drive_letter_from_perf_token("1"), None);
        assert_eq!(drive_letter_from_perf_token("0"), None);
    }

    #[test]
    fn chroma_deferred_respects_min_gap_and_disk_busy() {
        let now = 10_000u64;
        assert!(chroma_start_deferred_rotational(
            Some(10.0),
            Some(now - 100),
            now
        ));
        assert!(chroma_start_deferred_rotational(Some(90.0), Some(now - 600), now));
        assert!(!chroma_start_deferred_rotational(
            Some(10.0),
            Some(now - 600),
            now
        ));
    }

    #[test]
    fn chroma_deferred_wmi_fallback_requires_three_seconds() {
        let now = 10_000u64;
        assert!(!chroma_start_deferred_rotational(None, None, now));
        assert!(chroma_start_deferred_rotational(None, Some(now - 1000), now));
        assert!(!chroma_start_deferred_rotational(None, Some(now - 3000), now));
    }

    #[test]
    fn normalize_volume_key_uppercases_drive() {
        assert_eq!(normalize_volume_key(r"s:\"), "S:/");
    }

    #[test]
    fn drive_letter_from_extended_path() {
        assert_eq!(drive_letter_from_volume_key("//?/S:/"), Some('S'));
        assert_eq!(
            drive_letter_from_path(Path::new(r"\\?\G:\Anime\file.mkv")),
            Some('G')
        );
    }

    /// Known layout on the developer machine; skipped when a drive letter is absent.
    #[test]
    #[cfg(windows)]
    fn known_drive_letters_match_media_type() {
        const CASES: &[(char, bool, &str)] = &[
            ('S', false, "SATA SSD"),
            ('G', true, "SATA HDD"),
            ('C', false, "NVMe SSD"),
            ('D', false, "NVMe SSD"),
            ('V', false, "NVMe SSD"),
        ];

        for &(letter, expect_stagger, label) in CASES {
            let root = format!("{letter}:\\");
            if !Path::new(&root).exists() {
                continue;
            }
            let actual = path_requires_chroma_stagger(&root);
            assert_eq!(
                actual, expect_stagger,
                "{label} ({letter}:): expected stagger={expect_stagger}, got {actual}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn disk_busy_percent_when_env_set() {
        if std::env::var_os("DISK_VOLUME_DEBUG").is_none() {
            return;
        }
        refresh_disk_busy_cache(0);
        for letter in ['G', 'C'] {
            let root = format!("{letter}:\\");
            if !Path::new(&root).exists() {
                continue;
            }
            let counter = disk_busy_percent_via_powershell(letter);
            let cached = cached_disk_busy_percent_for_path(&root, 0);
            let deferred = chroma_start_deferred(&root, Some(0), 500);
            eprintln!(
                "{letter}: cached={cached:?} counter={counter:?} deferred(now)={deferred}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn wmi_media_type_when_env_set() {
        if std::env::var_os("DISK_VOLUME_DEBUG").is_none() {
            return;
        }
        for letter in ['S', 'G', 'C', 'D', 'V'] {
            let root = format!("{letter}:\\");
            if !Path::new(&root).exists() {
                continue;
            }
            let media = wmi_storage::media_type_for_drive_letter(letter);
            let rotational = media.and_then(media_type_implies_rotational);
            eprintln!("{letter}: wmi MediaType={media:?} rotational={rotational:?}");
        }
    }

    #[test]
    #[cfg(windows)]
    fn unknown_volume_defaults_to_stagger() {
        assert!(path_requires_chroma_stagger(
            r"Z:\this\path\should\not\resolve\on\most\systems\file.mkv"
        ));
    }
}
