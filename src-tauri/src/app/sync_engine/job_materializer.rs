// Turns the plan in `sync_files.desired_action` into work. Downloads and
// uploads go through the job queue (workers, retries, progress); folder
// creates, content checks, conflicts, deletes and forgets are applied by the
// cycle directly from `PlannedActions`.

/// The plan's non-transfer actions for this cycle.
#[derive(Debug, Default)]
struct PlannedActions {
    verify: Vec<SyncFileRow>,
    conflicts: Vec<SyncFileRow>,
    create_local_folders: Vec<SyncFileRow>,
    create_remote_folders: Vec<SyncFileRow>,
    delete_local: Vec<SyncFileRow>,
    delete_remote: Vec<SyncFileRow>,
    forget: Vec<SyncFileRow>,
}

fn collect_planned_actions(profile_id: &str) -> Result<PlannedActions, String> {
    Ok(PlannedActions {
        verify: list_sync_file_rows_by_action(profile_id, PLANNER_ACTION_VERIFY)?,
        conflicts: list_sync_file_rows_by_action(profile_id, PLANNER_ACTION_CONFLICT)?,
        create_local_folders: list_sync_file_rows_by_action(
            profile_id,
            PLANNER_ACTION_CREATE_LOCAL_FOLDER,
        )?,
        create_remote_folders: list_sync_file_rows_by_action(
            profile_id,
            PLANNER_ACTION_CREATE_REMOTE_FOLDER,
        )?,
        delete_local: list_sync_file_rows_by_action(profile_id, PLANNER_ACTION_DELETE_LOCAL)?,
        delete_remote: list_sync_file_rows_by_action(profile_id, PLANNER_ACTION_DELETE_REMOTE)?,
        forget: list_sync_file_rows_by_action(profile_id, PLANNER_ACTION_FORGET)?,
    })
}

/// Queues every planned download (re-queuing finished jobs) and drops queued
/// downloads the plan no longer wants. Returns the number planned.
fn materialize_planner_download_jobs(profile_id: &str) -> Result<usize, String> {
    let candidates = list_sync_file_download_candidates(profile_id)?;
    let mut planned_item_ids: HashSet<String> = HashSet::new();
    for candidate in &candidates {
        planned_item_ids.insert(candidate.item_id.clone());
        upsert_download_job(
            profile_id,
            &candidate.item_id,
            &candidate.path,
            candidate.remote_size,
            candidate.remote_modified_ts,
        )?;
    }
    prune_unplanned_queued_jobs(profile_id, DOWNLOAD_JOB_DIRECTION, &planned_item_ids)?;
    Ok(candidates.len())
}

/// Queues every planned upload (re-queuing finished jobs) and drops queued
/// uploads the plan no longer wants. Returns the number planned.
fn materialize_planner_upload_jobs(profile_id: &str) -> Result<usize, String> {
    let candidates = list_sync_file_upload_candidates(profile_id)?;
    let mut planned_paths: HashSet<String> = HashSet::new();
    for candidate in &candidates {
        planned_paths.insert(candidate.path.clone());
        upsert_upload_job_queued(
            profile_id,
            &candidate.path,
            candidate.local_size,
            candidate.local_mtime_ns,
        )?;
    }
    prune_unplanned_queued_jobs(profile_id, UPLOAD_JOB_DIRECTION, &planned_paths)?;
    Ok(candidates.len())
}

/// Removes queued or retry-waiting jobs of `direction` whose item the plan
/// no longer wants. Running and finished jobs are left alone.
fn prune_unplanned_queued_jobs(
    profile_id: &str,
    direction: &str,
    planned_item_ids: &HashSet<String>,
) -> Result<usize, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let queued: Vec<(i64, String)> = {
        let mut statement = connection
            .prepare(
                "SELECT id, item_id FROM sync_jobs
                 WHERE profile_id = ?1 AND direction = ?2 AND state IN (?3, ?4)",
            )
            .map_err(|error| format!("Failed preparing queued jobs query: {error}"))?;
        let rows = statement
            .query_map(
                params![
                    profile_id,
                    direction,
                    DOWNLOAD_JOB_STATE_QUEUED,
                    DOWNLOAD_JOB_STATE_RETRY_WAIT
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| format!("Failed querying queued jobs: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed reading queued jobs: {error}"))?
    };
    let mut removed = 0;
    for (job_id, item_id) in queued {
        if planned_item_ids.contains(&item_id) {
            continue;
        }
        removed += connection
            .execute(
                "DELETE FROM sync_jobs WHERE profile_id = ?1 AND direction = ?2 AND id = ?3",
                params![profile_id, direction, job_id],
            )
            .map_err(|error| format!("Failed removing unplanned job: {error}"))?;
    }
    Ok(removed)
}
