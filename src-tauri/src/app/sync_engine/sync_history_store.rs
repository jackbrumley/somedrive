use super::*;

// Finished actions that are not transfers (deletes) recorded next to the
// finished downloads and uploads, so "Recently synced" shows everything the
// sync did. Queue counters and claims filter on the download and upload
// directions, so these rows never count as work.

/// Deleted in OneDrive, so moved to the Trash on this device.
pub(crate) const HISTORY_TRASHED_LOCALLY: &str = "trash_local";
/// Deleted on this device, so deleted in OneDrive (its recycle bin keeps it).
pub(crate) const HISTORY_DELETED_REMOTELY: &str = "delete_remote";

/// Records a finished delete for "Recently synced".
pub(crate) fn record_finished_delete(
    profile_id: &str,
    direction: &str,
    path: &str,
    size: u64,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "INSERT INTO sync_jobs (
                profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                state, run_state, attempt_count, bytes_done, bytes_total,
                created_at, updated_at, started_at, finished_at
            ) VALUES (?1, ?2, ?3, ?3, ?4, 0, ?5, ?6, 0, ?4, ?4, ?7, ?7, ?7, ?7)
            ON CONFLICT(profile_id, direction, item_id) DO UPDATE SET
                remote_size = excluded.remote_size,
                state = excluded.state,
                bytes_done = excluded.bytes_done,
                bytes_total = excluded.bytes_total,
                updated_at = excluded.updated_at,
                finished_at = excluded.finished_at",
            params![
                profile_id,
                direction,
                path,
                size as i64,
                DOWNLOAD_JOB_STATE_DONE,
                JOB_RUN_STATE_IDLE,
                now,
            ],
        )
        .map_err(|error| format!("Failed recording the delete of '{path}': {error}"))?;
    Ok(())
}

#[cfg(test)]
mod sync_history_tests {
    use super::*;

    #[test]
    fn a_delete_shows_as_finished_without_counting_as_work() {
        let profile_id = test_profile_id("history-delete");
        record_finished_delete(&profile_id, HISTORY_TRASHED_LOCALLY, "Photos/a.png", 1208)
            .expect("record");
        record_finished_delete(&profile_id, HISTORY_TRASHED_LOCALLY, "Photos/a.png", 1208)
            .expect("record again");

        let connection = open_sync_jobs_connection(&profile_id).expect("open");
        let projection = read_sync_job_activity_projection(&connection, &profile_id, 10, 10)
            .expect("projection");
        let recent: Vec<(&str, &str)> = projection
            .recent_completed
            .iter()
            .map(|item| (item.direction.as_str(), item.path.as_str()))
            .collect();
        assert_eq!(recent, vec![(HISTORY_TRASHED_LOCALLY, "Photos/a.png")]);

        let downloads = query_download_job_counters(&connection, &profile_id).expect("counters");
        assert_eq!(downloads.planned_total, 0, "not a download");
        assert_eq!(projection.active.len(), 0, "never active work");
    }
}
