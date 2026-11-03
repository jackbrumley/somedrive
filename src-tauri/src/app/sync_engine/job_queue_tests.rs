#[cfg(test)]
mod job_queue_tests {
    use super::*;

    fn test_profile_id(label: &str) -> String {
        format!(
            "job-queue-test-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
    }

    fn clear_profile_rows(profile_id: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        connection
            .execute("DELETE FROM sync_jobs WHERE profile_id = ?1", params![profile_id])
            .expect("clear sync_jobs");
        connection
            .execute("DELETE FROM sync_files WHERE profile_id = ?1", params![profile_id])
            .expect("clear sync_files");
        connection
            .execute(
                "DELETE FROM sync_lifecycle_state WHERE profile_id = ?1",
                params![profile_id],
            )
            .expect("clear sync_lifecycle_state");
    }

    fn insert_running_job(profile_id: &str, direction: &str, item_id: &str, path: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at,
                    lease_owner, lease_until, bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, 1, 1,
                    'in_progress', 'running', 1, NULL, NULL,
                    'test-lease', ?5, 0, 1, ?5,
                    ?5, ?5, ?5, NULL
                )",
                params![profile_id, direction, item_id, path, now],
            )
            .expect("insert running sync job");
    }

    fn insert_failed_download_job(profile_id: &str, item_id: &str, path: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at,
                    lease_owner, lease_until, bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                ) VALUES (
                    ?1, 'download', ?2, ?3, 10, 10,
                    'failed_terminal', 'idle', 2, 'network', NULL,
                    NULL, NULL, 3, 10, ?4,
                    ?4, ?4, ?4, ?4
                )",
                params![profile_id, item_id, path, now],
            )
            .expect("insert failed download job");
    }

    fn insert_download_job_with_state(
        profile_id: &str,
        item_id: &str,
        path: &str,
        state: &str,
        run_state: &str,
        next_retry_at: Option<i64>,
    ) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at,
                    lease_owner, lease_until, bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                ) VALUES (
                    ?1, 'download', ?2, ?3, 10, 10,
                    ?4, ?5, 1, NULL, ?6,
                    NULL, NULL, 0, 10, ?7,
                    ?7, ?7, NULL, NULL
                )",
                params![profile_id, item_id, path, state, run_state, next_retry_at, now],
            )
            .expect("insert download job with state");
    }

    // a test fixture that writes one sync_jobs row column by column.
    #[allow(clippy::too_many_arguments)]
    fn insert_job_with_state_and_lease(
        profile_id: &str,
        direction: &str,
        item_id: &str,
        state: &str,
        run_state: &str,
        next_retry_at: Option<i64>,
        lease_owner: Option<&str>,
        lease_until: Option<i64>,
    ) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, last_error, next_retry_at,
                    lease_owner, lease_until, bytes_done, bytes_total, progress_updated_at,
                    created_at, updated_at, started_at, finished_at
                ) VALUES (
                    ?1, ?2, ?3, ?3, 10, 10,
                    ?4, ?5, 1, NULL, ?6,
                    ?7, ?8, 0, 10, ?9,
                    ?9, ?9, NULL, NULL
                )",
                params![
                    profile_id,
                    direction,
                    item_id,
                    state,
                    run_state,
                    next_retry_at,
                    lease_owner,
                    lease_until,
                    now
                ],
            )
            .expect("insert job with state and lease");
    }

    #[test]
    fn reset_running_jobs_for_pause_requeues_all_directions() {
        let profile_id = test_profile_id("pause-reset");
        clear_profile_rows(&profile_id);
        insert_running_job(&profile_id, DOWNLOAD_JOB_DIRECTION, "dl-1", "download.txt");
        insert_running_job(&profile_id, UPLOAD_JOB_DIRECTION, "up-1", "upload.txt");

        let reset_count = reset_running_sync_jobs_for_pause(&profile_id).expect("reset running jobs");
        assert_eq!(reset_count, 2);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let mut statement = connection
            .prepare(
                "SELECT direction, state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                 ORDER BY direction ASC",
            )
            .expect("prepare job state query");
        let rows = statement
            .query_map(params![&profile_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query job state rows");

        let collected: Vec<(String, String, String)> =
            rows.map(|row| row.expect("read job state row")).collect();
        assert_eq!(collected.len(), 2);
        for (_direction, state, run_state) in collected {
            assert_eq!(state, DOWNLOAD_JOB_STATE_QUEUED);
            assert_eq!(run_state, JOB_RUN_STATE_IDLE);
        }
    }

    #[test]
    fn read_sync_authority_row_counts_reports_table_counts() {
        let profile_id = test_profile_id("row-counts");
        clear_profile_rows(&profile_id);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, is_shared_reference,
                    remote_item_id, remote_present, local_present,
                    remote_size, local_size, remote_modified_ts, local_modified_ts,
                    desired_action, conflict_state, updated_at
                ) VALUES (
                    ?1, 'file.txt', 0, 0,
                    'remote-id', 1, 1,
                    10, 10, 10, 10,
                    'none', NULL, ?2
                )",
                params![&profile_id, now],
            )
            .expect("insert sync_files row");
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, created_at, updated_at
                ) VALUES (?1, 'download', 'job-id', 'file.txt', 10, 10, 'queued', 'idle', 0, ?2, ?2)",
                params![&profile_id, now],
            )
            .expect("insert sync_jobs row");
        persist_sync_lifecycle_phase(&profile_id, "idle", "Idle").expect("persist lifecycle row");

        let (lifecycle_rows, planner_rows, job_rows) =
            read_sync_authority_row_counts(&profile_id).expect("read authority row counts");
        assert_eq!(lifecycle_rows, 1);
        assert_eq!(planner_rows, 1);
        assert_eq!(job_rows, 1);
    }

    #[test]
    fn claim_action_job_paths_marks_jobs_claimed() {
        let profile_id = test_profile_id("claim-action-jobs");
        clear_profile_rows(&profile_id);

        upsert_action_job_queued(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/delete-local.txt",
        )
        .expect("queue action job");
        let claimed = claim_action_job_paths(&profile_id, DELETE_LOCAL_JOB_DIRECTION, "cycle-test")
            .expect("claim action job paths");

        assert!(claimed.contains("docs/delete-local.txt"));

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let (state, run_state): (String, String) = connection
            .query_row(
                "SELECT state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = ?2
                   AND item_id = ?3",
                params![
                    &profile_id,
                    DELETE_LOCAL_JOB_DIRECTION,
                    "docs/delete-local.txt"
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read claimed action job state");
        assert_eq!(state, DOWNLOAD_JOB_STATE_IN_PROGRESS);
        assert_eq!(run_state, JOB_RUN_STATE_CLAIMED);
    }

    #[test]
    fn claim_upload_job_path_transitions_to_claimed_and_running() {
        let profile_id = test_profile_id("claim-upload-job");
        clear_profile_rows(&profile_id);

        upsert_upload_job_queued(&profile_id, "docs/upload.txt", 42, 100).expect("queue upload job");
        let claimed = claim_upload_job_path(&profile_id, "docs/upload.txt", "cycle-test")
            .expect("claim upload job path");
        assert!(claimed);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let (claimed_state, claimed_run_state): (String, String) = connection
            .query_row(
                "SELECT state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = ?2
                   AND item_id = ?3",
                params![&profile_id, UPLOAD_JOB_DIRECTION, "docs/upload.txt"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read claimed upload job state");
        assert_eq!(claimed_state, DOWNLOAD_JOB_STATE_IN_PROGRESS);
        assert_eq!(claimed_run_state, JOB_RUN_STATE_CLAIMED);

        let job_id = begin_upload_job(&profile_id, "docs/upload.txt", 42, 100, "cycle-test")
            .expect("begin upload job for claimed path");
        assert!(job_id > 0);

        let (running_state, running_run_state): (String, String) = connection
            .query_row(
                "SELECT state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = ?2
                   AND item_id = ?3",
                params![&profile_id, UPLOAD_JOB_DIRECTION, "docs/upload.txt"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read running upload job state");
        assert_eq!(running_state, DOWNLOAD_JOB_STATE_IN_PROGRESS);
        assert_eq!(running_run_state, JOB_RUN_STATE_RUNNING);
    }

    #[test]
    fn lifecycle_operational_state_round_trips_through_db() {
        let profile_id = test_profile_id("lifecycle-operational");
        clear_profile_rows(&profile_id);

        let expected = SyncLifecycleOperationalState {
            two_way_ready: true,
            bootstrap_scan_initialized: true,
            bootstrap_full_scan_completed: true,
            delta_link: Some("https://example.test/delta".to_string()),
            active_delta_next_link: Some("https://example.test/next".to_string()),
            last_cycle_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        };

        persist_sync_lifecycle_operational_state(&profile_id, &expected)
            .expect("persist lifecycle operational state");
        let actual = read_sync_lifecycle_operational_state(&profile_id)
            .expect("read lifecycle operational state");

        assert_eq!(actual.two_way_ready, expected.two_way_ready);
        assert_eq!(
            actual.bootstrap_scan_initialized,
            expected.bootstrap_scan_initialized
        );
        assert_eq!(
            actual.bootstrap_full_scan_completed,
            expected.bootstrap_full_scan_completed
        );
        assert_eq!(actual.delta_link, expected.delta_link);
        assert_eq!(
            actual.active_delta_next_link,
            expected.active_delta_next_link
        );
        assert_eq!(actual.last_cycle_at, expected.last_cycle_at);
    }

    #[test]
    fn retry_all_failed_download_jobs_requeues_terminal_failures() {
        let profile_id = test_profile_id("retry-all-failed");
        clear_profile_rows(&profile_id);
        insert_failed_download_job(&profile_id, "failed-1", "docs/failed-1.txt");
        insert_failed_download_job(&profile_id, "failed-2", "docs/failed-2.txt");

        let report =
            retry_all_failed_download_jobs(&profile_id).expect("retry all failed download jobs");
        assert_eq!(report.retried, 2);
        assert_eq!(report.already_retrying, 0);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let queued_count: i64 = connection
            .query_row(
                "SELECT COUNT(1)
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = 'download'
                   AND state = 'queued'
                   AND run_state = 'idle'",
                params![&profile_id],
                |row| row.get(0),
            )
            .expect("count retried queued jobs");
        assert_eq!(queued_count, 2);
    }

    #[test]
    fn bootstrap_gate_transitions_from_blocked_to_ready_after_retry_and_completion() {
        let profile_id = test_profile_id("bootstrap-retry-ready");
        clear_profile_rows(&profile_id);

        let lifecycle = SyncLifecycleOperationalState {
            two_way_ready: false,
            bootstrap_scan_initialized: true,
            bootstrap_full_scan_completed: true,
            delta_link: Some("https://example.test/delta".to_string()),
            active_delta_next_link: None,
            last_cycle_at: None,
        };
        persist_sync_lifecycle_operational_state(&profile_id, &lifecycle)
            .expect("persist lifecycle gate state");

        insert_failed_download_job(&profile_id, "failed-bootstrap", "docs/failed-bootstrap.txt");

        let lifecycle = read_sync_lifecycle_operational_state(&profile_id)
            .expect("read lifecycle gate state");
        assert!(!lifecycle.two_way_ready);
        let blocked_counters =
            read_download_job_counters(&profile_id).expect("read blocked download counters");
        assert!(!bootstrap_ready_for_two_way(&lifecycle, &blocked_counters));

        let retry_report = retry_all_failed_download_jobs(&profile_id)
            .expect("retry failed download jobs for bootstrap gate");
        assert_eq!(retry_report.retried, 1);

        let queued_counters =
            read_download_job_counters(&profile_id).expect("read queued download counters");
        assert!(!bootstrap_ready_for_two_way(&lifecycle, &queued_counters));

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let retried_job_id: i64 = connection
            .query_row(
                "SELECT id
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = 'download'
                   AND item_id = 'failed-bootstrap'",
                params![&profile_id],
                |row| row.get(0),
            )
            .expect("read retried download job id");
        mark_download_job_done(&profile_id, retried_job_id, false)
            .expect("mark retried bootstrap job done");

        let ready_counters =
            read_download_job_counters(&profile_id).expect("read ready download counters");
        assert!(bootstrap_ready_for_two_way(&lifecycle, &ready_counters));

        let mut transitioned_lifecycle = lifecycle;
        transitioned_lifecycle.two_way_ready = true;
        persist_sync_lifecycle_operational_state(&profile_id, &transitioned_lifecycle)
            .expect("persist two-way-ready lifecycle state");
        let ready_lifecycle = read_sync_lifecycle_operational_state(&profile_id)
            .expect("read two-way-ready lifecycle state");
        assert!(ready_lifecycle.two_way_ready);
    }

    #[test]
    fn multi_cycle_download_claims_progress_queued_and_retry_wait_jobs() {
        let profile_id = test_profile_id("multi-cycle-claim-progression");
        clear_profile_rows(&profile_id);

        let now = current_unix_seconds();
        insert_download_job_with_state(
            &profile_id,
            "queued-1",
            "docs/queued-1.txt",
            DOWNLOAD_JOB_STATE_QUEUED,
            JOB_RUN_STATE_IDLE,
            None,
        );
        insert_download_job_with_state(
            &profile_id,
            "retry-due-now",
            "docs/retry-due-now.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            Some(now.saturating_sub(1)),
        );
        insert_download_job_with_state(
            &profile_id,
            "retry-future",
            "docs/retry-future.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            Some(now.saturating_add(3600)),
        );

        let first_cycle_claimed =
            claim_download_jobs(&profile_id, "cycle-1", 10).expect("claim first cycle jobs");
        assert_eq!(first_cycle_claimed.len(), 2);

        for job in &first_cycle_claimed {
            mark_download_job_done(&profile_id, job.job_id, false)
                .expect("mark first-cycle claimed job done");
        }

        let after_first_cycle =
            read_download_job_counters(&profile_id).expect("read counters after first cycle");
        assert_eq!(after_first_cycle.completed, 2);
        assert_eq!(after_first_cycle.remaining, 1);
        assert_eq!(after_first_cycle.retry_waiting, 1);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        connection
            .execute(
                "UPDATE sync_jobs
                 SET next_retry_at = ?1,
                     updated_at = ?1,
                     progress_updated_at = ?1
                 WHERE profile_id = ?2
                   AND direction = 'download'
                   AND item_id = 'retry-future'",
                params![current_unix_seconds().saturating_sub(1), &profile_id],
            )
            .expect("expire retry-future job for second cycle");

        let second_cycle_claimed =
            claim_download_jobs(&profile_id, "cycle-2", 10).expect("claim second cycle jobs");
        assert_eq!(second_cycle_claimed.len(), 1);
        assert_eq!(second_cycle_claimed[0].item_id, "retry-future");

        mark_download_job_done(&profile_id, second_cycle_claimed[0].job_id, false)
            .expect("mark second-cycle claimed job done");

        let after_second_cycle =
            read_download_job_counters(&profile_id).expect("read counters after second cycle");
        assert_eq!(after_second_cycle.completed, 3);
        assert_eq!(after_second_cycle.remaining, 0);
        assert_eq!(after_second_cycle.retry_waiting, 0);
    }

    #[test]
    fn lifecycle_phase_flow_tracks_bootstrap_blocked_retry_and_two_way_ready() {
        let profile_id = test_profile_id("lifecycle-bootstrap-phase-flow");
        clear_profile_rows(&profile_id);

        let lifecycle = SyncLifecycleOperationalState {
            two_way_ready: false,
            bootstrap_scan_initialized: true,
            bootstrap_full_scan_completed: true,
            delta_link: Some("https://example.test/delta".to_string()),
            active_delta_next_link: None,
            last_cycle_at: None,
        };
        persist_sync_lifecycle_operational_state(&profile_id, &lifecycle)
            .expect("persist initial bootstrap lifecycle state");

        persist_sync_lifecycle_phase(
            &profile_id,
            "syncing",
            "Initial sync in progress - downloading cloud files only",
        )
        .expect("persist blocked syncing phase");

        let blocked_row = read_sync_lifecycle_row(&profile_id)
            .expect("read blocked lifecycle row")
            .expect("blocked lifecycle row exists");
        assert_eq!(blocked_row.phase, "syncing");
        assert_eq!(
            blocked_row.phase_message,
            "Initial sync in progress - downloading cloud files only"
        );
        assert_eq!(blocked_row.activity_stage, "syncing");
        assert_eq!(blocked_row.activity_progress_mode, "indeterminate");
        assert!(!blocked_row.two_way_ready);

        insert_failed_download_job(&profile_id, "phase-flow-failed", "docs/phase-flow-failed.txt");

        let blocked_counters =
            read_download_job_counters(&profile_id).expect("read blocked counters");
        assert!(!bootstrap_ready_for_two_way(&lifecycle, &blocked_counters));

        let retry_report = retry_all_failed_download_jobs(&profile_id)
            .expect("retry failed terminal jobs in phase flow");
        assert_eq!(retry_report.retried, 1);

        let queued_counters = read_download_job_counters(&profile_id).expect("read queued counters");
        assert!(!bootstrap_ready_for_two_way(&lifecycle, &queued_counters));

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let retried_job_id: i64 = connection
            .query_row(
                "SELECT id
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = 'download'
                   AND item_id = 'phase-flow-failed'",
                params![&profile_id],
                |row| row.get(0),
            )
            .expect("read retried job id");
        mark_download_job_done(&profile_id, retried_job_id, false)
            .expect("mark retried phase-flow job done");

        let ready_counters = read_download_job_counters(&profile_id).expect("read ready counters");
        assert!(bootstrap_ready_for_two_way(&lifecycle, &ready_counters));

        persist_sync_lifecycle_phase(
            &profile_id,
            "preparing_two_way_baseline",
            "Preparing two-way sync - building your local baseline",
        )
        .expect("persist preparing baseline phase");

        let mut ready_lifecycle = lifecycle;
        ready_lifecycle.two_way_ready = true;
        persist_sync_lifecycle_operational_state(&profile_id, &ready_lifecycle)
            .expect("persist two-way-ready operational state");
        persist_sync_lifecycle_phase(&profile_id, "idle", "Idle - waiting for next sync cycle")
            .expect("persist idle phase after readiness");

        let final_row = read_sync_lifecycle_row(&profile_id)
            .expect("read final lifecycle row")
            .expect("final lifecycle row exists");
        assert_eq!(final_row.phase, "idle");
        assert_eq!(final_row.phase_message, "Idle - waiting for next sync cycle");
        assert_eq!(final_row.activity_stage, "idle");
        assert_eq!(final_row.activity_progress_mode, "hidden");
        assert!(final_row.two_way_ready);
    }

    #[test]
    fn claim_upload_job_recovers_expired_lease_and_claims() {
        let profile_id = test_profile_id("upload-lease-recovery");
        clear_profile_rows(&profile_id);

        let expired_lease = current_unix_seconds().saturating_sub(30);
        insert_job_with_state_and_lease(
            &profile_id,
            UPLOAD_JOB_DIRECTION,
            "docs/upload-expired-lease.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            None,
            Some("old-cycle"),
            Some(expired_lease),
        );

        let claimed = claim_upload_job_path(
            &profile_id,
            "docs/upload-expired-lease.txt",
            "new-cycle",
        )
        .expect("claim upload job after stale lease recovery");
        assert!(claimed);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let (state, run_state, lease_owner): (String, String, Option<String>) = connection
            .query_row(
                "SELECT state, run_state, lease_owner
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = ?2
                   AND item_id = ?3",
                params![
                    &profile_id,
                    UPLOAD_JOB_DIRECTION,
                    "docs/upload-expired-lease.txt"
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read recovered upload lease row");
        assert_eq!(state, DOWNLOAD_JOB_STATE_IN_PROGRESS);
        assert_eq!(run_state, JOB_RUN_STATE_CLAIMED);
        assert_eq!(lease_owner.as_deref(), Some("new-cycle"));
    }

    #[test]
    fn claim_action_jobs_respects_retry_schedule_and_recovers_stale_leases() {
        let profile_id = test_profile_id("action-claim-retry-and-lease");
        clear_profile_rows(&profile_id);

        let now = current_unix_seconds();
        insert_job_with_state_and_lease(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-queued.txt",
            DOWNLOAD_JOB_STATE_QUEUED,
            JOB_RUN_STATE_IDLE,
            None,
            None,
            None,
        );
        insert_job_with_state_and_lease(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-retry-due.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            Some(now.saturating_sub(1)),
            None,
            None,
        );
        insert_job_with_state_and_lease(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-retry-future.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            Some(now.saturating_add(3600)),
            None,
            None,
        );
        insert_job_with_state_and_lease(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-expired-lease.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            None,
            Some("old-cycle"),
            Some(now.saturating_sub(60)),
        );

        let claimed = claim_action_job_paths(&profile_id, DELETE_REMOTE_JOB_DIRECTION, "cycle-action")
            .expect("claim action jobs");

        assert!(claimed.contains("docs/delete-queued.txt"));
        assert!(claimed.contains("docs/delete-retry-due.txt"));
        assert!(claimed.contains("docs/delete-expired-lease.txt"));
        assert!(!claimed.contains("docs/delete-retry-future.txt"));

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let future_state: String = connection
            .query_row(
                "SELECT state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                   AND direction = ?2
                   AND item_id = ?3",
                params![
                    &profile_id,
                    DELETE_REMOTE_JOB_DIRECTION,
                    "docs/delete-retry-future.txt"
                ],
                |row| row.get(0),
            )
            .expect("read future retry action state");
        assert_eq!(future_state, DOWNLOAD_JOB_STATE_RETRY_WAIT);
    }

    #[test]
    fn upload_retry_wait_is_not_claimed_until_due() {
        let profile_id = test_profile_id("upload-retry-wait-schedule");
        clear_profile_rows(&profile_id);

        upsert_upload_job_queued(&profile_id, "docs/retry-upload.txt", 100, 42)
            .expect("queue upload job");
        let claimed = claim_upload_job_path(&profile_id, "docs/retry-upload.txt", "cycle-1")
            .expect("claim upload job for retry wait test");
        assert!(claimed);

        let job_id = begin_upload_job(&profile_id, "docs/retry-upload.txt", 100, 42, "cycle-1")
            .expect("begin upload job for retry wait test");
        mark_upload_job_retry_wait(
            &profile_id,
            job_id,
            "temporary network issue",
            Duration::from_secs(3600),
        )
        .expect("mark upload job retry wait");

        let not_due = claim_upload_job_path(&profile_id, "docs/retry-upload.txt", "cycle-2")
            .expect("claim upload retry-wait not due");
        assert!(!not_due);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        connection
            .execute(
                "UPDATE sync_jobs
                 SET next_retry_at = ?1,
                     updated_at = ?1,
                     progress_updated_at = ?1
                 WHERE id = ?2",
                params![current_unix_seconds().saturating_sub(1), job_id],
            )
            .expect("force retry wait job due");

        let due = claim_upload_job_path(&profile_id, "docs/retry-upload.txt", "cycle-2")
            .expect("claim upload retry-wait due");
        assert!(due);
    }
}
