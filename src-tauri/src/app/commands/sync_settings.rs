use crate::app::app_settings::{
    load_conflict_policy, load_exclude_patterns, load_sync_download_concurrency,
    save_conflict_policy, save_exclude_patterns, save_sync_download_concurrency, ConflictPolicy,
    DownloadConcurrencySetting,
};
use crate::app::sync_engine;

#[tauri::command]
pub fn get_sync_download_concurrency() -> Result<DownloadConcurrencySetting, String> {
    load_sync_download_concurrency()
}

#[tauri::command]
pub fn set_sync_download_concurrency(value: usize) -> Result<DownloadConcurrencySetting, String> {
    let updated = save_sync_download_concurrency(value)?;
    log::info!(
        "SYNC_SETTING_UPDATED key=download_concurrency value={}",
        updated.value
    );
    Ok(updated)
}

#[tauri::command]
pub fn get_conflict_policy() -> Result<ConflictPolicy, String> {
    load_conflict_policy()
}

#[tauri::command]
pub fn set_conflict_policy(policy: ConflictPolicy) -> Result<ConflictPolicy, String> {
    save_conflict_policy(policy)?;
    log::info!(
        "SYNC_SETTING_UPDATED key=conflict_policy value={:?}",
        policy
    );
    Ok(policy)
}

#[tauri::command]
pub fn get_exclude_patterns() -> Result<Vec<String>, String> {
    load_exclude_patterns()
}

/// Saves the exclude rules after checking every one is a valid pattern.
#[tauri::command]
pub fn set_exclude_patterns(patterns: Vec<String>) -> Result<Vec<String>, String> {
    let cleaned: Vec<String> = patterns
        .iter()
        .map(|pattern| pattern.trim().to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect();
    sync_engine::validate_exclude_patterns(&cleaned)?;
    save_exclude_patterns(cleaned.clone())?;
    log::info!(
        "SYNC_SETTING_UPDATED key=exclude_patterns count={}",
        cleaned.len()
    );
    Ok(cleaned)
}
