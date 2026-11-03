//! Logging (App Standard section 3).
//!
//! - `debug/session.log`: this run's log. At start-up the previous run's log
//!   becomes `last-session.log`, so the run before a crash or restart is kept.
//!   SomeDrive runs for weeks in the background, so a session log over
//!   `SESSION_LOG_MAX_BYTES` rolls over to `session.log.1` mid-session.
//! - `debug/profiles/<account>.log`: each account's detailed sync trace (every
//!   file and delta page), size-rotated. Only that account's warnings, errors
//!   and cycle milestones also go to the session log, which keeps it readable
//!   with several busy accounts.
//! - `debug/raw/raw-<time>.log`: everything combined, only while the developer
//!   option "Raw logger mode" is on.

use crate::app::app_settings;
use crate::app::log_context;
use crate::app::system_info::SystemInfo;
use chrono::{Local, SecondsFormat};
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const SESSION_LOG_FILE_NAME: &str = "session.log";
const LAST_SESSION_LOG_FILE_NAME: &str = "last-session.log";
const SESSION_LOG_MAX_BYTES: u64 = 20 * 1024 * 1024;
const SESSION_LOG_ROLLOVER_COUNT: usize = 1;
const PROFILE_LOG_MAX_BYTES: u64 = 20 * 1024 * 1024;
const RAW_LOG_MAX_BYTES: u64 = 200 * 1024 * 1024;
const PROFILE_LOG_ROTATION_COUNT: usize = 5;
const RAW_LOG_RETENTION_COUNT: usize = 10;

/// The log levels offered in Settings -> Debug, most to least severe.
pub const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

static SESSION_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static PROFILE_LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static RAW_LOG_MODE: AtomicBool = AtomicBool::new(false);
static RAW_LOG_SESSION_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

struct SessionLogger;

static LOGGER: SessionLogger = SessionLogger;

impl Log for SessionLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let message = record.args().to_string();
        let line = format!("[{}] {}: {}", log_timestamp(), record.level(), message);

        match record.level() {
            Level::Error | Level::Warn => eprintln!("{}", line),
            _ => println!("{}", line),
        }

        route_log_line(record.level(), &message, &line);
    }

    fn flush(&self) {}
}

/// RFC 3339 local time with milliseconds and offset,
/// e.g. `2026-11-03T19:55:39.123+11:00`.
fn log_timestamp() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

/// Starts this run's session log: rotates the previous one to
/// `last-session.log` and writes the header. Runs after the single-instance
/// guard, so a second launch never rotates the running instance's log.
pub fn initialize_session_logging() -> Result<(), String> {
    let debug_dir = crate::paths::debug_dir()?;
    let session_log_path = debug_dir.join(SESSION_LOG_FILE_NAME);
    let profile_log_dir = debug_dir.join("profiles");
    fs::create_dir_all(&profile_log_dir).map_err(|error| error.to_string())?;

    if session_log_path.exists() {
        let last_session_path = debug_dir.join(LAST_SESSION_LOG_FILE_NAME);
        if let Err(error) = fs::rename(&session_log_path, &last_session_path) {
            eprintln!("Failed to keep the previous session log: {error}");
        }
    }

    let _ = SESSION_LOG_PATH.set(session_log_path);
    let _ = PROFILE_LOG_DIR.set(profile_log_dir);
    let _ = RAW_LOG_SESSION_PATH.set(Mutex::new(None));

    let raw_mode_enabled = app_settings::load_raw_logger_mode().unwrap_or(false);
    RAW_LOG_MODE.store(raw_mode_enabled, Ordering::Relaxed);
    if raw_mode_enabled {
        let _ = ensure_raw_session_path();
    }

    let timestamp = log_timestamp();
    append_session_log_line(&format!("[{timestamp}] INFO: SESSION START"));
    for header_line in SystemInfo::collect().summary().lines() {
        append_session_log_line(&format!("[{timestamp}] INFO: {header_line}"));
    }

    Ok(())
}

/// Installs the `log` backend at the level chosen in Settings -> Debug.
pub fn initialize_app_logger() -> Result<(), String> {
    log::set_logger(&LOGGER).map_err(|error| error.to_string())?;
    let level = app_settings::load_log_level().unwrap_or_else(|_| "info".to_string());
    log::set_max_level(parse_log_level(&level).unwrap_or(LevelFilter::Info));
    Ok(())
}

fn parse_log_level(level: &str) -> Option<LevelFilter> {
    LOG_LEVELS
        .contains(&level)
        .then(|| level.parse::<LevelFilter>().ok())
        .flatten()
}

/// Records every panic in the session log (then runs the default hook), so an
/// unexpected exit always leaves its reason and location behind. Installed
/// first thing in `main`.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|message| message.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic payload".to_string());
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let thread = std::thread::current();
        append_session_log_line(&format!(
            "[{}] PANIC: thread '{}' panicked at {}: {}",
            log_timestamp(),
            thread.name().unwrap_or("unnamed"),
            location,
            message
        ));
        default_hook(info);
    }));
}

pub fn resolve_session_log_path() -> Result<PathBuf, String> {
    match SESSION_LOG_PATH.get() {
        Some(path) => Ok(path.clone()),
        None => Ok(crate::paths::debug_dir()?.join(SESSION_LOG_FILE_NAME)),
    }
}

#[tauri::command]
pub fn get_log_level() -> Result<String, String> {
    app_settings::load_log_level()
}

#[tauri::command]
pub fn set_log_level(level: String) -> Result<String, String> {
    let filter = parse_log_level(&level).ok_or_else(|| format!("Unknown log level '{level}'"))?;
    app_settings::save_log_level(&level)?;
    log::set_max_level(filter);
    log::info!("LOG_LEVEL_UPDATED level={level}");
    Ok(level)
}

#[tauri::command]
pub fn get_raw_logger_mode() -> Result<bool, String> {
    Ok(RAW_LOG_MODE.load(Ordering::Relaxed))
}

#[tauri::command]
pub fn set_raw_logger_mode(enabled: bool) -> Result<(), String> {
    RAW_LOG_MODE.store(enabled, Ordering::Relaxed);
    app_settings::save_raw_logger_mode(enabled)?;

    if enabled {
        let _ = ensure_raw_session_path()?;
    } else if let Some(raw_mutex) = RAW_LOG_SESSION_PATH.get() {
        let mut guard = raw_mutex
            .lock()
            .map_err(|_| "Raw log state lock poisoned".to_string())?;
        *guard = None;
    }

    Ok(())
}

#[tauri::command]
pub fn get_session_log_text() -> Result<String, String> {
    let log_path = resolve_session_log_path()?;
    std::fs::read_to_string(log_path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn copy_session_log() -> Result<(), String> {
    let logs = get_session_log_text()?;
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(logs).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_session_log() -> Result<(), String> {
    let log_path = resolve_session_log_path()?;
    open_path_with_system(&log_path)
}

#[tauri::command]
pub fn open_profile_log(profile_id: String) -> Result<(), String> {
    let log_path = resolve_profile_log_path(&profile_id)?;
    if !log_path.exists() {
        return Err(format!(
            "Profile log does not exist yet for '{}'. Start a sync and try again.",
            profile_id
        ));
    }
    open_path_with_system(&log_path)
}

#[tauri::command]
pub fn open_log_folder() -> Result<(), String> {
    open_path_with_system(&crate::paths::debug_dir()?)
}

fn open_path_with_system(path: &Path) -> Result<(), String> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn log_ui_event(message: String) -> Result<(), String> {
    let sanitized = message.trim();
    if sanitized.is_empty() {
        return Ok(());
    }
    log::info!("UI: {}", sanitized);
    Ok(())
}

fn route_log_line(level: Level, message: &str, line: &str) {
    if let Some(account_key) = extract_account_key(message) {
        append_profile_log_line(&account_key, line);
        if matches!(level, Level::Warn | Level::Error)
            || (matches!(level, Level::Info) && is_account_app_activity(message))
        {
            append_session_log_line(line);
        }
    } else {
        append_session_log_line(line);
    }

    if RAW_LOG_MODE.load(Ordering::Relaxed) {
        append_raw_log_line(line);
    }
}

fn is_account_app_activity(message: &str) -> bool {
    const APP_ACTIVITY_MARKERS: [&str; 16] = [
        "SYNC_WORKER_ALREADY_RUNNING",
        "SYNC_WORKER_STARTING",
        "SYNC_WORKER_STOP_SIGNAL",
        "SYNC_WORKER_RESUME_DELAY",
        "SYNC_TICK",
        "SYNC_TICK_SKIPPED",
        "SYNC_HEARTBEAT",
        "SYNC_CYCLE_START",
        "SYNC_CYCLE_COMPLETE",
        "SYNC_CYCLE_FAILED",
        "SYNC_CYCLE_CANCELLED",
        "REMOTE_PIPELINE_START",
        "REMOTE_SCAN_PROGRESS",
        "LOCAL_SCAN_SUMMARY",
        "DOWNLOAD_RETRY_RECOVERED",
        "UPLOAD_RETRY_RECOVERED",
    ];

    APP_ACTIVITY_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
}

fn append_session_log_line(line: &str) {
    if let Ok(log_path) = resolve_session_log_path() {
        append_line_with_rotation(
            &log_path,
            line,
            SESSION_LOG_MAX_BYTES,
            SESSION_LOG_ROLLOVER_COUNT,
        );
    }
}

fn append_profile_log_line(account_key: &str, line: &str) {
    if let Ok(path) = resolve_profile_log_path_by_account_key(account_key) {
        append_line_with_rotation(
            &path,
            line,
            PROFILE_LOG_MAX_BYTES,
            PROFILE_LOG_ROTATION_COUNT,
        );
    }
}

fn append_raw_log_line(line: &str) {
    if let Ok(path) = ensure_raw_session_path() {
        append_line_with_rotation(&path, line, RAW_LOG_MAX_BYTES, 0);
    }
}

fn append_line_with_rotation(path: &Path, line: &str, max_bytes: u64, keep_count: usize) {
    maybe_rotate(path, max_bytes, keep_count);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", line);
    }
}

fn maybe_rotate(path: &Path, max_bytes: u64, keep_count: usize) {
    if keep_count == 0 {
        return;
    }
    let file_size = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if file_size < max_bytes {
        return;
    }

    for index in (1..=keep_count).rev() {
        let from = if index == 1 {
            path.to_path_buf()
        } else {
            PathBuf::from(format!("{}.{}", path.display(), index - 1))
        };
        let to = PathBuf::from(format!("{}.{}", path.display(), index));
        if from.exists() {
            let _ = fs::rename(&from, &to);
        }
    }
}

fn extract_account_key(message: &str) -> Option<String> {
    let prefix = "[acct:";
    let start = message.find(prefix)?;
    let after_prefix = &message[start + prefix.len()..];
    let end = after_prefix.find(']')?;
    let account_key = after_prefix[..end].trim();
    if account_key.is_empty() {
        return None;
    }
    Some(account_key.to_string())
}

fn sanitize_path_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || character == '.'
        {
            output.push(character);
        } else {
            output.push('_');
        }
    }
    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "unknown-profile".to_string()
    } else {
        trimmed.to_string()
    }
}

fn resolve_profile_log_path(profile_id: &str) -> Result<PathBuf, String> {
    let account_key = log_context::account_identity(profile_id);
    resolve_profile_log_path_by_account_key(&account_key)
}

fn resolve_profile_log_path_by_account_key(account_key: &str) -> Result<PathBuf, String> {
    let dir = PROFILE_LOG_DIR
        .get()
        .cloned()
        .or_else(|| {
            crate::paths::debug_dir()
                .ok()
                .map(|path| path.join("profiles"))
        })
        .ok_or_else(|| "Profile log directory unavailable".to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let safe_key = sanitize_path_component(account_key);
    Ok(dir.join(format!("{}.log", safe_key)))
}

fn ensure_raw_session_path() -> Result<PathBuf, String> {
    let raw_mutex = RAW_LOG_SESSION_PATH
        .get()
        .ok_or_else(|| "Raw log state unavailable".to_string())?;
    let mut guard = raw_mutex
        .lock()
        .map_err(|_| "Raw log state lock poisoned".to_string())?;

    if let Some(existing) = &*guard {
        return Ok(existing.clone());
    }

    let raw_dir = crate::paths::debug_dir()?.join("raw");
    fs::create_dir_all(&raw_dir).map_err(|error| error.to_string())?;
    prune_old_raw_logs(&raw_dir);

    let file_name = format!("raw-{}.log", Local::now().format("%Y-%m-%dT%H-%M-%S"));
    let raw_path = raw_dir.join(file_name);
    *guard = Some(raw_path.clone());
    Ok(raw_path)
}

fn prune_old_raw_logs(raw_dir: &Path) {
    let entries = match fs::read_dir(raw_dir) {
        Ok(value) => value,
        Err(_) => return,
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("log"))
        .collect();
    files.sort();

    if files.len() <= RAW_LOG_RETENTION_COUNT {
        return;
    }

    let remove_count = files.len().saturating_sub(RAW_LOG_RETENTION_COUNT);
    for path in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{log_timestamp, parse_log_level};
    use log::LevelFilter;

    #[test]
    fn timestamps_are_rfc3339_with_millis_and_offset() {
        let timestamp = log_timestamp();
        assert!(chrono::DateTime::parse_from_rfc3339(&timestamp).is_ok());
        // Milliseconds and a numeric offset, e.g. 2026-11-03T19:55:39.123+11:00.
        assert_eq!(timestamp.len(), "2026-11-03T19:55:39.123+11:00".len());
        assert_eq!(&timestamp[19..20], ".");
    }

    #[test]
    fn only_offered_log_levels_parse() {
        assert_eq!(parse_log_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_log_level("off"), None);
        assert_eq!(parse_log_level("loud"), None);
    }
}
