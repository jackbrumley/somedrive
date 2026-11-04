use super::*;

/// The plan's downloads as SQL: files the planner wants downloaded.
const PLANNED_DOWNLOADS_WHERE: &str = "profile_id = ?1
    AND desired_action = ?2
    AND is_dir = 0
    AND is_shared_reference = 0
    AND cloud_present = 1
    AND remote_item_id IS NOT NULL";

/// Makes the download queue match the plan in one short transaction: queues
/// new downloads, re-queues finished jobs the plan wants again, refreshes
/// changed metadata, and drops queued or retry-waiting jobs the plan no
/// longer wants. Running jobs are left alone. Only rows that differ are
/// written, so calling it after every change-feed page stays cheap however
/// long the queue is. Returns the number of planned downloads.
pub(crate) fn queue_planned_downloads(profile_id: &str) -> Result<usize, String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting download queue transaction: {error}"))?;
    let now = current_unix_seconds();

    transaction
        .execute(
            &format!(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at, lease_owner, lease_until,
                    bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                )
                SELECT profile_id, ?3, remote_item_id, path, cloud_size, cloud_mtime,
                       ?4, ?5, 0, NULL, NULL, NULL, NULL,
                       0, NULL, ?6,
                       ?6, ?6, NULL, NULL
                FROM sync_files
                WHERE {PLANNED_DOWNLOADS_WHERE}
                ON CONFLICT(profile_id, direction, item_id) DO UPDATE SET
                    path = excluded.path,
                    remote_size = excluded.remote_size,
                    remote_modified_ts = excluded.remote_modified_ts,
                    state = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 'queued' ELSE state END,
                    run_state = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 'idle' ELSE run_state END,
                    attempt_count = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN 0 ELSE attempt_count END,
                    last_error = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN NULL ELSE last_error END,
                    finished_at = CASE WHEN state IN ('done', 'failed_terminal', 'skipped') THEN NULL ELSE finished_at END,
                    bytes_total = CASE WHEN excluded.remote_size > 0 THEN excluded.remote_size ELSE bytes_total END,
                    progress_updated_at = COALESCE(progress_updated_at, excluded.updated_at),
                    updated_at = excluded.updated_at
                WHERE sync_jobs.path != excluded.path
                   OR sync_jobs.remote_size != excluded.remote_size
                   OR sync_jobs.remote_modified_ts != excluded.remote_modified_ts
                   OR sync_jobs.state IN ('done', 'failed_terminal', 'skipped')"
            ),
            params![
                profile_id,
                PLANNER_ACTION_DOWNLOAD,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
                now,
            ],
        )
        .map_err(|error| format!("Failed queueing planned downloads: {error}"))?;

    transaction
        .execute(
            "DELETE FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?3
               AND state IN (?4, ?5)
               AND NOT EXISTS (
                   SELECT 1 FROM sync_files AS planned
                   WHERE planned.profile_id = sync_jobs.profile_id
                     AND planned.remote_item_id = sync_jobs.item_id
                     AND planned.desired_action = ?2
                     AND planned.is_dir = 0
                     AND planned.is_shared_reference = 0
                     AND planned.cloud_present = 1
               )",
            params![
                profile_id,
                PLANNER_ACTION_DOWNLOAD,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_QUEUED,
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
            ],
        )
        .map_err(|error| format!("Failed dropping unplanned downloads: {error}"))?;

    let planned: i64 = transaction
        .query_row(
            &format!("SELECT COUNT(1) FROM sync_files WHERE {PLANNED_DOWNLOADS_WHERE}"),
            params![profile_id, PLANNER_ACTION_DOWNLOAD],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed counting planned downloads: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing download queue: {error}"))?;
    Ok(usize::try_from(planned).unwrap_or(0))
}

/// Claims up to `max_jobs` queued downloads, oldest first, in two lanes:
/// files of `LARGE_DOWNLOAD_BYTES` or more may hold at most
/// `large_file_slots` running downloads while smaller files are waiting, so
/// a few big videos cannot stall thousands of documents. Slots no small file
/// needs go to large files.
pub(crate) fn claim_download_jobs(
    profile_id: &str,
    lease_owner: &str,
    max_jobs: usize,
    large_file_slots: usize,
) -> Result<Vec<ClaimedDownloadJob>, String> {
    if max_jobs == 0 {
        return Ok(Vec::new());
    }

    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
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

    let large_in_flight: i64 = transaction
        .query_row(
            "SELECT COUNT(1) FROM sync_jobs
             WHERE profile_id = ?1 AND direction = ?2 AND state = ?3 AND remote_size >= ?4",
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                LARGE_DOWNLOAD_BYTES as i64,
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed counting running large downloads: {error}"))?;
    let large_allowance = large_file_slots
        .saturating_sub(usize::try_from(large_in_flight).unwrap_or(usize::MAX))
        .min(max_jobs);

    let mut jobs = select_claimable_downloads(
        &transaction,
        profile_id,
        now,
        DownloadLane::Large,
        large_allowance,
        0,
    )?;
    let small = select_claimable_downloads(
        &transaction,
        profile_id,
        now,
        DownloadLane::Small,
        max_jobs - jobs.len(),
        0,
    )?;
    let large_claimed = jobs.len();
    jobs.extend(small);
    let idle_slots = max_jobs - jobs.len();
    let spare_large = select_claimable_downloads(
        &transaction,
        profile_id,
        now,
        DownloadLane::Large,
        idle_slots,
        large_claimed,
    )?;
    jobs.extend(spare_large);

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

pub(crate) fn mark_download_job_done(
    profile_id: &str,
    job_id: i64,
    skipped: bool,
) -> Result<(), String> {
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

pub(crate) fn mark_download_job_running(profile_id: &str, job_id: i64) -> Result<(), String> {
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

pub(crate) fn update_download_job_progress(
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

pub(crate) fn mark_download_job_failed(
    profile_id: &str,
    job_id: i64,
    error_text: &str,
) -> Result<(), String> {
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

pub(crate) fn mark_download_job_retry_wait(
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

/// Records a failed attempt that the same worker is about to retry in place.
/// The job stays claimed (state and lease unchanged): releasing it here would
/// let the dispatcher hand the file to a second worker while this one still
/// retries it, and both would write the same temporary file.
pub(crate) fn record_download_job_retrying_in_place(
    profile_id: &str,
    job_id: i64,
    error_text: &str,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "UPDATE sync_jobs
             SET last_error = ?1,
                 bytes_done = 0,
                 updated_at = ?2,
                 progress_updated_at = ?2
             WHERE profile_id = ?3 AND direction = ?4 AND id = ?5",
            params![error_text, now, profile_id, DOWNLOAD_JOB_DIRECTION, job_id],
        )
        .map_err(|error| format!("Failed recording download retry: {error}"))?;
    Ok(())
}

pub(crate) fn reset_running_sync_jobs_for_pause(profile_id: &str) -> Result<usize, String> {
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

pub(crate) fn read_download_job_counters(profile_id: &str) -> Result<DownloadJobCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    query_download_job_counters(&connection, profile_id)
}

/// The download counters read on a caller-owned connection, so they can share
/// one snapshot with other reads (see `hydrate_runtime_status_from_db`).
pub(crate) fn query_download_job_counters(
    connection: &Connection,
    profile_id: &str,
) -> Result<DownloadJobCounters, String> {
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

#[derive(Clone, Copy)]
enum DownloadLane {
    Small,
    Large,
}

/// Queued downloads of one lane that may start now, oldest first, skipping
/// the first `skip` (already taken by this claim).
fn select_claimable_downloads(
    transaction: &rusqlite::Transaction<'_>,
    profile_id: &str,
    now: i64,
    lane: DownloadLane,
    limit: usize,
    skip: usize,
) -> Result<Vec<ClaimedDownloadJob>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let size_condition = match lane {
        DownloadLane::Small => "remote_size < ?6",
        DownloadLane::Large => "remote_size >= ?6",
    };
    let mut statement = transaction
        .prepare(&format!(
            "SELECT id, item_id, path
             FROM sync_jobs
             WHERE profile_id = ?1
               AND direction = ?2
               AND state IN (?3, ?4)
               AND (next_retry_at IS NULL OR next_retry_at <= ?5)
               AND {size_condition}
             ORDER BY created_at ASC, id ASC
             LIMIT ?7 OFFSET ?8"
        ))
        .map_err(|error| format!("Failed preparing download claim query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_QUEUED,
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
                now,
                LARGE_DOWNLOAD_BYTES as i64,
                limit as i64,
                skip as i64,
            ],
            |row| {
                Ok(ClaimedDownloadJob {
                    job_id: row.get(0)?,
                    item_id: row.get(1)?,
                    path: row.get(2)?,
                })
            },
        )
        .map_err(|error| format!("Failed executing download claim query: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("Failed reading claimed download job: {error}")))
        .collect()
}
