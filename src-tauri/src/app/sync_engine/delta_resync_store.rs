use super::*;

// Expired delta links (410, resyncRequired): the fresh-enumeration marker
// and the per-item check for items it did not return.

/// OneDrive expired the delta link: forget it and remember when the fresh
/// enumeration started, so items it does not return can be checked one by
/// one afterwards instead of being taken as deleted.
pub(crate) fn begin_delta_resync(profile_id: &str) -> Result<i64, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "INSERT INTO sync_lifecycle_state (profile_id, resync_started_at, updated_at)
             VALUES (?1, ?2, ?2)
             ON CONFLICT(profile_id) DO UPDATE SET
                delta_link = NULL,
                active_delta_next_link = NULL,
                resync_started_at = excluded.resync_started_at,
                updated_at = excluded.updated_at",
            params![profile_id, now],
        )
        .map_err(|error| format!("Failed starting delta resync: {error}"))?;
    Ok(now)
}

pub(crate) fn read_delta_resync_started_at(profile_id: &str) -> Result<Option<i64>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT resync_started_at FROM sync_lifecycle_state WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|error| format!("Failed reading delta resync marker: {error}"))
}

pub(crate) fn finish_delta_resync(profile_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_lifecycle_state SET resync_started_at = NULL WHERE profile_id = ?1",
            params![profile_id],
        )
        .map_err(|error| format!("Failed clearing delta resync marker: {error}"))?;
    Ok(())
}

/// Cloud items recorded before `since` that a fresh enumeration did not
/// return: `(path, remote item id)`.
pub(crate) fn list_cloud_items_unseen_since(
    profile_id: &str,
    since: i64,
) -> Result<Vec<(String, String)>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path, remote_item_id FROM sync_files
             WHERE profile_id = ?1 AND cloud_present = 1 AND remote_item_id IS NOT NULL
               AND cloud_seen_at < ?2 AND path != ''
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing unseen items query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id, since], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| format!("Failed querying unseen items: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading unseen items: {error}"))
}

/// A cloud item confirmed gone (it and anything under it).
pub(crate) fn record_cloud_item_deleted(
    profile_id: &str,
    remote_item_id: &str,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    mark_cloud_item_deleted(
        &connection,
        profile_id,
        remote_item_id,
        current_unix_seconds(),
    )
}
