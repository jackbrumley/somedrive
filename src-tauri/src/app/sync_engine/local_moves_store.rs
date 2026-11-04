use super::*;

// Cloud renames and moves repeated on this device: following the move in
// sync_files and queuing (or undoing) the local rename.

/// Rows at `path` or anywhere under it.
pub(crate) const PATH_OR_DESCENDANT: &str =
    "(path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')";

/// A cloud item moved from `old_path` to `new_path` and nothing is tracked at
/// the new path yet: the whole record (cloud, local and baseline) moves with
/// it, and the same rename is queued for this device. Returns false when the
/// new path is already in use, so the caller falls back to moving only the
/// cloud state (re-download plus a guarded delete of the old copy).
pub(crate) fn follow_cloud_move(
    connection: &Connection,
    profile_id: &str,
    old_path: &str,
    new_path: &str,
    now: i64,
) -> Result<bool, String> {
    let destination_in_use: bool = connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM sync_files WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT})"),
            params![profile_id, new_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed checking move destination '{new_path}': {error}"))?;
    if destination_in_use {
        return Ok(false);
    }
    let has_local_copy: bool = connection
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM sync_files WHERE profile_id = ?1 AND local_present = 1 AND {PATH_OR_DESCENDANT})"
            ),
            params![profile_id, old_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed checking local copy of '{old_path}': {error}"))?;
    connection
        .execute(
            &format!(
                "UPDATE sync_files SET path = ?3 || substr(path, length(?2) + 1), updated_at = ?4
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}"
            ),
            params![profile_id, old_path, new_path, now],
        )
        .map_err(|error| format!("Failed moving '{old_path}' to '{new_path}': {error}"))?;
    if has_local_copy {
        connection
            .execute(
                "INSERT INTO sync_local_moves (profile_id, old_path, new_path, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(profile_id, old_path) DO UPDATE SET new_path = excluded.new_path",
                params![profile_id, old_path, new_path, now],
            )
            .map_err(|error| format!("Failed queuing local move of '{old_path}': {error}"))?;
    }
    Ok(true)
}

/// A cloud rename waiting to be repeated on this device.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingLocalMove {
    pub(crate) old_path: String,
    pub(crate) new_path: String,
}

pub(crate) fn list_pending_local_moves(profile_id: &str) -> Result<Vec<PendingLocalMove>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT old_path, new_path FROM sync_local_moves
             WHERE profile_id = ?1 ORDER BY created_at, old_path",
        )
        .map_err(|error| format!("Failed preparing local moves query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| {
            Ok(PendingLocalMove {
                old_path: row.get(0)?,
                new_path: row.get(1)?,
            })
        })
        .map_err(|error| format!("Failed querying local moves: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading local moves: {error}"))
}

pub(crate) fn finish_local_move(profile_id: &str, old_path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_local_moves WHERE profile_id = ?1 AND old_path = ?2",
            params![profile_id, old_path],
        )
        .map_err(|error| format!("Failed finishing local move of '{old_path}': {error}"))?;
    Ok(())
}

/// The rename could not be repeated on disk: the local state and baseline go
/// back to the old path, while the cloud state stays at the new one. The
/// planner then downloads the new path and (through the delete guard)
/// removes the old copy.
pub(crate) fn undo_local_follow(
    profile_id: &str,
    old_path: &str,
    new_path: &str,
) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting move undo: {error}"))?;
    let now = current_unix_seconds();
    transaction
        .execute(
            &format!(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, local_present, local_size, local_mtime_ns,
                    base_present, base_size, base_cloud_size, base_local_mtime_ns, base_etag,
                    base_ctag, base_hash, updated_at
                 )
                 SELECT profile_id, ?3 || substr(path, length(?2) + 1), is_dir, local_present,
                        local_size, local_mtime_ns, base_present, base_size, base_cloud_size,
                        base_local_mtime_ns, base_etag, base_ctag, base_hash, ?4
                 FROM sync_files
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}
                   AND (local_present = 1 OR base_present = 1)"
            ),
            params![profile_id, new_path, old_path, now],
        )
        .map_err(|error| format!("Failed restoring local state at '{old_path}': {error}"))?;
    transaction
        .execute(
            &format!(
                "UPDATE sync_files
                 SET local_present = 0, local_size = 0, local_mtime_ns = 0,
                     base_present = 0, base_size = 0, base_cloud_size = 0, base_local_mtime_ns = 0,
                     base_etag = NULL, base_ctag = NULL, base_hash = NULL, updated_at = ?3
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}"
            ),
            params![profile_id, new_path, now],
        )
        .map_err(|error| format!("Failed clearing local state at '{new_path}': {error}"))?;
    transaction
        .execute(
            "DELETE FROM sync_local_moves WHERE profile_id = ?1 AND old_path = ?2",
            params![profile_id, old_path],
        )
        .map_err(|error| format!("Failed dropping local move of '{old_path}': {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing move undo: {error}"))
}
