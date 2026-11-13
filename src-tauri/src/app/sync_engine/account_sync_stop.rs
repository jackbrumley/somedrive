use super::*;

// Stopping one account's sync for good, before the account is removed: its
// worker and download tasks must have exited, so nothing writes to the
// account's database or sync folder while removal deletes them.

/// How long removal waits for the account's sync to stop. A transfer that is
/// still running is cancelled as Pause does, so this only covers unwinding.
const ACCOUNT_SYNC_STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Cancels the account's sync worker exactly as Pause does, then waits for the
/// worker and its download tasks to exit. The saved agent state is not
/// changed; if removal does not go ahead, `on_agent_state_changed` with the
/// saved state restarts the account.
pub async fn stop_account_sync(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let account_prefix = log_context::account_prefix(profile_id);
    log::info!("{account_prefix} ACCOUNT_SYNC_STOP_BEGIN");
    stop_sync_worker(state, profile_id)?;
    let deadline = tokio::time::Instant::now() + ACCOUNT_SYNC_STOP_TIMEOUT;
    let worker_task = state
        .sync_worker_tasks
        .lock()
        .map_err(|_| "Sync worker task lock is poisoned".to_string())?
        .remove(profile_id);
    if let Some(worker_task) = worker_task {
        if tokio::time::timeout_at(deadline, worker_task)
            .await
            .is_err()
        {
            log::error!(
                "{account_prefix} ACCOUNT_SYNC_STOP_TIMEOUT waiting_for=worker timeout_seconds={}",
                ACCOUNT_SYNC_STOP_TIMEOUT.as_secs()
            );
            return Err(stop_timeout_message());
        }
    }
    while active_download_worker_tasks(Some(profile_id)) > 0 {
        if tokio::time::Instant::now() >= deadline {
            log::error!(
                "{account_prefix} ACCOUNT_SYNC_STOP_TIMEOUT waiting_for=downloads active_tasks={}",
                active_download_worker_tasks(Some(profile_id))
            );
            return Err(stop_timeout_message());
        }
        tokio::time::sleep(SYNC_SHUTDOWN_POLL_INTERVAL).await;
    }
    log::info!("{account_prefix} ACCOUNT_SYNC_STOPPED");
    Ok(())
}

fn stop_timeout_message() -> String {
    format!(
        "Sync did not stop within {} seconds. Try again in a moment.",
        ACCOUNT_SYNC_STOP_TIMEOUT.as_secs()
    )
}

/// Files the account has queued for upload that have not reached OneDrive:
/// waiting, uploading, retrying or failed for good. Each may exist only on
/// this device.
pub fn count_unuploaded_files(profile_id: &str) -> Result<usize, String> {
    let counters = read_upload_job_counters(profile_id)?;
    Ok(counters.planned_total.saturating_sub(counters.completed))
}

/// Forgets the account's in-memory sync state once it is removed: its live
/// status, cancel flag, cycle lock, retry timer and database connections.
/// Call after the account is gone from `profiles.json`; from then on its
/// database refuses every connection.
pub fn forget_removed_account(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    if let Ok(mut runtime_map) = state.sync_runtime.lock() {
        crate::app::sync_runtime::remove_account(&mut runtime_map, profile_id);
    }
    if let Ok(mut flags) = state.sync_cancel_flags.lock() {
        flags.remove(profile_id);
    }
    if let Ok(mut locks) = state.sync_cycle_locks.lock() {
        locks.remove(profile_id);
    }
    if let Ok(mut pending) = state.auth_pending.lock() {
        pending.remove(profile_id);
    }
    if let Ok(mut pending) = state.interactive_auth_pending.lock() {
        pending.retain(|_, entry| entry.profile_id != profile_id);
    }
    clear_sync_retry(profile_id);
    retire_sync_database(profile_id)
}
