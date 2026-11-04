use crate::app::app_settings::{
    load_conflict_policy, load_sync_download_concurrency, save_conflict_policy,
    save_sync_download_concurrency, ConflictPolicy, DownloadConcurrencySetting,
};

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
