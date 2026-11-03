// Pre-update hook (Release and Update Standard, section 9): before the in-app
// updater exits SomeDrive, sync is brought to a clean stop so an update never
// kills a transfer mid-flight or leaves the job queue claimed.
//
//   Running
//     -> FinishingTransfers  no new cycles, downloads or uploads start; the
//                            transfers already running finish (bounded wait)
//     -> StoppingWorkers     every sync worker is cancelled through the same
//                            path as Pause and must fully exit (bounded wait)
//     -> Stopped             running/claimed jobs are returned to the queue
//
// The persisted per-account agent state is never changed, so accounts that
// were syncing resume automatically when the updated version starts. If the
// update cannot proceed, `resume_sync_after_cancelled_update` returns to
// Running and restarts those accounts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncShutdownPhase {
    Running,
    FinishingTransfers,
    StoppingWorkers,
    Stopped,
}

static SYNC_SHUTDOWN_PHASE: std::sync::Mutex<SyncShutdownPhase> =
    std::sync::Mutex::new(SyncShutdownPhase::Running);

/// Download worker tasks still alive across all accounts. They outlive their
/// cycle briefly after a cancel (removing temp files, requeueing their job), so
/// shutdown waits for them as well as for the sync workers.
static ACTIVE_DOWNLOAD_WORKER_TASKS: AtomicUsize = AtomicUsize::new(0);

/// How long in-flight transfers may run to completion before they are
/// cancelled (cancelling is safe: downloads write to a temp file that is
/// discarded, and the job is requeued).
const UPDATE_TRANSFER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long cancelled sync workers may take to unwind. Cancellation is
/// normally observed within a second; exceeding this aborts the update.
const UPDATE_WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(200);

fn sync_shutdown_phase() -> SyncShutdownPhase {
    SYNC_SHUTDOWN_PHASE
        .lock()
        .map(|phase| *phase)
        .unwrap_or(SyncShutdownPhase::Stopped)
}

fn set_sync_shutdown_phase(next_phase: SyncShutdownPhase) {
    if let Ok(mut phase) = SYNC_SHUTDOWN_PHASE.lock() {
        log::info!(
            "SYNC_UPDATE_SHUTDOWN_PHASE from={:?} to={:?}",
            *phase,
            next_phase
        );
        *phase = next_phase;
    }
}

/// False once an update has started preparing: no new sync cycle, download or
/// upload may begin.
fn sync_accepts_new_work() -> bool {
    sync_shutdown_phase() == SyncShutdownPhase::Running
}

/// Unwinds the current cycle (as a cancellation) instead of starting another
/// transfer while an update is being prepared.
fn ensure_accepting_new_transfers() -> Result<(), String> {
    if sync_accepts_new_work() {
        Ok(())
    } else {
        Err(SYNC_CANCELLED_ERROR.to_string())
    }
}

/// Counts a download worker task from before it is spawned until it exits.
struct DownloadWorkerTaskGuard;

impl DownloadWorkerTaskGuard {
    fn register() -> Self {
        ACTIVE_DOWNLOAD_WORKER_TASKS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for DownloadWorkerTaskGuard {
    fn drop(&mut self) {
        ACTIVE_DOWNLOAD_WORKER_TASKS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn count_in_flight_transfers(sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>) -> usize {
    sync_runtime
        .lock()
        .map(|runtime_map| {
            runtime_map
                .values()
                .map(|status| status.in_progress.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Claims the update shutdown. Fails if one is already under way, so a second
/// request can never resume sync underneath the first.
pub fn begin_update_shutdown() -> Result<(), String> {
    let mut phase = SYNC_SHUTDOWN_PHASE
        .lock()
        .map_err(|_| "Sync shutdown state lock is poisoned".to_string())?;
    if *phase != SyncShutdownPhase::Running {
        return Err("An update is already being prepared".to_string());
    }
    log::info!("SYNC_UPDATE_SHUTDOWN_PHASE from=Running to=FinishingTransfers");
    *phase = SyncShutdownPhase::FinishingTransfers;
    Ok(())
}

/// Brings every account's sync to a clean stop for an update. Call after
/// `begin_update_shutdown`. On error the caller must call
/// `resume_sync_after_cancelled_update`.
pub async fn drain_sync_for_update(state: &tauri::State<'_, AppState>) -> Result<(), String> {
    wait_for_in_flight_transfers(&state.sync_runtime).await;

    set_sync_shutdown_phase(SyncShutdownPhase::StoppingWorkers);
    stop_all_sync_workers_for_update(state)?;
    wait_for_sync_workers_to_exit(state).await?;

    set_sync_shutdown_phase(SyncShutdownPhase::Stopped);
    requeue_interrupted_jobs_for_update()?;
    log::info!("SYNC_UPDATE_SHUTDOWN_COMPLETE");
    Ok(())
}

async fn wait_for_in_flight_transfers(sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>) {
    let deadline = tokio::time::Instant::now() + UPDATE_TRANSFER_DRAIN_TIMEOUT;
    let mut in_flight = count_in_flight_transfers(sync_runtime);
    log::info!(
        "SYNC_UPDATE_DRAIN_START in_flight_transfers={} timeout_seconds={}",
        in_flight,
        UPDATE_TRANSFER_DRAIN_TIMEOUT.as_secs()
    );
    while in_flight > 0 {
        if tokio::time::Instant::now() >= deadline {
            log::warn!(
                "SYNC_UPDATE_DRAIN_TIMEOUT in_flight_transfers={} action=cancel_and_requeue",
                in_flight
            );
            return;
        }
        tokio::time::sleep(UPDATE_SHUTDOWN_POLL_INTERVAL).await;
        in_flight = count_in_flight_transfers(sync_runtime);
    }
    log::info!("SYNC_UPDATE_DRAIN_COMPLETE in_flight_transfers=0");
}

/// Cancels every running worker through the same signals as Pause, without
/// touching the persisted agent state.
fn stop_all_sync_workers_for_update(state: &tauri::State<'_, AppState>) -> Result<(), String> {
    let running_profile_ids: Vec<String> = state
        .sync_worker_stops
        .lock()
        .map_err(|_| "Sync worker lock is poisoned".to_string())?
        .keys()
        .cloned()
        .collect();
    for profile_id in running_profile_ids {
        log::info!(
            "{} SYNC_UPDATE_STOP_WORKER",
            log_context::account_prefix(&profile_id)
        );
        stop_sync_worker(state, &profile_id)?;
        let _ = activity_log::append_event(
            &profile_id,
            &log_context::account_identity(&profile_id),
            "info",
            &format!(
                "{} Sync paused to install an app update",
                log_context::account_prefix(&profile_id)
            ),
        );
    }
    Ok(())
}

async fn wait_for_sync_workers_to_exit(state: &tauri::State<'_, AppState>) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + UPDATE_WORKER_STOP_TIMEOUT;
    let worker_tasks: Vec<(String, tauri::async_runtime::JoinHandle<()>)> = state
        .sync_worker_tasks
        .lock()
        .map_err(|_| "Sync worker task lock is poisoned".to_string())?
        .drain()
        .collect();

    for (profile_id, worker_task) in worker_tasks {
        if tokio::time::timeout_at(deadline, worker_task)
            .await
            .is_err()
        {
            log::error!(
                "{} SYNC_UPDATE_WORKER_STOP_TIMEOUT timeout_seconds={}",
                log_context::account_prefix(&profile_id),
                UPDATE_WORKER_STOP_TIMEOUT.as_secs()
            );
            return Err(format!(
                "Sync did not stop within {} seconds, so the update was not started. Try again in a moment.",
                UPDATE_WORKER_STOP_TIMEOUT.as_secs()
            ));
        }
    }

    while ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst) > 0 {
        if tokio::time::Instant::now() >= deadline {
            log::error!(
                "SYNC_UPDATE_DOWNLOAD_WORKERS_STOP_TIMEOUT active_tasks={}",
                ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst)
            );
            return Err(format!(
                "Downloads did not stop within {} seconds, so the update was not started. Try again in a moment.",
                UPDATE_WORKER_STOP_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(UPDATE_SHUTDOWN_POLL_INTERVAL).await;
    }
    log::info!("SYNC_UPDATE_WORKERS_EXITED");
    Ok(())
}

/// Returns any job still marked claimed or running to the queue, so the next
/// start picks it up (startup does the same, this keeps the DB tidy at exit).
fn requeue_interrupted_jobs_for_update() -> Result<(), String> {
    for profile in load_profiles()? {
        match reset_running_sync_jobs_for_pause(&profile.id) {
            Ok(requeued_jobs) => log::info!(
                "{} SYNC_UPDATE_JOBS_REQUEUED count={}",
                log_context::account_prefix(&profile.id),
                requeued_jobs
            ),
            Err(error) => {
                return Err(format!(
                    "Failed to save sync state before updating: {error}"
                ))
            }
        }
    }
    Ok(())
}

/// Undoes an update shutdown that could not finish (the updater failed to
/// start, or sync did not stop in time): new work is accepted again and every
/// account whose saved state is "syncing" is restarted.
pub fn resume_sync_after_cancelled_update(state: &tauri::State<'_, AppState>) {
    set_sync_shutdown_phase(SyncShutdownPhase::Running);
    let profiles = match load_profiles() {
        Ok(profiles) => profiles,
        Err(error) => {
            log::error!("SYNC_UPDATE_RESUME_FAILED error={}", error);
            return;
        }
    };
    for profile in profiles {
        if profile.agent_state != "syncing" || !profile.auth_configured {
            continue;
        }
        if let Err(error) = on_agent_state_changed(state, &profile.id, "syncing") {
            log::error!(
                "{} SYNC_UPDATE_RESUME_FAILED error={}",
                log_context::account_prefix(&profile.id),
                error
            );
        }
    }
    log::info!("SYNC_UPDATE_SHUTDOWN_CANCELLED sync_resumed=true");
}

#[cfg(test)]
mod update_shutdown_tests {
    use super::*;

    // One test owns the process-wide phase, so parallel tests cannot race on it.
    #[test]
    fn update_shutdown_stops_new_work_and_refuses_a_second_request() {
        assert!(sync_accepts_new_work());
        assert!(ensure_accepting_new_transfers().is_ok());

        begin_update_shutdown().expect("first update shutdown is accepted");
        assert!(!sync_accepts_new_work());
        assert_eq!(
            ensure_accepting_new_transfers(),
            Err(SYNC_CANCELLED_ERROR.to_string())
        );
        assert!(begin_update_shutdown().is_err());

        let before = ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst);
        let guard = DownloadWorkerTaskGuard::register();
        assert_eq!(
            ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst),
            before + 1
        );
        drop(guard);
        assert_eq!(ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst), before);

        set_sync_shutdown_phase(SyncShutdownPhase::Running);
        assert!(sync_accepts_new_work());
    }
}
