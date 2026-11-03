pub(crate) fn list_terminal_failed_download_paths(
    profile_id: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND state = ?3
             ORDER BY finished_at DESC, updated_at DESC
             LIMIT ?4",
        )
        .map_err(|error| format!("Failed preparing terminal failed path query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
                limit as i64,
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Failed querying terminal failed download paths: {error}"))?;

    let mut paths: Vec<String> = Vec::new();
    for row in rows {
        paths.push(row.map_err(|error| format!("Failed reading terminal failed path row: {error}"))?);
    }
    Ok(paths)
}

fn remove_download_job_by_item_id(profile_id: &str, item_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND item_id = ?3",
            params![profile_id, DOWNLOAD_JOB_DIRECTION, item_id],
        )
        .map_err(|error| format!("Failed removing download job by item id: {error}"))?;
    Ok(())
}

fn upsert_download_job(
    profile_id: &str,
    item_id: &str,
    path: &str,
    remote_size: u64,
    remote_modified_ts: i64,
) -> Result<bool, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();

    let existing_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM sync_jobs WHERE profile_id = ?1 AND direction = ?2 AND item_id = ?3",
            params![profile_id, DOWNLOAD_JOB_DIRECTION, item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed querying existing download job: {error}"))?;

    if existing_id.is_none() {
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at, lease_owner, lease_until,
                    bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, 0, NULL, NULL, NULL, NULL,
                    0, NULL, ?9,
                    ?9, ?9, NULL, NULL
                )",
                params![
                    profile_id,
                    DOWNLOAD_JOB_DIRECTION,
                    item_id,
                    path,
                    remote_size as i64,
                    remote_modified_ts,
                    DOWNLOAD_JOB_STATE_QUEUED,
                    JOB_RUN_STATE_IDLE,
                    now,
                ],
            )
            .map_err(|error| format!("Failed inserting download job: {error}"))?;
        return Ok(true);
    }

    connection
        .execute(
            "UPDATE sync_jobs
             SET path = ?1,
                 remote_size = ?2,
                 remote_modified_ts = ?3,
                 bytes_total = CASE WHEN ?2 > 0 THEN ?2 ELSE bytes_total END,
                 progress_updated_at = COALESCE(progress_updated_at, ?4),
                 updated_at = ?4
             WHERE profile_id = ?5 AND direction = ?6 AND item_id = ?7",
            params![
                path,
                remote_size as i64,
                remote_modified_ts,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                item_id,
            ],
        )
        .map_err(|error| format!("Failed updating download job metadata: {error}"))?;
    Ok(false)
}

fn claim_download_jobs(
    profile_id: &str,
    lease_owner: &str,
    max_jobs: usize,
) -> Result<Vec<ClaimedDownloadJob>, String> {
    if max_jobs == 0 {
        return Ok(Vec::new());
    }

    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting download claim transaction: {error}"))?;
    let now = current_unix_seconds();
    let lease_until = now.saturating_add(DOWNLOAD_JOB_LEASE_SECONDS);

    transaction
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 next_retry_at = NULL,
                 updated_at = ?3,
                 progress_updated_at = COALESCE(progress_updated_at, ?3)
             WHERE profile_id = ?4
               AND direction = ?5
               AND state = ?6
               AND lease_until IS NOT NULL
               AND lease_until <= ?3",
            params![
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
            ],
        )
        .map_err(|error| format!("Failed recovering stale in-progress download jobs: {error}"))?;

    let mut statement = transaction
        .prepare(
            "SELECT id, item_id, path, remote_size, remote_modified_ts
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND state IN (?3, ?4)
               AND (next_retry_at IS NULL OR next_retry_at <= ?5)
             ORDER BY created_at ASC
             LIMIT ?6",
        )
        .map_err(|error| format!("Failed preparing download claim query: {error}"))?;

    let mut jobs: Vec<ClaimedDownloadJob> = Vec::new();
    let rows = statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_QUEUED,
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
                now,
                max_jobs as i64,
            ],
            |row| {
                Ok(ClaimedDownloadJob {
                    job_id: row.get(0)?,
                    item_id: row.get(1)?,
                    path: row.get(2)?,
                    remote_size: row.get::<_, i64>(3)?.max(0) as u64,
                    remote_modified_ts: row.get(4)?,
                })
            },
        )
        .map_err(|error| format!("Failed executing download claim query: {error}"))?;

    for row in rows {
        jobs.push(row.map_err(|error| format!("Failed reading claimed download job: {error}"))?);
    }
    drop(statement);

    for job in &jobs {
        transaction
            .execute(
                "UPDATE sync_jobs
                 SET state = ?1,
                     run_state = ?2,
                     attempt_count = attempt_count + 1,
                     lease_owner = ?3,
                     lease_until = ?4,
                     started_at = COALESCE(started_at, ?5),
                     updated_at = ?5,
                     last_error = NULL,
                     next_retry_at = NULL,
                     bytes_done = 0,
                     bytes_total = CASE WHEN remote_size > 0 THEN remote_size ELSE NULL END,
                     progress_updated_at = ?5
                 WHERE id = ?6",
                params![
                    DOWNLOAD_JOB_STATE_IN_PROGRESS,
                    JOB_RUN_STATE_CLAIMED,
                    lease_owner,
                    lease_until,
                    now,
                    job.job_id,
                ],
            )
            .map_err(|error| format!("Failed marking download job as in-progress: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("Failed committing download claim transaction: {error}"))?;
    Ok(jobs)
}

fn mark_download_job_done(profile_id: &str, job_id: i64, skipped: bool) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let final_state = if skipped {
        DOWNLOAD_JOB_STATE_SKIPPED
    } else {
        DOWNLOAD_JOB_STATE_DONE
    };
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 bytes_done = COALESCE(bytes_total, bytes_done),
                 updated_at = ?3,
                 finished_at = ?3,
                 progress_updated_at = ?3
             WHERE profile_id = ?4 AND direction = ?5 AND id = ?6",
            params![
                final_state,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking download job done: {error}"))?;
    Ok(())
}

fn mark_download_job_done_with_local_index(
    profile_id: &str,
    job_id: i64,
    remote_entry: &RemoteKnownItem,
    local_entry: &LocalSnapshotEntry,
) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting download completion transaction: {error}"))?;
    let now = current_unix_seconds();

    transaction
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 bytes_done = COALESCE(bytes_total, bytes_done),
                 updated_at = ?3,
                 finished_at = ?3,
                 progress_updated_at = ?3
             WHERE profile_id = ?4 AND direction = ?5 AND id = ?6",
            params![
                DOWNLOAD_JOB_STATE_DONE,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking download job done: {error}"))?;

    transaction
        .execute(
            "INSERT INTO sync_files (
                profile_id, path, is_dir, is_shared_reference, shared_drive_id, shared_item_id, shared_kind,
                remote_item_id, remote_present, local_present,
                remote_size, local_size, remote_modified_ts, local_modified_ts,
                desired_action, conflict_state, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, 1, 1,
                ?9, ?10, ?11, ?12,
                'none', NULL, ?13
             )
             ON CONFLICT(profile_id, path)
             DO UPDATE SET
                is_dir = excluded.is_dir,
                is_shared_reference = excluded.is_shared_reference,
                shared_drive_id = excluded.shared_drive_id,
                shared_item_id = excluded.shared_item_id,
                shared_kind = excluded.shared_kind,
                remote_item_id = excluded.remote_item_id,
                remote_present = 1,
                local_present = 1,
                remote_size = excluded.remote_size,
                local_size = excluded.local_size,
                remote_modified_ts = excluded.remote_modified_ts,
                local_modified_ts = excluded.local_modified_ts,
                updated_at = excluded.updated_at",
            params![
                profile_id,
                remote_entry.path,
                bool_to_sql(remote_entry.is_dir),
                bool_to_sql(remote_entry.is_shared_reference),
                remote_entry.shared_drive_id.as_deref(),
                remote_entry.shared_item_id.as_deref(),
                remote_entry.shared_kind.as_deref(),
                remote_entry.id,
                remote_entry.size as i64,
                local_entry.size as i64,
                remote_entry.modified_ts,
                local_entry.modified_ts,
                now,
            ],
        )
        .map_err(|error| format!("Failed upserting local sync file row during completion: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Failed committing download completion transaction: {error}"))?;
    Ok(())
}

fn upsert_sync_file_remote_presence(profile_id: &str, remote_entry: &RemoteKnownItem) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "INSERT INTO sync_files (
                profile_id, path, is_dir, is_shared_reference, shared_drive_id, shared_item_id, shared_kind,
                remote_item_id, remote_present, local_present,
                remote_size, local_size, remote_modified_ts, local_modified_ts,
                desired_action, conflict_state, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, 1, 0,
                ?9, 0, ?10, 0,
                'none', NULL, ?11
             )
             ON CONFLICT(profile_id, path)
             DO UPDATE SET
                is_dir = excluded.is_dir,
                is_shared_reference = excluded.is_shared_reference,
                shared_drive_id = excluded.shared_drive_id,
                shared_item_id = excluded.shared_item_id,
                shared_kind = excluded.shared_kind,
                remote_item_id = excluded.remote_item_id,
                remote_present = 1,
                remote_size = excluded.remote_size,
                remote_modified_ts = excluded.remote_modified_ts,
                updated_at = excluded.updated_at",
            params![
                profile_id,
                remote_entry.path,
                bool_to_sql(remote_entry.is_dir),
                bool_to_sql(remote_entry.is_shared_reference),
                remote_entry.shared_drive_id.as_deref(),
                remote_entry.shared_item_id.as_deref(),
                remote_entry.shared_kind.as_deref(),
                remote_entry.id,
                remote_entry.size as i64,
                remote_entry.modified_ts,
                now,
            ],
        )
        .map_err(|error| format!("Failed upserting remote sync file row: {error}"))?;
    Ok(())
}

fn mark_download_job_running(profile_id: &str, job_id: i64) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let lease_until = now.saturating_add(DOWNLOAD_JOB_LEASE_SECONDS);
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 started_at = COALESCE(started_at, ?3),
                 lease_until = ?4,
                 updated_at = ?3,
                 progress_updated_at = ?3
              WHERE profile_id = ?5 AND direction = ?6 AND id = ?7",
            params![
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                now,
                lease_until,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking download job running: {error}"))?;
    Ok(())
}

fn update_download_job_progress(
    profile_id: &str,
    job_id: i64,
    bytes_done: u64,
    bytes_total: Option<u64>,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let lease_until = now.saturating_add(DOWNLOAD_JOB_LEASE_SECONDS);
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 bytes_done = ?3,
                 bytes_total = COALESCE(?4, bytes_total),
                 lease_until = ?6,
                 updated_at = ?5,
                 progress_updated_at = ?5
             WHERE profile_id = ?7 AND direction = ?8 AND id = ?9",
            params![
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                bytes_done as i64,
                bytes_total.map(|value| value as i64),
                now,
                lease_until,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed updating download job progress: {error}"))?;
    Ok(())
}

fn mark_download_job_failed(profile_id: &str, job_id: i64, error_text: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 last_error = ?3,
                 updated_at = ?4,
                 finished_at = ?4,
                 progress_updated_at = ?4
             WHERE profile_id = ?5 AND direction = ?6 AND id = ?7",
            params![
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
                JOB_RUN_STATE_IDLE,
                error_text,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking download job failed: {error}"))?;
    Ok(())
}

pub fn retry_failed_download_job(
    profile_id: &str,
    recent_item_id: &str,
) -> Result<RetryFailedDownloadJobStatus, String> {
    let job_id_text = recent_item_id
        .strip_prefix("job-")
        .ok_or_else(|| "Invalid sync activity item id; expected 'job-<id>'".to_string())?;
    let job_id = job_id_text
        .parse::<i64>()
        .map_err(|_| "Invalid sync activity job id".to_string())?;
    if job_id <= 0 {
        return Err("Invalid sync activity job id".to_string());
    }

    let connection = open_sync_jobs_connection(profile_id)?;
    let current_state: Option<String> = connection
        .query_row(
            "SELECT state
             FROM sync_jobs
             WHERE profile_id = ?1 AND direction = ?2 AND id = ?3",
            params![profile_id, DOWNLOAD_JOB_DIRECTION, job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed loading failed download job: {error}"))?;

    let Some(state) = current_state else {
        return Ok(RetryFailedDownloadJobStatus::AlreadyRetrying);
    };

    if state != DOWNLOAD_JOB_STATE_FAILED_TERMINAL {
        return Ok(RetryFailedDownloadJobStatus::AlreadyRetrying);
    }

    let now = current_unix_seconds();
    let updated = connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 last_error = NULL,
                 next_retry_at = NULL,
                 bytes_done = 0,
                 finished_at = NULL,
                 updated_at = ?3,
                 progress_updated_at = ?3
             WHERE profile_id = ?4
               AND direction = ?5
               AND id = ?6
               AND state = ?7",
            params![
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
            ],
        )
        .map_err(|error| format!("Failed retrying failed download job: {error}"))?;

    if updated == 0 {
        return Ok(RetryFailedDownloadJobStatus::AlreadyRetrying);
    }

    Ok(RetryFailedDownloadJobStatus::Retried)
}

pub fn retry_all_failed_download_jobs(
    profile_id: &str,
) -> Result<RetryAllFailedDownloadJobsReport, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let failed_total: usize = connection
        .query_row(
            "SELECT
                SUM(CASE WHEN state = ?1 THEN 1 ELSE 0 END)
             FROM sync_jobs
             WHERE profile_id = ?2
               AND direction = ?3",
            params![
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
            ],
            |row| Ok(row.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as usize),
        )
        .map_err(|error| format!("Failed counting failed download jobs: {error}"))?;
    let now = current_unix_seconds();
    let retried = connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 last_error = NULL,
                 next_retry_at = NULL,
                 bytes_done = 0,
                 finished_at = NULL,
                 updated_at = ?3,
                 progress_updated_at = ?3
             WHERE profile_id = ?4
               AND direction = ?5
                AND state = ?6",
            params![
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
            ],
        )
        .map_err(|error| format!("Failed retrying failed download jobs: {error}"))?;

    let already_retrying = failed_total.saturating_sub(retried);

    Ok(RetryAllFailedDownloadJobsReport {
        retried,
        skipped_permission_denied: 0,
        already_retrying,
    })
}

fn mark_download_job_retry_wait(
    profile_id: &str,
    job_id: i64,
    error_text: &str,
    delay: Duration,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let delay_seconds = delay.as_secs().clamp(1, i64::MAX as u64) as i64;
    let next_retry_at = now.saturating_add(delay_seconds);
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 lease_owner = NULL,
                 lease_until = NULL,
                 last_error = ?3,
                 next_retry_at = ?4,
                 updated_at = ?5,
                 finished_at = NULL,
                 progress_updated_at = ?5
             WHERE profile_id = ?6 AND direction = ?7 AND id = ?8",
            params![
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
                JOB_RUN_STATE_IDLE,
                error_text,
                next_retry_at,
                now,
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking download job retry wait: {error}"))?;
    Ok(())
}

fn reset_running_sync_jobs_for_pause(profile_id: &str) -> Result<usize, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = CASE
                     WHEN state = ?1 THEN ?2
                     ELSE state
                 END,
                 run_state = ?3,
                 lease_owner = NULL,
                 lease_until = NULL,
                 next_retry_at = NULL,
                 updated_at = ?4,
                 progress_updated_at = COALESCE(progress_updated_at, ?4)
             WHERE profile_id = ?5
               AND run_state IN (?6, ?7)",
            params![
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                JOB_RUN_STATE_RUNNING,
                JOB_RUN_STATE_CLAIMED,
            ],
        )
        .map_err(|error| format!("Failed resetting running sync jobs for pause: {error}"))
}

fn read_download_job_counters(profile_id: &str) -> Result<DownloadJobCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT
                SUM(CASE WHEN state != 'skipped' THEN 1 ELSE 0 END),
                SUM(CASE WHEN state = 'in_progress' AND run_state = 'running' THEN 1 ELSE 0 END),
                SUM(CASE WHEN state = 'retry_wait' THEN 1 ELSE 0 END),
                SUM(CASE WHEN state = 'done' THEN 1 ELSE 0 END),
                SUM(CASE WHEN state = 'failed_terminal' THEN 1 ELSE 0 END),
                SUM(CASE
                    WHEN state IN ('queued', 'retry_wait') THEN COALESCE(bytes_total, remote_size, 0)
                    WHEN state = 'in_progress' AND run_state = 'running' THEN CASE
                        WHEN COALESCE(bytes_total, remote_size, 0) > COALESCE(bytes_done, 0)
                            THEN COALESCE(bytes_total, remote_size, 0) - COALESCE(bytes_done, 0)
                        ELSE 0
                    END
                    ELSE 0
                END),
                SUM(CASE WHEN state != 'skipped' THEN COALESCE(bytes_total, remote_size, 0) ELSE 0 END),
                SUM(CASE WHEN state = 'done' THEN COALESCE(bytes_total, bytes_done, remote_size, 0) ELSE 0 END),
                SUM(CASE WHEN state = 'in_progress' AND run_state = 'running' THEN COALESCE(bytes_done, 0) ELSE 0 END)
             FROM sync_jobs
             WHERE profile_id = ?1 AND direction = ?2",
            params![profile_id, DOWNLOAD_JOB_DIRECTION],
            |row| {
                let planned_total = row.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as usize;
                let in_progress = row.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as usize;
                let retry_waiting = row.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as usize;
                let completed = row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as usize;
                let failed_terminal = row.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0) as usize;
                let remaining_bytes = row.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0) as u64;
                let planned_bytes = row.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u64;
                let completed_bytes = row.get::<_, Option<i64>>(7)?.unwrap_or(0).max(0) as u64;
                let in_flight_bytes_done = row.get::<_, Option<i64>>(8)?.unwrap_or(0).max(0) as u64;
                let remaining = planned_total
                    .saturating_sub(completed)
                    .saturating_sub(failed_terminal);
                Ok(DownloadJobCounters {
                    planned_total,
                    planned_bytes,
                    in_progress,
                    in_flight_bytes_done,
                    retry_waiting,
                    completed,
                    completed_bytes,
                    failed_terminal,
                    remaining,
                    remaining_bytes,
                })
            },
        )
        .map_err(|error| format!("Failed reading download counters: {error}"))
}
