use crate::app::sync_engine::{self, SyncProblemsView};

/// The account's sync problems (held conflicts, items SomeDrive cannot sync,
/// failed downloads) and the items it skips by design, with the reasons.
#[tauri::command]
pub fn list_sync_problems(profile_id: String) -> Result<SyncProblemsView, String> {
    sync_engine::list_sync_problems(&profile_id)
}

/// Resolves held conflicts: `resolution` is `keep_local`, `keep_cloud` or
/// `keep_both`. Returns how many files took the choice; the next sync cycle
/// applies it.
#[tauri::command]
pub fn resolve_sync_conflicts(
    profile_id: String,
    paths: Vec<String>,
    resolution: String,
) -> Result<usize, String> {
    sync_engine::resolve_sync_conflicts(&profile_id, &paths, &resolution)
}
