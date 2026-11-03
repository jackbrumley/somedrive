use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 12;
const MIN_DOWNLOAD_CONCURRENCY: usize = 8;
const MAX_DOWNLOAD_CONCURRENCY: usize = 256;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DeveloperSettings {
    #[serde(default)]
    raw_logger_mode: bool,
}

fn default_download_concurrency() -> usize {
    DEFAULT_DOWNLOAD_CONCURRENCY
}

pub fn load_sync_download_concurrency() -> Result<usize, String> {
    let settings = load_or_create_app_settings()?;
    Ok(clamp_download_concurrency(
        settings.sync.download_concurrency,
    ))
}

pub fn save_sync_download_concurrency(value: usize) -> Result<usize, String> {
    let normalized = clamp_download_concurrency(value);
    let mut settings = load_or_create_app_settings()?;
    settings.sync.download_concurrency = normalized;
    save_app_settings(&settings)?;
    Ok(normalized)
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

pub fn clamp_download_concurrency(value: usize) -> usize {
    value.clamp(MIN_DOWNLOAD_CONCURRENCY, MAX_DOWNLOAD_CONCURRENCY)
}

fn load_or_create_app_settings() -> Result<AppSettingsFile, String> {
    let path = app_settings_path()?;
    if !path.exists() {
        let settings = AppSettingsFile::default();
        save_app_settings(&settings)?;
        return Ok(settings);
    }

    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "Failed reading app settings '{}': {}",
            path.display(),
            error
        )
    })?;
    let mut parsed: AppSettingsFile = serde_json::from_str(&text).map_err(|error| {
        format!(
            "Failed decoding app settings '{}': {}",
            path.display(),
            error
        )
    })?;
    parsed.sync.download_concurrency = clamp_download_concurrency(parsed.sync.download_concurrency);
    Ok(parsed)
}

fn save_app_settings(settings: &AppSettingsFile) -> Result<(), String> {
    let path = app_settings_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed creating app settings directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }

    let text = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("Failed encoding app settings JSON: {error}"))?;
    std::fs::write(&path, text).map_err(|error| {
        format!(
            "Failed writing app settings '{}': {}",
            path.display(),
            error
        )
    })
}

fn app_settings_path() -> Result<PathBuf, String> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Could not resolve config directory".to_string())?;
    Ok(config_dir.join("somedrive").join("app_settings.json"))
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
