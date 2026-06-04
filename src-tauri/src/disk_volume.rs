//! Windows volume helpers for chroma job scheduling (HDD vs SSD detection).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

/// Bumped when detection logic changes so stale entries are not reused.
const ROTATIONAL_CACHE_PREFIX: &str = "r5:";

/// `true` when chroma jobs on this volume should use start staggering (rotational media).
static ROTATIONAL_CACHE: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn rotational_cache_key(volume: &str) -> String {
    format!("{ROTATIONAL_CACHE_PREFIX}{volume}")
}

/// Whether chroma start staggering should apply to files on this path.
/// Defaults to `true` when the path or disk cannot be classified (HDD-safe default).
pub fn path_requires_chroma_stagger(path: &str) -> bool {
    path_requires_chroma_stagger_impl(path).unwrap_or(true)
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
