//! Windows volume / physical-disk helpers for job scheduling.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetVolumePathNameW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
#[cfg(windows)]
use windows::Win32::System::IO::DeviceIoControl;

/// Bumped when detection logic changes so stale entries are not reused.
const ROTATIONAL_CACHE_PREFIX: &str = "r4:";

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

#[cfg(windows)]
fn wide_null_terminated(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn volume_path_for_file(path: &str) -> Option<String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return None;
    }

    if let Some(letter) = drive_letter_from_path(path) {
        return Some(format!("{}:/", letter.to_ascii_uppercase()));
    }

    let input = wide_null_terminated(&path.to_string_lossy());
    let mut output = vec![0u16; 260];
    if unsafe { GetVolumePathNameW(PCWSTR(input.as_ptr()), &mut output) }.is_err() {
        return None;
    }

    let len = output.iter().position(|&c| c == 0).unwrap_or(output.len());
    let volume = String::from_utf16_lossy(&output[..len]);
    if volume.is_empty() {
        return None;
    }
    Some(normalize_volume_key(&volume))
}

#[cfg(not(windows))]
fn volume_path_for_file(_path: &str) -> Option<String> {
    None
}

fn normalize_volume_key(volume: &str) -> String {
    volume.replace('\\', "/").to_ascii_uppercase()
}

/// Drive letter from `S:\foo`, `\\?\S:\foo`, or normalized `//?/S:/`.
fn drive_letter_from_path(path: &Path) -> Option<char> {
    let text = path.to_string_lossy();
    drive_letter_from_volume_key(&text)
}

fn drive_letter_from_volume_key(key: &str) -> Option<char> {
    let upper = key.replace('\\', "/").to_ascii_uppercase();
    let colon = upper.find(":/")?;
    let before = &upper[..colon];
    before
        .chars()
        .rev()
        .find(|c| c.is_ascii_alphabetic())
}

/// Classify a volume as rotational only when every backing disk is confidently non-rotational.
#[cfg(windows)]
fn volume_is_rotational(volume_key: &str) -> Option<bool> {
    let letter = drive_letter_from_volume_key(volume_key)?;

    if let Some(device) = volume_device_path(volume_key) {
        if let Some(handle) = open_device_read(&device) {
            if let Some(rotational) = storage_handle_is_rotational(handle) {
                let _ = unsafe { CloseHandle(handle) };
                return Some(rotational);
            }

            if let Some(disks) = disk_numbers_on_volume(handle) {
                let mut any_rotational = false;
                let mut all_confidently_non_rotational = true;
                for disk in disks {
                    match physical_disk_is_rotational(disk) {
                        Some(true) => {
                            any_rotational = true;
                            all_confidently_non_rotational = false;
                        }
                        Some(false) => {}
                        None => all_confidently_non_rotational = false,
                    }
                }
                let _ = unsafe { CloseHandle(handle) };
                if any_rotational {
                    return Some(true);
                }
                if all_confidently_non_rotational {
                    return Some(false);
                }
            } else {
                let _ = unsafe { CloseHandle(handle) };
            }
        }
    }

    media_type_is_hdd_via_powershell(letter)
}

#[cfg(windows)]
fn storage_handle_is_rotational(handle: HANDLE) -> Option<bool> {
    query_seek_penalty(handle)
        .or_else(|| query_rotation_rate(handle))
        .or_else(|| query_rotational_from_bus_type(handle))
}

/// Uses `Get-PhysicalDisk` / `Get-Partition` (same data as Storage settings).
#[cfg(windows)]
fn media_type_is_hdd_via_powershell(letter: char) -> Option<bool> {
    use std::process::Command;

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

#[cfg(not(windows))]
fn volume_is_rotational(_volume_key: &str) -> Option<bool> {
    None
}

#[cfg(windows)]
fn volume_device_path(volume_key: &str) -> Option<String> {
    let letter = drive_letter_from_volume_key(volume_key)?;
    Some(format!("\\\\.\\{letter}:"))
}

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;

#[cfg(windows)]
fn open_device_read(device: &str) -> Option<HANDLE> {
    let wide = wide_null_terminated(device);
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    }
    .ok()
    .filter(|h| *h != INVALID_HANDLE_VALUE)
}

#[cfg(windows)]
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
#[cfg(windows)]
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
#[cfg(windows)]
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
#[cfg(windows)]
const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: u32 = 7;
#[cfg(windows)]
const STORAGE_DEVICE_ROTATION_RATE_PROPERTY: u32 = 8;
#[cfg(windows)]
const STORAGE_DEVICE_PROPERTY: u32 = 0;
#[cfg(windows)]
const PROPERTY_STANDARD_QUERY: u32 = 0;

/// `STORAGE_PROPERTY_QUERY` for `PropertyStandardQuery` (PropertyId + QueryType only).
#[repr(C)]
#[cfg(windows)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
}

#[repr(C)]
#[cfg(windows)]
struct DiskExtent {
    disk_number: u32,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
#[cfg(windows)]
struct VolumeDiskExtentsHeader {
    number_of_disk_extents: u32,
    _padding: u32,
}

#[repr(C)]
#[cfg(windows)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: u32,
    partition_number: u32,
}

#[repr(C)]
#[cfg(windows)]
struct DeviceSeekPenaltyDescriptor {
    version: u32,
    size: u32,
    incurs_seek_penalty: u8,
}

#[repr(C)]
#[cfg(windows)]
struct StorageDescriptorHeader {
    version: u32,
    size: u32,
}

#[cfg(windows)]
const BUS_TYPE_NVME: u32 = 17;
#[cfg(windows)]
const BUS_TYPE_SD: u32 = 12;
#[cfg(windows)]
const BUS_TYPE_MMC: u32 = 13;
#[cfg(windows)]
const BUS_TYPE_UFS: u32 = 19;
#[cfg(windows)]
const STORAGE_DEVICE_BUS_TYPE_OFFSET: usize = 28;

#[cfg(windows)]
fn disk_number_via_storage_device_number(volume: HANDLE) -> Option<u32> {
    let mut info = StorageDeviceNumber {
        device_type: 0,
        device_number: 0,
        partition_number: 0,
    };
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            volume,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some(&mut info as *mut _ as *mut c_void),
            size_of::<StorageDeviceNumber>() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() {
        return None;
    }
    Some(info.device_number)
}

#[cfg(windows)]
fn disk_numbers_via_volume_extents(volume: HANDLE) -> Option<Vec<u32>> {
    let header_size = size_of::<VolumeDiskExtentsHeader>();
    let extent_size = size_of::<DiskExtent>();
    let mut buffer = vec![0u8; header_size + extent_size * 8];
    debug_assert_eq!(header_size, 8);
    let mut returned = 0u32;

    let ok = unsafe {
        DeviceIoControl(
            volume,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut c_void),
            buffer.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() || returned < header_size as u32 {
        return None;
    }

    let header = unsafe { &*(buffer.as_ptr() as *const VolumeDiskExtentsHeader) };
    let count = header.number_of_disk_extents as usize;
    let needed = header_size + count * extent_size;
    if (returned as usize) < needed {
        return None;
    }

    let mut disks = Vec::with_capacity(count);
    for i in 0..count {
        let offset = header_size + i * extent_size;
        let extent = unsafe { &*(buffer.as_ptr().add(offset) as *const DiskExtent) };
        disks.push(extent.disk_number);
    }
    Some(disks)
}

#[cfg(windows)]
fn disk_numbers_on_volume(volume: HANDLE) -> Option<Vec<u32>> {
    if let Some(n) = disk_number_via_storage_device_number(volume) {
        return Some(vec![n]);
    }
    disk_numbers_via_volume_extents(volume)
}

#[cfg(windows)]
fn physical_disk_is_rotational(disk_number: u32) -> Option<bool> {
    let device = format!("\\\\.\\PhysicalDrive{disk_number}");
    let handle = open_device_read(&device)?;
    let rotational = query_seek_penalty(handle)
        .or_else(|| query_rotation_rate(handle))
        .or_else(|| query_rotational_from_bus_type(handle));
    let _ = unsafe { CloseHandle(handle) };
    rotational
}

#[cfg(windows)]
fn storage_property_query(property_id: u32) -> StoragePropertyQuery {
    StoragePropertyQuery {
        property_id,
        query_type: PROPERTY_STANDARD_QUERY,
    }
}

#[cfg(windows)]
fn query_storage_descriptor(device: HANDLE, property_id: u32) -> Option<Vec<u8>> {
    let query = storage_property_query(property_id);
    let query_size = size_of::<StoragePropertyQuery>() as u32;

    let mut header = StorageDescriptorHeader {
        version: 0,
        size: 0,
    };
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            device,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const c_void),
            query_size,
            Some(&mut header as *mut _ as *mut c_void),
            size_of::<StorageDescriptorHeader>() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() || header.size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; header.size as usize];
    let ok = unsafe {
        DeviceIoControl(
            device,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const c_void),
            query_size,
            Some(buffer.as_mut_ptr() as *mut c_void),
            buffer.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() {
        return None;
    }
    buffer.truncate(returned as usize);
    Some(buffer)
}

#[cfg(windows)]
fn query_seek_penalty(device: HANDLE) -> Option<bool> {
    let buffer = query_storage_descriptor(device, STORAGE_DEVICE_SEEK_PENALTY_PROPERTY)?;
    if buffer.len() < size_of::<DeviceSeekPenaltyDescriptor>() {
        return None;
    }
    let descriptor = unsafe { &*(buffer.as_ptr() as *const DeviceSeekPenaltyDescriptor) };
    Some(descriptor.incurs_seek_penalty != 0)
}

/// `STORAGE_ROTATION_RATE_DESCRIPTOR`: non-rotating media reports no meaningful RPM.
#[cfg(windows)]
fn query_rotation_rate(device: HANDLE) -> Option<bool> {
    let buffer = query_storage_descriptor(device, STORAGE_DEVICE_ROTATION_RATE_PROPERTY)?;
    if buffer.len() < 16 {
        return None;
    }
    let rotation_rate_in_rpm = buffer[8] != 0;
    if !rotation_rate_in_rpm {
        return Some(false);
    }
    let rpm = u32::from_le_bytes(buffer[12..16].try_into().ok()?);
    if rpm == 0 || rpm == 1 {
        return Some(false);
    }
    if rpm == 0xFFFF_FFFF {
        return None;
    }
    if rpm >= 1_000 {
        return Some(true);
    }
    None
}

#[cfg(windows)]
fn query_rotational_from_bus_type(device: HANDLE) -> Option<bool> {
    let buffer = query_storage_descriptor(device, STORAGE_DEVICE_PROPERTY)?;
    if buffer.len() <= STORAGE_DEVICE_BUS_TYPE_OFFSET + 4 {
        return None;
    }
    let bus_type = u32::from_le_bytes(
        buffer[STORAGE_DEVICE_BUS_TYPE_OFFSET..STORAGE_DEVICE_BUS_TYPE_OFFSET + 4]
            .try_into()
            .ok()?,
    );
    bus_type_implies_rotational(bus_type)
}

#[cfg(windows)]
fn bus_type_implies_rotational(bus_type: u32) -> Option<bool> {
    match bus_type {
        BUS_TYPE_NVME | BUS_TYPE_SD | BUS_TYPE_MMC | BUS_TYPE_UFS => Some(false),
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
        assert_eq!(
            drive_letter_from_volume_key("//?/S:/"),
            Some('S')
        );
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
            let probe = root.clone();
            let actual = path_requires_chroma_stagger(&probe);
            assert_eq!(
                actual, expect_stagger,
                "{label} ({letter}:): expected stagger={expect_stagger}, got {actual}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn probe_volume_ioctl() {
        if std::env::var_os("DISK_VOLUME_DEBUG").is_none() {
            return;
        }
        use windows::Win32::Foundation::GetLastError;
        for letter in ['S', 'G', 'C'] {
            let dev = format!("\\\\.\\{letter}:");
            let handle = open_device_read(&dev);
            let err = unsafe { GetLastError() };
            eprintln!("{letter}: open={} err={err:?}", handle.is_some());
            let Some(handle) = handle else {
                continue;
            };
            eprintln!("  device# {:?}", disk_number_via_storage_device_number(handle));
            eprintln!(
                "  volume ioctl: seek={:?} rotation={:?} bus={:?} combined={:?}",
                query_seek_penalty(handle),
                query_rotation_rate(handle),
                query_rotational_from_bus_type(handle),
                storage_handle_is_rotational(handle),
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn debug_classification_when_env_set() {
        if std::env::var_os("DISK_VOLUME_DEBUG").is_none() {
            return;
        }
        for letter in ['S', 'G', 'C', 'D', 'V'] {
            let root = format!("{letter}:\\");
            if !Path::new(&root).exists() {
                continue;
            }
            let vol = volume_path_for_file(&root).expect("volume");
            let rotational = volume_is_rotational(&vol);
            let stagger = path_requires_chroma_stagger(&root);
            eprintln!("{letter}: volume={vol:?} rotational={rotational:?} stagger={stagger}");
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
