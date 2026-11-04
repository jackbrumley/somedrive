fn begin_upload_job(
    profile_id: &str,
    relative_path: &str,
    size_bytes: u64,
    modified_ts: i64,
    lease_owner: &str,
) -> Result<i64, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let lease_until = now.saturating_add(300);
    let item_id = relative_path;

    let claimed_updated = connection
        .execute(
            "UPDATE sync_jobs
             SET path = ?1,
                 remote_size = ?2,
                 remote_modified_ts = ?3,
                 state = ?4,
                 run_state = ?5,
                 lease_owner = ?6,
                 lease_until = ?7,
                 bytes_done = 0,
                 bytes_total = ?8,
                 progress_updated_at = ?9,
                 finished_at = NULL,
                 updated_at = ?9
             WHERE profile_id = ?10
               AND direction = ?11
               AND item_id = ?12
               AND state = ?13
               AND run_state = ?14
               AND lease_owner = ?6",
            params![
                relative_path,
                size_bytes as i64,
                modified_ts,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                lease_owner,
                lease_until,
                size_bytes as i64,
                now,
                profile_id,
                UPLOAD_JOB_DIRECTION,
                item_id,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_CLAIMED,
            ],
        )
        .map_err(|error| format!("Failed transitioning claimed upload job to running: {error}"))?;

    if claimed_updated > 0 {
        let job_id = connection
            .query_row(
                "SELECT id FROM sync_jobs WHERE profile_id = ?1 AND direction = ?2 AND item_id = ?3",
                params![profile_id, UPLOAD_JOB_DIRECTION, item_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Failed fetching claimed upload job id: {error}"))?;
        return Ok(job_id);
    }

    let claimed_by_other: Option<String> = connection
        .query_row(
            "SELECT lease_owner
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND item_id = ?3
               AND state = ?4
               AND run_state = ?5
               AND lease_owner IS NOT NULL",
            params![
                profile_id,
                UPLOAD_JOB_DIRECTION,
                item_id,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_CLAIMED,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed reading claimed upload owner: {error}"))?;
    if let Some(owner) = claimed_by_other {
        if owner != lease_owner {
            return Err(format!(
                "Upload job '{}' is claimed by another cycle owner",
                relative_path
            ));
        }
    }

    connection
        .execute(
            "INSERT INTO sync_jobs (
                profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                state, run_state, attempt_count, last_error, next_retry_at, lease_owner, lease_until,
                bytes_done, bytes_total, progress_updated_at,
                created_at, updated_at, started_at, finished_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, 1, NULL, NULL, ?9, ?10,
                0, ?11, ?11,
                ?11, ?11, ?11, NULL
            )
            ON CONFLICT(profile_id, direction, item_id)
            DO UPDATE SET
                path = excluded.path,
                remote_size = excluded.remote_size,
                remote_modified_ts = excluded.remote_modified_ts,
                state = excluded.state,
                run_state = excluded.run_state,
                attempt_count = sync_jobs.attempt_count + 1,
                last_error = NULL,
                next_retry_at = NULL,
                lease_owner = excluded.lease_owner,
                lease_until = excluded.lease_until,
                bytes_done = 0,
                bytes_total = excluded.bytes_total,
                progress_updated_at = excluded.progress_updated_at,
                started_at = COALESCE(sync_jobs.started_at, excluded.started_at),
                finished_at = NULL,
                updated_at = excluded.updated_at",
            params![
                profile_id,
                UPLOAD_JOB_DIRECTION,
                item_id,
                relative_path,
                size_bytes as i64,
                modified_ts,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                lease_owner,
                lease_until,
                size_bytes as i64,
                now,
            ],
        )
        .map_err(|error| format!("Failed beginning upload job: {error}"))?;

    let job_id = connection
        .query_row(
            "SELECT id FROM sync_jobs WHERE profile_id = ?1 AND direction = ?2 AND item_id = ?3",
            params![profile_id, UPLOAD_JOB_DIRECTION, item_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed fetching upload job id: {error}"))?;
    Ok(job_id)
}

fn claim_upload_job_path(profile_id: &str, relative_path: &str, lease_owner: &str) -> Result<bool, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let lease_until = now.saturating_add(ACTION_JOB_LEASE_SECONDS);

    connection
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
                UPLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
            ],
        )
        .map_err(|error| format!("Failed recovering stale in-progress upload jobs: {error}"))?;

    let updated = connection
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
                 progress_updated_at = ?5
             WHERE profile_id = ?6
               AND direction = ?7
               AND item_id = ?8
               AND state IN (?9, ?10)
               AND (next_retry_at IS NULL OR next_retry_at <= ?5)",
            params![
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_CLAIMED,
                lease_owner,
                lease_until,
                now,
                profile_id,
                UPLOAD_JOB_DIRECTION,
                relative_path,
                DOWNLOAD_JOB_STATE_QUEUED,
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
            ],
        )
        .map_err(|error| format!("Failed claiming upload job path: {error}"))?;
    if updated > 0 {
        return Ok(true);
    }

    let claimed_by_owner: Option<i64> = connection
        .query_row(
            "SELECT id
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND item_id = ?3
               AND state = ?4
               AND run_state = ?5
               AND lease_owner = ?6",
            params![
                profile_id,
                UPLOAD_JOB_DIRECTION,
                relative_path,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_CLAIMED,
                lease_owner,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed reading claimed upload job path: {error}"))?;

    Ok(claimed_by_owner.is_some())
}

/// Queues an upload for `relative_path`, queuing a finished job again (see
/// `upsert_download_job`).
fn upsert_upload_job_queued(
    profile_id: &str,
    relative_path: &str,
    size_bytes: u64,
    modified_ts: i64,
) -> Result<bool, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let item_id = relative_path;

    let existing_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM sync_jobs WHERE profile_id = ?1 AND direction = ?2 AND item_id = ?3",
            params![profile_id, UPLOAD_JOB_DIRECTION, item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed querying existing upload job: {error}"))?;

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
                    0, ?9, ?10,
                    ?10, ?10, NULL, NULL
                )",
                params![
                    profile_id,
                    UPLOAD_JOB_DIRECTION,
                    item_id,
                    relative_path,
                    size_bytes as i64,
                    modified_ts,
                    DOWNLOAD_JOB_STATE_QUEUED,
                    JOB_RUN_STATE_IDLE,
                    size_bytes as i64,
                    now,
                ],
            )
            .map_err(|error| format!("Failed inserting queued upload job: {error}"))?;
        return Ok(true);
    }

    connection
        .execute(
            "UPDATE sync_jobs
             SET path = ?1,
                 remote_size = ?2,
                 remote_modified_ts = ?3,
                 state = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 'queued' ELSE state END,
                 run_state = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 'idle' ELSE run_state END,
                 attempt_count = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 0 ELSE attempt_count END,
                 last_error = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN NULL ELSE last_error END,
                 finished_at = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN NULL ELSE finished_at END,
                 bytes_total = ?4,
                 progress_updated_at = COALESCE(progress_updated_at, ?5),
                 updated_at = ?5
             WHERE profile_id = ?6
               AND direction = ?7
               AND item_id = ?8",
            params![
                relative_path,
                size_bytes as i64,
                modified_ts,
                size_bytes as i64,
                now,
                profile_id,
                UPLOAD_JOB_DIRECTION,
                item_id,
            ],
        )
        .map_err(|error| format!("Failed updating queued upload job metadata: {error}"))?;
    Ok(false)
}

fn mark_upload_job_done(profile_id: &str, job_id: i64) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
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
                DOWNLOAD_JOB_STATE_DONE,
                JOB_RUN_STATE_IDLE,
                now,
                profile_id,
                UPLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking upload job done: {error}"))?;
    Ok(())
}

fn mark_upload_job_failed(profile_id: &str, job_id: i64, error_text: &str) -> Result<(), String> {
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
                UPLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking upload job failed: {error}"))?;
    Ok(())
}

fn mark_upload_job_retry_wait(
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
                UPLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed marking upload job retry wait: {error}"))?;
    Ok(())
}

fn read_upload_job_attempt_count(profile_id: &str, job_id: i64) -> Result<u32, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT attempt_count
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND id = ?3",
            params![profile_id, UPLOAD_JOB_DIRECTION, job_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Failed reading upload job attempt count: {error}"))
        .map(|value| value.max(0) as u32)
}

fn update_upload_job_progress(
    profile_id: &str,
    job_id: i64,
    bytes_done: u64,
    bytes_total: Option<u64>,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "UPDATE sync_jobs
             SET state = ?1,
                 run_state = ?2,
                 bytes_done = ?3,
                 bytes_total = COALESCE(?4, bytes_total),
                 updated_at = ?5,
                 progress_updated_at = ?5
             WHERE profile_id = ?6 AND direction = ?7 AND id = ?8",
            params![
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                bytes_done as i64,
                bytes_total.map(|value| value as i64),
                now,
                profile_id,
                UPLOAD_JOB_DIRECTION,
                job_id,
            ],
        )
        .map_err(|error| format!("Failed updating upload job progress: {error}"))?;
    Ok(())
}

fn read_upload_job_counters(profile_id: &str) -> Result<UploadJobCounters, String> {
    query_upload_job_counters(&open_sync_jobs_connection(profile_id)?, profile_id)
}

/// The upload counters read on a caller-owned connection, so they can share
/// one snapshot with other reads (see `hydrate_runtime_status_from_db`).
fn query_upload_job_counters(connection: &Connection, profile_id: &str) -> Result<UploadJobCounters, String> {
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
            params![profile_id, UPLOAD_JOB_DIRECTION],
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
                Ok(UploadJobCounters {
                    planned_total,
                    planned_bytes,
                    in_progress,
                    in_flight_bytes_done,
                    retry_waiting,
                    completed,
                    completed_bytes,
                    failed_terminal,
                    remaining_bytes,
                })
            },
        )
        .map_err(|error| format!("Failed reading upload counters: {error}"))
}
