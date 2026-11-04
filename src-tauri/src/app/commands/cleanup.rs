use std::path::PathBuf;

use crate::app::account_profiles::load_profiles;
use crate::app::activity_log;
use crate::app::log_context;
use crate::app::state::AppState;
use crate::app::sync_engine::{self, CleanupDeleteResult, CleanupView};

// The Clean up tab: duplicate files and large files, worked out from the
// sync database, and deletes the user confirmed by typing "delete". The
// analysis and the byte-for-byte check run off the UI thread.

/// The account's Clean up suggestions.
#[tauri::command]
pub async fn list_cleanup(profile_id: String) -> Result<CleanupView, String> {
    run_blocking(move || sync_engine::list_cleanup(&profile_id)).await
}

/// Deletes a duplicate suggestion after checking every file byte for byte
/// against the copy that stays. The files go to the Trash here; the sync
/// deletes them in OneDrive, where the recycle bin keeps them.
#[tauri::command]
pub async fn delete_cleanup_duplicates(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    suggestion_id: String,
) -> Result<CleanupDeleteResult, String> {
    let sync_root = account_sync_root(&state, &profile_id)?;
    let account = profile_id.clone();
    let result = run_blocking(move || {
        sync_engine::delete_duplicate_suggestion(&account, &sync_root, &suggestion_id)
    })
    .await?;
    record_activity(&profile_id, "duplicate", &result);
    Ok(result)
}

/// Deletes one file from the Large files list (to the Trash here, then the
/// OneDrive recycle bin).
#[tauri::command]
pub async fn delete_cleanup_large_file(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    path: String,
) -> Result<CleanupDeleteResult, String> {
    let sync_root = account_sync_root(&state, &profile_id)?;
    let account = profile_id.clone();
    let result =
        run_blocking(move || sync_engine::delete_large_file(&account, &sync_root, &path)).await?;
    record_activity(&profile_id, "large", &result);
    Ok(result)
}

/// Hides a file or folder from Clean up.
#[tauri::command]
pub fn hide_cleanup_path(profile_id: String, path: String) -> Result<(), String> {
    sync_engine::hide_cleanup_path(&profile_id, &path)
}

/// Shows a hidden file or folder in Clean up again.
#[tauri::command]
pub fn unhide_cleanup_path(profile_id: String, path: String) -> Result<(), String> {
    sync_engine::unhide_cleanup_path(&profile_id, &path)
}

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("Clean up task failed: {error}"))?
}

fn account_sync_root(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<PathBuf, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    load_profiles()?
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| PathBuf::from(profile.sync_root))
        .ok_or_else(|| "Account profile not found".to_string())
}

fn record_activity(profile_id: &str, kind: &str, result: &CleanupDeleteResult) {
    let message = format!(
        "{} Clean up deleted {} {} file(s), {} bytes (to the Trash; OneDrive keeps them in its recycle bin)",
        log_context::account_prefix(profile_id),
        result.file_count,
        kind,
        result.bytes
    );
    let _ = activity_log::append_event(
        profile_id,
        &log_context::account_identity(profile_id),
        "warning",
        &message,
    );
}
