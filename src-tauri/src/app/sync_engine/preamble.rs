use crate::app::account_profiles::{load_profiles, save_profiles, AccountProfile};
use crate::app::activity_log;
use crate::app::auth::{
    load_auth_session, mark_account_sign_in_required, refresh_access_token, TokenRefreshError,
};
use crate::app::log_context;
use crate::app::state::AppState;
use crate::app::sync_runtime::{self, SyncRuntimeMap};
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 12;
const MAX_DOWNLOAD_RETRIES: u32 = 5;
const DOWNLOAD_PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 15;
const DEFAULT_DOWNLOAD_TIMEOUT_BASE_SECONDS: u64 = 30;
const DEFAULT_DOWNLOAD_TIMEOUT_MIN_SECONDS: u64 = 60;
const DEFAULT_DOWNLOAD_TIMEOUT_MAX_SECONDS: u64 = 1800;
const DEFAULT_DOWNLOAD_TIMEOUT_PER_MIB_MILLIS: u64 = 1400;
const DEFAULT_STALL_TIMEOUT_SECONDS: u64 = 60;
const SYNC_CANCELLED_ERROR: &str = "Synchronization cancelled";
const DOWNLOAD_RETRY_DEFERRED_ERROR: &str = "Download retry deferred";
const CANCEL_POLL_INTERVAL_MILLIS: u64 = 100;

pub fn on_agent_state_changed(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
    agent_state: &str,
) -> Result<(), String> {
    log::info!(
        "{} SYNC_AGENT_STATE_CHANGE requested_state={}",
        log_context::account_prefix(profile_id),
        agent_state
    );
    if let Err(error) = persist_sync_lifecycle_agent_state(profile_id, agent_state) {
        log::warn!(
            "{} SYNC_LIFECYCLE_AGENT_STATE_PERSIST_FAILED state={} error={}",
            log_context::account_prefix(profile_id),
            agent_state,
            error
        );
    }
    if agent_state == "syncing" {
        runtime_set_engine_state(&state.sync_runtime, profile_id, "running");
        let _ = set_cancel_flag(state, profile_id, false)?;
        runtime_clear_issue(&state.sync_runtime, profile_id);
        start_sync_worker(state, profile_id)?;
    } else {
        runtime_set_engine_state(&state.sync_runtime, profile_id, "paused");
        let _ = set_cancel_flag(state, profile_id, true)?;
        stop_sync_worker(state, profile_id)?;
        match reset_running_sync_jobs_for_pause(profile_id) {
            Ok(cleared_jobs) => {
                if cleared_jobs > 0 {
                    log::info!(
                        "{} SYNC_PAUSE_DRAINED running_jobs_cleared={}",
                        log_context::account_prefix(profile_id),
                        cleared_jobs
                    );
                }
            }
            Err(error) => {
                log::warn!(
                    "{} SYNC_PAUSE_DRAIN_FAILED error={}",
                    log_context::account_prefix(profile_id),
                    error
                );
            }
        }
        runtime_reset_transfer_activity(&state.sync_runtime, profile_id);
        let (phase, message) = if agent_state == "paused" {
            ("paused", "Synchronization paused")
        } else {
            ("idle", "Idle")
        };
        runtime_set_phase(&state.sync_runtime, profile_id, phase, message);
        runtime_clear_issue(&state.sync_runtime, profile_id);
    }
    Ok(())
}

pub fn prepare_startup_sync_resume(profile_id: &str) -> Result<usize, String> {
    let cleared_jobs = reset_running_sync_jobs_for_pause(profile_id)?;
    rebuild_sync_state_from_db(profile_id)?;
    if cleared_jobs > 0 {
        log::info!(
            "{} STARTUP_SYNC_PREP_DRAINED running_jobs_cleared={}",
            log_context::account_prefix(profile_id),
            cleared_jobs
        );
    }
    Ok(cleared_jobs)
}

pub fn confirm_large_delete_guard(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let mut guard_state = read_large_delete_guard_state(profile_id)?;
    if guard_state.pending_paths.is_empty() {
        return Err("No pending large deletion to confirm".to_string());
    }
    guard_state.approved = true;
    persist_large_delete_guard_state(profile_id, &guard_state)?;

    runtime_clear_issue(&state.sync_runtime, profile_id);
    runtime_set_phase(
        &state.sync_runtime,
        profile_id,
        "applying_local",
        "Large deletion confirmed - applying changes",
    );
    Ok(())
}

pub fn keep_cloud_files_after_large_delete(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let mut guard_state = read_large_delete_guard_state(profile_id)?;
    guard_state.approved = false;
    guard_state.pending_paths.clear();
    persist_large_delete_guard_state(profile_id, &guard_state)?;

    let mut lifecycle_state = read_sync_lifecycle_operational_state(profile_id)?;
    lifecycle_state.two_way_ready = false;
    lifecycle_state.bootstrap_scan_initialized = false;
    lifecycle_state.bootstrap_full_scan_completed = false;
    lifecycle_state.delta_link = None;
    lifecycle_state.active_delta_next_link = None;
    persist_lifecycle_operational_state(profile_id, &lifecycle_state)?;

    runtime_clear_issue(&state.sync_runtime, profile_id);
    runtime_set_phase(
        &state.sync_runtime,
        profile_id,
        "syncing",
        "Initial sync in progress - downloading cloud files only",
    );
    Ok(())
}

pub fn get_large_delete_pending_paths(profile_id: &str) -> Result<Vec<String>, String> {
    let guard_state = read_large_delete_guard_state(profile_id)?;
    Ok(guard_state.pending_paths)
}

#[cfg(test)]
mod preamble_tests {
    use super::*;
    use rusqlite::params;

    fn test_profile_id(label: &str) -> String {
        format!(
            "preamble-test-{label}-{}",
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
    }

    fn insert_job_row(
        profile_id: &str,
        direction: &str,
        item_id: &str,
        state: &str,
        run_state: &str,
        lease_owner: Option<&str>,
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
                    ?1, ?2, ?3, ?3, 1, 1,
                    ?4, ?5, 1, NULL, NULL,
                    ?6, ?7, 0, 1, ?8,
                    ?8, ?8, ?8, NULL
                )",
                params![
                    profile_id,
                    direction,
                    item_id,
                    state,
                    run_state,
                    lease_owner,
                    lease_owner.map(|_| now + 600),
                    now,
                ],
            )
            .expect("insert sync job row");
    }

    #[test]
    fn prepare_startup_sync_resume_resets_running_jobs_and_rebuilds_caches() {
        let profile_id = test_profile_id("startup-resume");
        clear_profile_rows(&profile_id);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, is_shared_reference,
                    shared_drive_id, shared_item_id, shared_kind,
                    remote_item_id, remote_present, local_present,
                    remote_size, local_size, remote_modified_ts, local_modified_ts,
                    desired_action, conflict_state, updated_at
                ) VALUES (
                    ?1, 'docs/resume.txt', 0, 0,
                    NULL, NULL, NULL,
                    'remote-resume', 1, 1,
                    10, 10, 100, 100,
                    'none', NULL, ?2
                )",
                params![&profile_id, now],
            )
            .expect("insert sync_files seed row");

        let stale_state = PersistedSyncState {
            remote_by_id: HashMap::from([(
                "stale".to_string(),
                RemoteKnownItem {
                    id: "stale".to_string(),
                    path: "stale/path.txt".to_string(),
                    is_dir: false,
                    size: 1,
                    modified_ts: 1,
                    is_shared_reference: false,
                    shared_drive_id: None,
                    shared_item_id: None,
                    shared_kind: None,
                },
            )]),
            remote_path_to_id: HashMap::from([("stale/path.txt".to_string(), "stale".to_string())]),
            local_snapshot: HashMap::from([(
                "stale/path.txt".to_string(),
                LocalSnapshotEntry {
                    is_dir: false,
                    size: 1,
                    modified_ts: 1,
                },
            )]),
        };
        save_sync_state(&profile_id, &stale_state).expect("write stale sync state");

        insert_job_row(
            &profile_id,
            DOWNLOAD_JOB_DIRECTION,
            "docs/dl-running.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-dl"),
        );
        insert_job_row(
            &profile_id,
            UPLOAD_JOB_DIRECTION,
            "docs/up-claimed.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-up"),
        );
        insert_job_row(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/delete-local-claimed.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-action"),
        );
        insert_job_row(
            &profile_id,
            DOWNLOAD_JOB_DIRECTION,
            "docs/retry-wait.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            None,
        );

        let drained = prepare_startup_sync_resume(&profile_id).expect("prepare startup sync resume");
        assert_eq!(drained, 3);

        let mut statement = connection
            .prepare(
                "SELECT item_id, state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                 ORDER BY item_id",
            )
            .expect("prepare read sync jobs state query");
        let rows = statement
            .query_map(params![&profile_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query sync job rows");
        let collected: Vec<(String, String, String)> =
            rows.map(|row| row.expect("read sync job row")).collect();

        for (item_id, state, run_state) in collected {
            if item_id == "docs/retry-wait.txt" {
                assert_eq!(state, DOWNLOAD_JOB_STATE_RETRY_WAIT);
                assert_eq!(run_state, JOB_RUN_STATE_IDLE);
            } else {
                assert_eq!(state, DOWNLOAD_JOB_STATE_QUEUED);
                assert_eq!(run_state, JOB_RUN_STATE_IDLE);
            }
        }

        let rebuilt = load_sync_state(&profile_id).expect("reload rebuilt state");
        assert!(rebuilt.remote_by_id.contains_key("remote-resume"));
        assert_eq!(
            rebuilt.remote_path_to_id.get("docs/resume.txt"),
            Some(&"remote-resume".to_string())
        );
        assert!(rebuilt.local_snapshot.contains_key("docs/resume.txt"));
        assert!(!rebuilt.remote_by_id.contains_key("stale"));
    }

    #[test]
    fn prepare_startup_sync_resume_is_idempotent_across_restarts() {
        let profile_id = test_profile_id("startup-idempotent");
        clear_profile_rows(&profile_id);

        insert_job_row(
            &profile_id,
            DOWNLOAD_JOB_DIRECTION,
            "docs/running-download.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-dl"),
        );
        insert_job_row(
            &profile_id,
            UPLOAD_JOB_DIRECTION,
            "docs/claimed-upload.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-up"),
        );
        insert_job_row(
            &profile_id,
            DOWNLOAD_JOB_DIRECTION,
            "docs/already-queued.txt",
            DOWNLOAD_JOB_STATE_QUEUED,
            JOB_RUN_STATE_IDLE,
            None,
        );
        insert_job_row(
            &profile_id,
            UPLOAD_JOB_DIRECTION,
            "docs/retry-upload.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            None,
        );
        insert_job_row(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/claimed-delete-remote.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-delete-remote"),
        );
        insert_job_row(
            &profile_id,
            CONFLICT_JOB_DIRECTION,
            "docs/running-conflict.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-conflict"),
        );
        insert_job_row(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/retry-delete-local.txt",
            DOWNLOAD_JOB_STATE_RETRY_WAIT,
            JOB_RUN_STATE_IDLE,
            None,
        );

        let first_drain =
            prepare_startup_sync_resume(&profile_id).expect("first startup resume drain");
        assert_eq!(first_drain, 4);

        let second_drain =
            prepare_startup_sync_resume(&profile_id).expect("second startup resume drain");
        assert_eq!(second_drain, 0);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let mut statement = connection
            .prepare(
                "SELECT item_id, state, run_state
                 FROM sync_jobs
                 WHERE profile_id = ?1
                 ORDER BY item_id",
            )
            .expect("prepare startup idempotency state query");
        let rows = statement
            .query_map(params![&profile_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query startup idempotency rows");
        let collected: Vec<(String, String, String)> =
            rows.map(|row| row.expect("read startup idempotency row")).collect();

        for (item_id, state, run_state) in collected {
            if item_id == "docs/retry-upload.txt" || item_id == "docs/retry-delete-local.txt" {
                assert_eq!(state, DOWNLOAD_JOB_STATE_RETRY_WAIT);
                assert_eq!(run_state, JOB_RUN_STATE_IDLE);
            } else {
                assert_eq!(state, DOWNLOAD_JOB_STATE_QUEUED);
                assert_eq!(run_state, JOB_RUN_STATE_IDLE);
            }
        }
    }

    #[test]
    fn full_cycle_resume_claim_and_complete_across_all_lanes() {
        let profile_id = test_profile_id("full-cycle-all-lanes");
        clear_profile_rows(&profile_id);

        insert_job_row(
            &profile_id,
            DOWNLOAD_JOB_DIRECTION,
            "docs/download-running.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-download"),
        );
        insert_job_row(
            &profile_id,
            UPLOAD_JOB_DIRECTION,
            "docs/upload-claimed.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-upload"),
        );
        insert_job_row(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-remote-claimed.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_CLAIMED,
            Some("lease-delete-remote"),
        );
        insert_job_row(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/delete-local-running.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-delete-local"),
        );
        insert_job_row(
            &profile_id,
            CONFLICT_JOB_DIRECTION,
            "docs/conflict-running.txt",
            DOWNLOAD_JOB_STATE_IN_PROGRESS,
            JOB_RUN_STATE_RUNNING,
            Some("lease-conflict"),
        );

        let drained = prepare_startup_sync_resume(&profile_id).expect("prepare startup sync resume");
        assert_eq!(drained, 5);

        let claimed_downloads =
            claim_download_jobs(&profile_id, "cycle-all-lanes", 8).expect("claim download jobs");
        assert_eq!(claimed_downloads.len(), 1);
        mark_download_job_done(&profile_id, claimed_downloads[0].job_id, false)
            .expect("complete claimed download");

        let claimed_upload = claim_upload_job_path(&profile_id, "docs/upload-claimed.txt", "cycle-all-lanes")
            .expect("claim upload path after resume");
        assert!(claimed_upload);
        let upload_job_id = begin_upload_job(
            &profile_id,
            "docs/upload-claimed.txt",
            10,
            10,
            "cycle-all-lanes",
        )
        .expect("begin upload job after claim");
        mark_upload_job_done(&profile_id, upload_job_id).expect("complete upload job");

        let claimed_delete_remote = claim_action_job_paths(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "cycle-all-lanes",
        )
        .expect("claim delete_remote action jobs");
        assert!(claimed_delete_remote.contains("docs/delete-remote-claimed.txt"));
        mark_action_job_running(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-remote-claimed.txt",
        )
        .expect("mark delete_remote action running");
        mark_action_job_done(
            &profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            "docs/delete-remote-claimed.txt",
        )
        .expect("mark delete_remote action done");

        let claimed_delete_local =
            claim_action_job_paths(&profile_id, DELETE_LOCAL_JOB_DIRECTION, "cycle-all-lanes")
                .expect("claim delete_local action jobs");
        assert!(claimed_delete_local.contains("docs/delete-local-running.txt"));
        mark_action_job_running(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/delete-local-running.txt",
        )
        .expect("mark delete_local action running");
        mark_action_job_done(
            &profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            "docs/delete-local-running.txt",
        )
        .expect("mark delete_local action done");

        let claimed_conflict =
            claim_action_job_paths(&profile_id, CONFLICT_JOB_DIRECTION, "cycle-all-lanes")
                .expect("claim conflict action jobs");
        assert!(claimed_conflict.contains("docs/conflict-running.txt"));
        mark_action_job_running(
            &profile_id,
            CONFLICT_JOB_DIRECTION,
            "docs/conflict-running.txt",
        )
        .expect("mark conflict action running");
        mark_action_job_done(&profile_id, CONFLICT_JOB_DIRECTION, "docs/conflict-running.txt")
            .expect("mark conflict action done");

        let download_counters =
            read_download_job_counters(&profile_id).expect("read final download counters");
        assert_eq!(download_counters.completed, 1);
        assert_eq!(download_counters.remaining, 0);

        let upload_counters = read_upload_job_counters(&profile_id).expect("read final upload counters");
        assert_eq!(upload_counters.completed, 1);

        assert!(
            list_pending_action_job_paths(&profile_id, DELETE_REMOTE_JOB_DIRECTION)
                .expect("list pending delete_remote actions")
                .is_empty()
        );
        assert!(
            list_pending_action_job_paths(&profile_id, DELETE_LOCAL_JOB_DIRECTION)
                .expect("list pending delete_local actions")
                .is_empty()
        );
        assert!(
            list_pending_action_job_paths(&profile_id, CONFLICT_JOB_DIRECTION)
                .expect("list pending conflict actions")
                .is_empty()
        );
    }
}
