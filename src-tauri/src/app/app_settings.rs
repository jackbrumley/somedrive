use crate::app::json_store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 12;
const MIN_DOWNLOAD_CONCURRENCY: usize = 4;
// Microsoft throttles a OneDrive account well before this many parallel
// downloads; higher values only produce 429 storms.
const MAX_DOWNLOAD_CONCURRENCY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AppSettingsFile {
    #[serde(default)]
    sync: SyncSettings,
    #[serde(default)]
    window: WindowSettings,
    #[serde(default)]
    developer: DeveloperSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowSettings {
    /// Closing the window parks it (tray, else taskbar) instead of quitting.
    #[serde(default = "default_close_to_tray")]
    close_to_tray: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            close_to_tray: default_close_to_tray(),
        }
    }
}

fn default_close_to_tray() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncSettings {
    #[serde(default = "default_download_concurrency")]
    download_concurrency: usize,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            download_concurrency: default_download_concurrency(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeveloperSettings {
    #[serde(default)]
    raw_logger_mode: bool,
    /// One of `session_log::LOG_LEVELS`.
    #[serde(default = "default_log_level")]
    log_level: String,
}

impl Default for DeveloperSettings {
    fn default() -> Self {
        Self {
            raw_logger_mode: false,
            log_level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_download_concurrency() -> usize {
    DEFAULT_DOWNLOAD_CONCURRENCY
}

/// The download concurrency setting with the range the UI may offer; the
/// limits are owned here so the slider cannot drift from the clamp.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadConcurrencySetting {
    pub value: usize,
    pub min: usize,
    pub max: usize,
}

impl DownloadConcurrencySetting {
    fn from_value(value: usize) -> Self {
        Self {
            value,
            min: MIN_DOWNLOAD_CONCURRENCY,
            max: MAX_DOWNLOAD_CONCURRENCY,
        }
    }
}

pub fn load_sync_download_concurrency() -> Result<DownloadConcurrencySetting, String> {
    let settings = load_or_create_app_settings()?;
    Ok(DownloadConcurrencySetting::from_value(
        clamp_download_concurrency(settings.sync.download_concurrency),
    ))
}

pub fn save_sync_download_concurrency(value: usize) -> Result<DownloadConcurrencySetting, String> {
    let normalized = clamp_download_concurrency(value);
    let mut settings = load_or_create_app_settings()?;
    settings.sync.download_concurrency = normalized;
    save_app_settings(&settings)?;
    Ok(DownloadConcurrencySetting::from_value(normalized))
}

pub fn load_close_to_tray() -> Result<bool, String> {
    let settings = load_or_create_app_settings()?;
    Ok(settings.window.close_to_tray)
}

pub fn save_close_to_tray(enabled: bool) -> Result<(), String> {
    let mut settings = load_or_create_app_settings()?;
    settings.window.close_to_tray = enabled;
    save_app_settings(&settings)
}

pub fn load_raw_logger_mode() -> Result<bool, String> {
    let settings = load_or_create_app_settings()?;
    Ok(settings.developer.raw_logger_mode)
}

pub fn save_raw_logger_mode(enabled: bool) -> Result<(), String> {
    let mut settings = load_or_create_app_settings()?;
    settings.developer.raw_logger_mode = enabled;
    save_app_settings(&settings)
}

pub fn load_log_level() -> Result<String, String> {
    let settings = load_or_create_app_settings()?;
    Ok(settings.developer.log_level)
}

pub fn save_log_level(level: &str) -> Result<(), String> {
    let mut settings = load_or_create_app_settings()?;
    settings.developer.log_level = level.to_string();
    save_app_settings(&settings)
}

pub fn clamp_download_concurrency(value: usize) -> usize {
    value.clamp(MIN_DOWNLOAD_CONCURRENCY, MAX_DOWNLOAD_CONCURRENCY)
}

fn load_or_create_app_settings() -> Result<AppSettingsFile, String> {
    let path = app_settings_path()?;
    match json_store::read_json::<AppSettingsFile>(&path)? {
        Some(mut parsed) => {
            parsed.sync.download_concurrency =
                clamp_download_concurrency(parsed.sync.download_concurrency);
            Ok(parsed)
        }
        // Missing, or unparseable and moved aside: start from defaults.
        None => {
            let settings = AppSettingsFile::default();
            save_app_settings(&settings)?;
            Ok(settings)
        }
    }
}

fn save_app_settings(settings: &AppSettingsFile) -> Result<(), String> {
    json_store::write_json(&app_settings_path()?, settings)
}

pub fn app_settings_path() -> Result<PathBuf, String> {
    crate::paths::app_settings_file()
}

#[cfg(test)]
mod tests {
    use super::AppSettingsFile;

    #[test]
    fn close_to_tray_defaults_on_for_new_and_existing_settings() {
        assert!(AppSettingsFile::default().window.close_to_tray);
        let existing: AppSettingsFile =
            serde_json::from_str(r#"{"sync":{"downloadConcurrency":12}}"#).unwrap();
        assert!(existing.window.close_to_tray);
    }
}
