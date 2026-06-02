//! Append-only diagnostic log beside the portable `data/` folder.
//!
//! Used for startup crashes, Rust panics, Windows native faults, and frontend
//! errors reported over IPC. Friends can zip `data/diagnostic.log` when reporting bugs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const LOG_FILE_NAME: &str = "diagnostic.log";
const MAX_LOG_BYTES: u64 = 1_048_576;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_MUTEX: Mutex<()> = Mutex::new(());

pub fn init() {
    let path = resolve_log_path();
    let _ = LOG_PATH.set(path.clone());
    rotate_if_oversized(&path);
    install_panic_hook();
    #[cfg(windows)]
    install_native_crash_handler();
    log("INFO", &format!("diagnostic log: {}", path.display()));
    if let Ok(exe) = std::env::current_exe() {
        log("INFO", &format!("executable: {}", exe.display()));
    }
}

pub fn log(level: &str, message: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let Ok(_guard) = LOG_MUTEX.lock() else {
        return;
    };
    let timestamp = chrono_like_timestamp();
    let line = format!("[{timestamp}] [{level}] {message}\n");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return "unknown-time".to_string();
    };
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    let days = secs / 86_400;
    let day_secs = secs % 86_400;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;
    // Approximate UTC date from unix epoch (good enough for diagnostics).
    let (year, month, day) = unix_days_to_ymd(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z"
    )
}

fn unix_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Civil calendar from days since 1970-01-01 (algorithm from Howard Hinnant).
    days += 719_468;
    let era = days / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

fn resolve_log_path() -> PathBuf {
    if let Some(data_dir) = portable_data_dir() {
        if fs::create_dir_all(&data_dir).is_ok() {
            return data_dir.join(LOG_FILE_NAME);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(LOG_FILE_NAME)))
        .unwrap_or_else(|| PathBuf::from(LOG_FILE_NAME))
}

fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    Some(parent.join("data"))
}

fn rotate_if_oversized(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() <= MAX_LOG_BYTES {
        return;
    }
    let backup = path.with_extension("log.old");
    let _ = fs::remove_file(&backup);
    let _ = fs::rename(path, backup);
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| {
                info.payload()
                    .downcast_ref::<String>()
                    .cloned()
            })
            .unwrap_or_else(|| "Box<dyn Any>".to_string());
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown".to_string());
        log("PANIC", &format!("{payload} at {location}"));
        let backtrace = std::backtrace::Backtrace::force_capture();
        log("PANIC", &format!("backtrace:\n{backtrace}"));
        default_hook(info);
    }));
}

#[cfg(windows)]
fn install_native_crash_handler() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(
            native_exception_filter,
        ));
    }
}

#[cfg(windows)]
unsafe extern "system" fn native_exception_filter(
    exception_info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    use windows::Win32::System::Diagnostics::Debug::EXCEPTION_CONTINUE_SEARCH;

    if !exception_info.is_null() {
        let info = &*exception_info;
        if let Some(record) = info.ExceptionRecord.as_ref() {
            let code = record.ExceptionCode.0 as u32;
            let address = record.ExceptionAddress as usize;
            log(
                "FATAL",
                &format!("unhandled native exception: code=0x{code:08X} at 0x{address:X}"),
            );
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

#[cfg(not(windows))]
fn install_native_crash_handler() {}
