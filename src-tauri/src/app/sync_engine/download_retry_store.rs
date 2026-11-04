use super::*;

// Retrying downloads that failed for good (from the Sync problems tab).

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
