use super::*;

// The Clean up tab's data: the account's tracked files read for the
// analysis (cleanup_finder.rs), and the paths the user hid from it.

/// The account's Clean up view, worked out from the sync database.
pub fn list_cleanup(profile_id: &str) -> Result<CleanupView, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let files = read_cleanup_files(&connection, profile_id)?;
    let hidden = read_cleanup_hidden(&connection, profile_id)?;
    Ok(analyse_cleanup(&files, &hidden))
}

/// Every tracked file, sorted by path. A file counts as in sync when both
/// sides hold the synced version and nothing is pending for it.
pub(crate) fn read_cleanup_files(
    connection: &Connection,
    profile_id: &str,
) -> Result<Vec<CleanupFile>, String> {
    let mut statement = connection
        .prepare(
            "SELECT path, cloud_size, cloud_hash, cloud_mtime,
                    cloud_present = 1 AND local_present = 1 AND base_present = 1
                    AND desired_action = 'none' AND issue_code IS NULL
                    AND cloud_hash IS NOT NULL AND cloud_hash = base_hash
                    AND cloud_size = base_cloud_size
                    AND local_size = base_size AND local_mtime_ns = base_local_mtime_ns
             FROM sync_files
             WHERE profile_id = ?1 AND is_dir = 0 AND is_shared_reference = 0
               AND (cloud_present = 1 OR local_present = 1)
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing clean-up query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| {
            Ok(CleanupFile {
                path: row.get(0)?,
                size: row.get::<_, i64>(1)?.max(0) as u64,
                hash: row.get(2)?,
                mtime: row.get(3)?,
                in_sync: sql_to_bool(row.get(4)?),
            })
        })
        .map_err(|error| format!("Failed querying clean-up files: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading clean-up files: {error}"))
}

pub(crate) fn read_cleanup_hidden(
    connection: &Connection,
    profile_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT path FROM cleanup_hidden WHERE profile_id = ?1 ORDER BY path")
        .map_err(|error| format!("Failed preparing hidden clean-up query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Failed querying hidden clean-up paths: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading hidden clean-up paths: {error}"))
}

/// Hides `path` (a file or folder) from Clean up: nothing in it is
/// suggested again, though its files still count as the copy that stays.
pub fn hide_cleanup_path(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "INSERT INTO cleanup_hidden (profile_id, path, hidden_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id, path) DO NOTHING",
            params![profile_id, path, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed hiding '{path}' from Clean up: {error}"))?;
    log::info!(
        "{} CLEANUP_HIDDEN path={}",
        log_context::account_prefix(profile_id),
        path
    );
    Ok(())
}

/// Shows a hidden path in Clean up again.
pub fn unhide_cleanup_path(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM cleanup_hidden WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path],
        )
        .map_err(|error| format!("Failed showing '{path}' in Clean up again: {error}"))?;
    log::info!(
        "{} CLEANUP_UNHIDDEN path={}",
        log_context::account_prefix(profile_id),
        path
    );
    Ok(())
}
