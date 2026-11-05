use super::*;

/// The Activity feed shows files finished (or failed) within this many
/// seconds, so it empties by itself a while after the sync has caught up.
pub(crate) const RECENTLY_SYNCED_WINDOW_SECONDS: i64 = 10 * 60;

pub(crate) fn read_sync_job_activity_projection(
    connection: &Connection,
    profile_id: &str,
    active_limit: usize,
    recent_limit: usize,
) -> Result<SyncJobActivityProjection, String> {
    let mut projection = SyncJobActivityProjection::default();

    // Running transfers sort first so the row limit can only cut queued
    // rows; otherwise recently requeued jobs push running ones out of the
    // projection and its count drifts from the in-flight counter.
    let mut active_statement = connection
        .prepare(
            "SELECT id,
                    state,
                    run_state,
                    direction,
                    path,
                    bytes_done,
                    bytes_total,
                    COALESCE(started_at, updated_at),
                    COALESCE(progress_updated_at, updated_at)
             FROM sync_jobs
             WHERE profile_id = ?1
                AND (
                    (state = ?2 AND run_state IN (?3, ?4))
                    OR state = ?5
                )
              ORDER BY (state = ?2 AND run_state = ?3) DESC,
                       COALESCE(progress_updated_at, updated_at) DESC
              LIMIT ?6",
        )
        .map_err(|error| format!("Failed preparing active sync job query: {error}"))?;
    let active_rows = active_statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_STATE_IN_PROGRESS,
                JOB_RUN_STATE_RUNNING,
                JOB_RUN_STATE_CLAIMED,
                DOWNLOAD_JOB_STATE_QUEUED,
                active_limit as i64,
            ],
            |row| {
                let job_id: i64 = row.get(0)?;
                let state: String = row.get(1)?;
                let run_state: String = row.get(2)?;
                let direction: String = row.get(3)?;
                let path: String = row.get(4)?;
                let bytes_done = row.get::<_, i64>(5)?.max(0) as u64;
                let bytes_total = row
                    .get::<_, Option<i64>>(6)?
                    .map(|value| value.max(0) as u64);
                let started_at_unix: i64 = row.get(7)?;
                let updated_at_unix: i64 = row.get(8)?;
                let transfer_state = if state == DOWNLOAD_JOB_STATE_IN_PROGRESS
                    && run_state == JOB_RUN_STATE_RUNNING
                {
                    "in_progress"
                } else {
                    "queued"
                };
                Ok((
                    SyncRuntimeTransfer {
                        id: format!("job-{job_id}"),
                        direction,
                        path,
                        state: transfer_state.to_string(),
                        bytes_done,
                        bytes_total,
                        started_at: unix_seconds_to_rfc3339(started_at_unix),
                        updated_at: unix_seconds_to_rfc3339(updated_at_unix),
                    },
                    job_id,
                ))
            },
        )
        .map_err(|error| format!("Failed querying active sync jobs: {error}"))?;
    for row in active_rows {
        let (transfer, _) =
            row.map_err(|error| format!("Failed reading active sync job row: {error}"))?;
        if transfer
            .direction
            .eq_ignore_ascii_case(DOWNLOAD_JOB_DIRECTION)
            && transfer.state == "in_progress"
        {
            projection.active_download_count += 1;
        } else if transfer
            .direction
            .eq_ignore_ascii_case(UPLOAD_JOB_DIRECTION)
            && transfer.state == "in_progress"
        {
            projection.active_upload_count += 1;
        }
        projection.active.push(transfer);
    }

    let mut completed_statement = connection
        .prepare(
            "SELECT id,
                    direction,
                    path,
                    bytes_total,
                    finished_at
             FROM sync_jobs
             WHERE profile_id = ?1
               AND state IN (?2, ?3)
               AND finished_at IS NOT NULL
               AND finished_at >= ?5
             ORDER BY finished_at DESC
             LIMIT ?4",
        )
        .map_err(|error| format!("Failed preparing completed sync job query: {error}"))?;
    let completed_rows = completed_statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_STATE_DONE,
                DOWNLOAD_JOB_STATE_SKIPPED,
                recent_limit as i64,
                current_unix_seconds() - RECENTLY_SYNCED_WINDOW_SECONDS,
            ],
            |row| {
                let job_id: i64 = row.get(0)?;
                let direction: String = row.get(1)?;
                let path: String = row.get(2)?;
                let bytes_total = row
                    .get::<_, Option<i64>>(3)?
                    .map(|value| value.max(0) as u64);
                let finished_at_unix: i64 = row.get(4)?;
                Ok(SyncRuntimeRecentItem {
                    id: format!("job-{job_id}"),
                    direction,
                    path,
                    bytes_total,
                    finished_at: unix_seconds_to_rfc3339(finished_at_unix),
                    status: "completed".to_string(),
                    error: None,
                })
            },
        )
        .map_err(|error| format!("Failed querying completed sync jobs: {error}"))?;
    for row in completed_rows {
        projection
            .recent_completed
            .push(row.map_err(|error| format!("Failed reading completed sync job row: {error}"))?);
    }

    let mut failed_statement = connection
        .prepare(
            "SELECT id,
                    direction,
                    path,
                    bytes_total,
                    finished_at,
                    last_error
             FROM sync_jobs
             WHERE profile_id = ?1
               AND state = ?2
               AND finished_at IS NOT NULL
               AND finished_at >= ?4
             ORDER BY finished_at DESC
             LIMIT ?3",
        )
        .map_err(|error| format!("Failed preparing failed sync job query: {error}"))?;
    let failed_rows = failed_statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL,
                recent_limit as i64,
                current_unix_seconds() - RECENTLY_SYNCED_WINDOW_SECONDS,
            ],
            |row| {
                let job_id: i64 = row.get(0)?;
                let direction: String = row.get(1)?;
                let path: String = row.get(2)?;
                let bytes_total = row
                    .get::<_, Option<i64>>(3)?
                    .map(|value| value.max(0) as u64);
                let finished_at_unix: i64 = row.get(4)?;
                let error_text: Option<String> = row.get(5)?;
                Ok(SyncRuntimeRecentItem {
                    id: format!("job-{job_id}"),
                    direction,
                    path,
                    bytes_total,
                    finished_at: unix_seconds_to_rfc3339(finished_at_unix),
                    status: "failed".to_string(),
                    error: error_text,
                })
            },
        )
        .map_err(|error| format!("Failed querying failed sync jobs: {error}"))?;
    for row in failed_rows {
        projection
            .recent_failed
            .push(row.map_err(|error| format!("Failed reading failed sync job row: {error}"))?);
    }

    let mut retry_wait_statement = connection
        .prepare(
            "SELECT id,
                    direction,
                    path,
                    bytes_total,
                    COALESCE(next_retry_at, updated_at),
                    last_error
             FROM sync_jobs
             WHERE profile_id = ?1
               AND state = ?2
             ORDER BY COALESCE(next_retry_at, updated_at) ASC
             LIMIT ?3",
        )
        .map_err(|error| format!("Failed preparing retry-wait sync job query: {error}"))?;
    let retry_wait_rows = retry_wait_statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_STATE_RETRY_WAIT,
                recent_limit as i64
            ],
            |row| {
                let job_id: i64 = row.get(0)?;
                let direction: String = row.get(1)?;
                let path: String = row.get(2)?;
                let bytes_total = row
                    .get::<_, Option<i64>>(3)?
                    .map(|value| value.max(0) as u64);
                let retry_at_unix: i64 = row.get(4)?;
                let error_text: Option<String> = row.get(5)?;
                Ok(SyncRuntimeRecentItem {
                    id: format!("job-{job_id}"),
                    direction,
                    path,
                    bytes_total,
                    finished_at: unix_seconds_to_rfc3339(retry_at_unix),
                    status: "retry_waiting".to_string(),
                    error: error_text,
                })
            },
        )
        .map_err(|error| format!("Failed querying retry-wait sync jobs: {error}"))?;
    for row in retry_wait_rows {
        projection
            .recent_retry_waiting
            .push(row.map_err(|error| format!("Failed reading retry-wait sync job row: {error}"))?);
    }

    Ok(projection)
}

pub fn hydrate_runtime_status_from_db(
    status: &mut sync_runtime::SyncRuntimeAccountStatus,
) -> Result<(), String> {
    const ACTIVE_ACTIVITY_LIMIT: usize = 512;
    const RECENT_ACTIVITY_LIMIT: usize = 120;
    let profile_id = status.profile_id.clone();
    // One read transaction: the activity rows and the in-flight counters come
    // from the same WAL snapshot, so workers claiming jobs between the reads
    // cannot make them disagree.
    let connection = open_sync_jobs_connection(&profile_id)?;
    let snapshot = connection
        .unchecked_transaction()
        .map_err(|error| format!("Failed starting sync activity snapshot: {error}"))?;
    let projection = read_sync_job_activity_projection(
        &snapshot,
        &profile_id,
        ACTIVE_ACTIVITY_LIMIT,
        RECENT_ACTIVITY_LIMIT,
    )?;
    let download_counters = query_download_job_counters(&snapshot, &profile_id)?;
    let upload_counters = query_upload_job_counters(&snapshot, &profile_id)?;
    status.data_revisions = read_data_revisions(&snapshot)?;
    status.local_changes_watched = read_local_watch_state(&snapshot)?;
    drop(snapshot);
    let planner_counters = read_sync_file_planner_counters(&profile_id)?;

    if projection.active_download_count != download_counters.in_progress {
        log::warn!(
            "{} SYNC_ACTIVITY_INVARIANT_MISMATCH lane=download running_rows={} counter_in_flight={}",
            log_context::account_prefix(&profile_id),
            projection.active_download_count,
            download_counters.in_progress
        );
    }
    if projection.active_upload_count != upload_counters.in_progress {
        log::warn!(
            "{} SYNC_ACTIVITY_INVARIANT_MISMATCH lane=upload running_rows={} counter_in_flight={}",
            log_context::account_prefix(&profile_id),
            projection.active_upload_count,
            upload_counters.in_progress
        );
    }

    status.in_progress = projection.active;
    status.recent_completed = projection.recent_completed;
    status.recent_retry_waiting = projection.recent_retry_waiting;
    status.recent_failed = projection.recent_failed;

    status.planner_cloud_discovered_total = planner_counters.cloud_discovered_total;
    status.planner_local_discovered_total = planner_counters.local_discovered_total;
    status.planner_none_total = planner_counters.none_total;
    status.planner_need_download_total = planner_counters.need_download_total;
    status.planner_need_upload_total = planner_counters.need_upload_total;
    status.planner_need_delete_remote_total = planner_counters.need_delete_remote_total;
    status.planner_need_delete_local_total = planner_counters.need_delete_local_total;
    status.planner_conflict_total = planner_counters.conflict_total;

    status.remote_download_planned_total = download_counters.planned_total;
    status.remote_download_completed_total = download_counters.completed;
    status.remote_download_failed_total = download_counters.failed_terminal;
    status.remote_download_in_flight = download_counters.in_progress;
    status.remote_download_retry_waiting = download_counters.retry_waiting;
    status.remote_download_planned_bytes_total = download_counters.planned_bytes;
    status.remote_download_completed_bytes_total = download_counters.completed_bytes;
    status.remote_download_remaining_bytes_total = download_counters.remaining_bytes;
    status.remote_download_in_flight_bytes_done = download_counters.in_flight_bytes_done;

    status.upload_planned_total = upload_counters.planned_total;
    status.upload_completed_total = upload_counters.completed;
    status.upload_failed_total = upload_counters.failed_terminal;
    status.upload_in_flight = upload_counters.in_progress;
    status.upload_retry_waiting = upload_counters.retry_waiting;
    status.upload_planned_bytes_total = upload_counters.planned_bytes;
    status.upload_completed_bytes_total = upload_counters.completed_bytes;
    status.upload_remaining_bytes_total = upload_counters.remaining_bytes;
    status.upload_in_flight_bytes_done = upload_counters.in_flight_bytes_done;

    let throttle = read_throttle_counters(&profile_id)?;
    status.remote_download_throttle_total = throttle.download_total;
    status.remote_download_throttle_last_minute = throttle.download_last_minute;
    status.upload_throttle_total = throttle.upload_total;
    status.upload_throttle_last_minute = throttle.upload_last_minute;

    if let Some(issue) = read_persisted_sync_issue(&profile_id)? {
        status.issue_code = Some(issue.issue_code);
        status.issue_message = Some(issue.issue_message);
        status.issue_actions = issue.issue_actions;
        status.issue_path = issue.issue_path;
        status.issue_secondary_path = issue.issue_secondary_path;
    } else {
        status.issue_code = None;
        status.issue_message = None;
        status.issue_actions.clear();
        status.issue_path = None;
        status.issue_secondary_path = None;
    }

    let lifecycle = read_sync_lifecycle_row(&profile_id)?.ok_or_else(|| {
        format!(
            "Missing sync lifecycle state row for profile '{}'.",
            profile_id
        )
    })?;
    status.phase = lifecycle.phase;
    status.phase_message = lifecycle.phase_message;
    status.remote_scan_complete = lifecycle.remote_scan_complete;
    status.last_sync_at = lifecycle.last_sync_at.clone();
    if lifecycle.activity_stage.trim().is_empty() {
        return Err(format!(
            "Missing lifecycle activity stage for profile '{}'.",
            profile_id
        ));
    }
    if lifecycle.activity_progress_mode.trim().is_empty() {
        return Err(format!(
            "Missing lifecycle activity progress mode for profile '{}'.",
            profile_id
        ));
    }
    status.current_activity.stage = lifecycle.activity_stage;
    status.current_activity.progress_mode = lifecycle.activity_progress_mode;
    status.current_activity.current = lifecycle.activity_current;
    status.current_activity.total = lifecycle.activity_total;
    status.current_activity.unit = lifecycle.activity_unit;
    status.current_activity.detail = lifecycle.activity_detail;
    status.current_activity.cycle_id = lifecycle.activity_cycle_id;
    status.current_activity.updated_at = unix_seconds_to_rfc3339(lifecycle.activity_updated_at);
    if matches!(status.phase.as_str(), "paused" | "idle" | "error")
        && status.current_activity.progress_mode != "hidden"
    {
        return Err(format!(
            "Invalid lifecycle activity state for profile '{}': phase='{}' requires hidden progress mode, found '{}'.",
            profile_id, status.phase, status.current_activity.progress_mode
        ));
    }
    if status.current_activity.stage.trim().is_empty() {
        return Err(format!(
            "Invalid lifecycle activity state for profile '{}': stage cannot be empty.",
            profile_id
        ));
    }
    if status.current_activity.stage == "scanning_local" {
        let scanned_count = status.current_activity.current.ok_or_else(|| {
            format!(
                "Missing lifecycle local scan current counter for profile '{}'.",
                profile_id
            )
        })?;
        status.local_scan_scanned_count = scanned_count;
        status.local_scan_estimated_total = status.current_activity.total;
        status.local_scan_current_path = status.current_activity.detail.clone();
    } else {
        status.local_scan_scanned_count = 0;
        status.local_scan_estimated_total = None;
        status.local_scan_current_path = None;
    }
    status.engine_state = if status.phase == "error" {
        "blocked".to_string()
    } else if lifecycle.agent_state == "syncing" {
        "running".to_string()
    } else {
        "paused".to_string()
    };

    Ok(())
}

pub fn build_authoritative_runtime_status(
    profile_id: &str,
    auth_ready: bool,
) -> Result<sync_runtime::SyncRuntimeAccountStatus, String> {
    let mut status = sync_runtime::SyncRuntimeAccountStatus::canonical_seed(profile_id, auth_ready);
    hydrate_runtime_status_from_db(&mut status)?;
    sync_runtime::recompute_authority_fields(&mut status);
    Ok(status)
}
