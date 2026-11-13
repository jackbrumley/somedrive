use super::*;

// Cloud items OneDrive refused to delete because they are locked (423: open
// in an Office app, checked out). Such an item is held as a per-file problem,
// so the rest of the account keeps syncing, and released for another try
// once its retry time has passed. Part of the `sync_files` store.

/// Issue code of a cloud item OneDrive would not delete because it is locked.
pub(crate) const ISSUE_CLOUD_LOCKED: &str = "cloud_locked";

/// How long a locked item waits before SomeDrive tries the delete again.
pub(crate) const CLOUD_LOCKED_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);

const CLOUD_LOCKED_DETAIL: &str = "Deleted on this device, but OneDrive won't delete it \
     because it is open in an Office app or checked out. Close it there; SomeDrive tries \
     again every 5 minutes.";

/// Holds `path` as locked in OneDrive until `now` plus the retry delay.
pub(crate) fn hold_cloud_locked_item(profile_id: &str, path: &str, now: i64) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting locked item transaction: {error}"))?;
    write_sync_file_issue(
        &transaction,
        profile_id,
        path,
        ISSUE_CLOUD_LOCKED,
        CLOUD_LOCKED_DETAIL,
    )?;
    transaction
        .execute(
            "INSERT INTO sync_cloud_locked (profile_id, path, retry_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id, path) DO UPDATE SET retry_at = excluded.retry_at",
            params![
                profile_id,
                path,
                now + CLOUD_LOCKED_RETRY_DELAY.as_secs() as i64
            ],
        )
        .map_err(|error| format!("Failed recording locked item '{path}': {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing locked item '{path}': {error}"))
}

/// Releases every locked item whose retry time has passed: its problem is
/// cleared, so the next plan acts on it again (and a delete that is still
/// refused holds it again). Returns the released paths.
pub(crate) fn release_due_cloud_locked_items(
    profile_id: &str,
    now: i64,
) -> Result<Vec<String>, String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting locked item release: {error}"))?;
    let due: Vec<String> = {
        let mut statement = transaction
            .prepare(
                "SELECT path FROM sync_cloud_locked
                 WHERE profile_id = ?1 AND retry_at <= ?2
                 ORDER BY path",
            )
            .map_err(|error| format!("Failed preparing locked item query: {error}"))?;
        let rows = statement
            .query_map(params![profile_id, now], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Failed querying locked items: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed reading locked items: {error}"))?
    };
    for path in &due {
        transaction
            .execute(
                "UPDATE sync_files
                 SET issue_code = NULL, issue_detail = NULL, updated_at = ?4
                 WHERE profile_id = ?1 AND path = ?2 AND issue_code = ?3",
                params![profile_id, path, ISSUE_CLOUD_LOCKED, now],
            )
            .map_err(|error| format!("Failed releasing locked item '{path}': {error}"))?;
        transaction
            .execute(
                "DELETE FROM sync_cloud_locked WHERE profile_id = ?1 AND path = ?2",
                params![profile_id, path],
            )
            .map_err(|error| format!("Failed releasing locked item '{path}': {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed committing locked item release: {error}"))?;
    Ok(due)
}
