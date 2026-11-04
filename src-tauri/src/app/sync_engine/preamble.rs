use super::*;

pub(crate) const GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";
pub(crate) const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 12;
pub(crate) const MAX_DOWNLOAD_RETRIES: u32 = 5;
pub(crate) const DOWNLOAD_PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const MAX_RETRY_DELAY_SECONDS: u64 = 30;
pub(crate) const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 60;
pub(crate) const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 15;
pub(crate) const DEFAULT_STALL_TIMEOUT_SECONDS: u64 = 60;
pub(crate) const SYNC_CANCELLED_ERROR: &str = "Synchronization cancelled";
pub(crate) const DOWNLOAD_RETRY_DEFERRED_ERROR: &str = "Download retry deferred";
pub(crate) const CANCEL_POLL_INTERVAL_MILLIS: u64 = 100;

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
        clear_sync_retry(profile_id);
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
    if cleared_jobs > 0 {
        log::info!(
            "{} STARTUP_SYNC_PREP_DRAINED running_jobs_cleared={}",
            log_context::account_prefix(profile_id),
            cleared_jobs
        );
    }
    Ok(cleared_jobs)
}

/// The user reviewed the held deletions and allows them. The next cycle
/// deletes only paths that are on the reviewed list and still planned for
/// deletion then.
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
        "syncing",
        "Deletions confirmed - applying them",
    );
    Ok(())
}

/// The user keeps the files instead: each held path forgets its baseline, so
/// the next plan restores it on whichever side it is missing (an upload or a
/// download) rather than deleting it.
pub fn keep_cloud_files_after_large_delete(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let guard_state = read_large_delete_guard_state(profile_id)?;
    for path in &guard_state.pending_paths {
        forget_sync_file(profile_id, path)?;
    }
    log::info!(
        "{} LARGE_DELETE_KEPT_FILES count={}",
        log_context::account_prefix(profile_id),
        guard_state.pending_paths.len()
    );
    persist_large_delete_guard_state(profile_id, &LargeDeleteGuardState::default())?;
    runtime_clear_issue(&state.sync_runtime, profile_id);
    runtime_set_phase(
        &state.sync_runtime,
        profile_id,
        "syncing",
        "Keeping files - restoring them",
    );
    Ok(())
}

/// What the account's Sync problems and Not synced tabs show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProblemsView {
    /// Things that need the user: held conflicts, items SomeDrive cannot
    /// sync, and downloads that failed for good.
    pub problems: Vec<SyncProblem>,
    /// Things skipped by design (OneNote, exclude rules, shared folders,
    /// malware), one line per top-most item, with the reason.
    pub not_synced: Vec<NotSyncedItem>,
}

pub fn list_sync_problems(profile_id: &str) -> Result<SyncProblemsView, String> {
    let mut problems = list_file_problems(profile_id)?;
    problems.extend(list_failed_download_problems(profile_id)?);
    Ok(SyncProblemsView {
        problems,
        not_synced: list_not_synced_items(profile_id)?,
    })
}

/// Records the user's choice for held conflicts; the next cycle applies it.
pub fn resolve_sync_conflicts(
    profile_id: &str,
    paths: &[String],
    resolution: &str,
) -> Result<usize, String> {
    let resolved = set_conflict_resolution(profile_id, paths, resolution)?;
    log::info!(
        "{} CONFLICT_RESOLUTION_CHOSEN resolution={} requested={} recorded={}",
        log_context::account_prefix(profile_id),
        resolution,
        paths.len(),
        resolved
    );
    Ok(resolved)
}

pub fn get_large_delete_pending_paths(profile_id: &str) -> Result<Vec<String>, String> {
    let guard_state = read_large_delete_guard_state(profile_id)?;
    Ok(guard_state.pending_paths)
}
