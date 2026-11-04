use super::*;

// Sync shutdown: before SomeDrive exits for any reason (an in-app update, tray
// Quit, closing with close-to-tray off, SIGTERM/SIGINT/SIGHUP, session logout),
// sync is brought to a clean stop so an exit never kills a transfer mid-flight
// or leaves the job queue claimed.
//
//   Running
//     -> FinishingTransfers  no new cycles, downloads or uploads start; the
//                            transfers already running finish (bounded wait)
//     -> StoppingWorkers     every sync worker is cancelled through the same
//                            path as Pause and must fully exit (bounded wait)
//     -> Stopped             running/claimed jobs are returned to the queue
//
// The persisted per-account agent state is never changed, so accounts that
// were syncing resume automatically on the next start. An update that cannot
// proceed calls `resume_sync_after_cancelled_shutdown` to return to Running.
// Only one shutdown ever runs: `begin_sync_shutdown` claims it, and a later
// exit path waits for that one (`wait_for_sync_stopped`) instead of draining
// twice.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncShutdownPhase {
    Running,
    FinishingTransfers,
    StoppingWorkers,
    Stopped,
}

/// Why sync is shutting down. Sets the time budget and the activity message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncShutdownReason {
    /// Installing an update; the user is watching "Finishing sync before
    /// updating…", so transfers get longer to finish.
    Update,
    /// The app is exiting (Quit, close, signal, logout). The budget stays
    /// inside the installer's 25 s SIGTERM grace and session-logout timeouts.
    Exit,
}

impl SyncShutdownReason {
    /// How long in-flight transfers may run to completion before they are
    /// cancelled (cancelling is safe: downloads write to a temp file that is
    /// discarded, and the job is requeued).
    pub(crate) fn transfer_drain_timeout(self) -> Duration {
        match self {
            Self::Update => Duration::from_secs(30),
            Self::Exit => Duration::from_secs(10),
        }
    }

    /// How long cancelled sync workers may take to unwind. Cancellation is
    /// normally observed within a second.
    pub(crate) fn worker_stop_timeout(self) -> Duration {
        match self {
            Self::Update => Duration::from_secs(20),
            Self::Exit => Duration::from_secs(10),
        }
    }

    /// The longest a shutdown for this reason can take.
    pub fn total_timeout(self) -> Duration {
        self.transfer_drain_timeout() + self.worker_stop_timeout()
    }

    pub(crate) fn activity_message(self) -> &'static str {
        match self {
            Self::Update => "Sync paused to install an app update",
            Self::Exit => "Sync paused because SomeDrive is closing",
        }
    }
}

pub(crate) static SYNC_SHUTDOWN_PHASE: std::sync::Mutex<SyncShutdownPhase> =
    std::sync::Mutex::new(SyncShutdownPhase::Running);

/// Download worker tasks still alive across all accounts. They outlive their
/// cycle briefly after a cancel (removing temp files, requeueing their job), so
/// shutdown waits for them as well as for the sync workers.
pub(crate) static ACTIVE_DOWNLOAD_WORKER_TASKS: AtomicUsize = AtomicUsize::new(0);

pub(crate) const SYNC_SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(200);

pub(crate) fn sync_shutdown_phase() -> SyncShutdownPhase {
    SYNC_SHUTDOWN_PHASE
        .lock()
        .map(|phase| *phase)
        .unwrap_or(SyncShutdownPhase::Stopped)
}

pub(crate) fn set_sync_shutdown_phase(next_phase: SyncShutdownPhase) {
    if let Ok(mut phase) = SYNC_SHUTDOWN_PHASE.lock() {
        log::info!("SYNC_SHUTDOWN_PHASE from={:?} to={:?}", *phase, next_phase);
        *phase = next_phase;
    }
}

/// False once a shutdown has started: no new sync cycle, download or upload
/// may begin.
pub(crate) fn sync_accepts_new_work() -> bool {
    sync_shutdown_phase() == SyncShutdownPhase::Running
}

/// Unwinds the current cycle (as a cancellation) instead of starting another
/// transfer while sync is shutting down.
pub(crate) fn ensure_accepting_new_transfers() -> Result<(), String> {
    if sync_accepts_new_work() {
        Ok(())
    } else {
        Err(SYNC_CANCELLED_ERROR.to_string())
    }
}

/// Counts a download worker task from before it is spawned until it exits.
pub(crate) struct DownloadWorkerTaskGuard;

impl DownloadWorkerTaskGuard {
    pub(crate) fn register() -> Self {
        ACTIVE_DOWNLOAD_WORKER_TASKS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for DownloadWorkerTaskGuard {
    fn drop(&mut self) {
        ACTIVE_DOWNLOAD_WORKER_TASKS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn count_in_flight_transfers(
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
) -> usize {
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

/// Claims the one sync shutdown. Fails if one is already under way or done,
/// so a second request can never drain twice or resume sync underneath the
/// first.
pub fn begin_sync_shutdown(reason: SyncShutdownReason) -> Result<(), String> {
    let mut phase = SYNC_SHUTDOWN_PHASE
        .lock()
        .map_err(|_| "Sync shutdown state lock is poisoned".to_string())?;
    if *phase != SyncShutdownPhase::Running {
        return Err("Sync is already shutting down".to_string());
    }
    log::info!("SYNC_SHUTDOWN_BEGIN reason={reason:?}");
    *phase = SyncShutdownPhase::FinishingTransfers;
    Ok(())
}

/// Brings every account's sync to a clean stop. Call after
/// `begin_sync_shutdown`. On error during an update the caller must call
/// `resume_sync_after_cancelled_shutdown`.
pub async fn drain_sync(
    state: &tauri::State<'_, AppState>,
    reason: SyncShutdownReason,
) -> Result<(), String> {
    wait_for_in_flight_transfers(&state.sync_runtime, reason.transfer_drain_timeout()).await;

    set_sync_shutdown_phase(SyncShutdownPhase::StoppingWorkers);
    stop_all_sync_workers(state, reason)?;
    wait_for_sync_workers_to_exit(state, reason.worker_stop_timeout()).await?;

    set_sync_shutdown_phase(SyncShutdownPhase::Stopped);
    requeue_interrupted_jobs()?;
    log::info!("SYNC_SHUTDOWN_COMPLETE reason={reason:?}");
    Ok(())
}

/// Waits (bounded) for a shutdown another exit path already started, e.g. an
/// update whose installer launch is followed by the app's normal exit.
pub async fn wait_for_sync_stopped(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match sync_shutdown_phase() {
            SyncShutdownPhase::Stopped => return true,
            SyncShutdownPhase::Running => return false,
            SyncShutdownPhase::FinishingTransfers | SyncShutdownPhase::StoppingWorkers => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(SYNC_SHUTDOWN_POLL_INTERVAL).await;
    }
}

pub(crate) async fn wait_for_in_flight_transfers(
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut in_flight = count_in_flight_transfers(sync_runtime);
    log::info!(
        "SYNC_SHUTDOWN_DRAIN_START in_flight_transfers={} timeout_seconds={}",
        in_flight,
        timeout.as_secs()
    );
    while in_flight > 0 {
        if tokio::time::Instant::now() >= deadline {
            log::warn!(
                "SYNC_SHUTDOWN_DRAIN_TIMEOUT in_flight_transfers={} action=cancel_and_requeue",
                in_flight
            );
            return;
        }
        tokio::time::sleep(SYNC_SHUTDOWN_POLL_INTERVAL).await;
        in_flight = count_in_flight_transfers(sync_runtime);
    }
    log::info!("SYNC_SHUTDOWN_DRAIN_COMPLETE in_flight_transfers=0");
}

/// Cancels every running worker through the same signals as Pause, without
/// touching the persisted agent state.
pub(crate) fn stop_all_sync_workers(
    state: &tauri::State<'_, AppState>,
    reason: SyncShutdownReason,
) -> Result<(), String> {
    let running_profile_ids: Vec<String> = state
        .sync_worker_stops
        .lock()
        .map_err(|_| "Sync worker lock is poisoned".to_string())?
        .keys()
        .cloned()
        .collect();
    for profile_id in running_profile_ids {
        log::info!(
            "{} SYNC_SHUTDOWN_STOP_WORKER",
            log_context::account_prefix(&profile_id)
        );
        stop_sync_worker(state, &profile_id)?;
        let _ = activity_log::append_event(
            &profile_id,
            &log_context::account_identity(&profile_id),
            "info",
            &format!(
                "{} {}",
                log_context::account_prefix(&profile_id),
                reason.activity_message()
            ),
        );
    }
    Ok(())
}

pub(crate) async fn wait_for_sync_workers_to_exit(
    state: &tauri::State<'_, AppState>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
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
                "{} SYNC_SHUTDOWN_WORKER_STOP_TIMEOUT timeout_seconds={}",
                log_context::account_prefix(&profile_id),
                timeout.as_secs()
            );
            return Err(format!(
                "Sync did not stop within {} seconds. Try again in a moment.",
                timeout.as_secs()
            ));
        }
    }

    while ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst) > 0 {
        if tokio::time::Instant::now() >= deadline {
            log::error!(
                "SYNC_SHUTDOWN_DOWNLOAD_WORKERS_STOP_TIMEOUT active_tasks={}",
                ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst)
            );
            return Err(format!(
                "Downloads did not stop within {} seconds. Try again in a moment.",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(SYNC_SHUTDOWN_POLL_INTERVAL).await;
    }
    log::info!("SYNC_SHUTDOWN_WORKERS_EXITED");
    Ok(())
}

/// Returns any job still marked claimed or running to the queue, so the next
/// start picks it up (startup does the same, this keeps the DB tidy at exit).
pub(crate) fn requeue_interrupted_jobs() -> Result<(), String> {
    for profile in load_profiles()? {
        match reset_running_sync_jobs_for_pause(&profile.id) {
            Ok(requeued_jobs) => log::info!(
                "{} SYNC_SHUTDOWN_JOBS_REQUEUED count={}",
                log_context::account_prefix(&profile.id),
                requeued_jobs
            ),
            Err(error) => return Err(format!("Failed to save sync state: {error}")),
        }
    }
    Ok(())
}

/// Undoes a shutdown that could not finish (an update whose installer failed
/// to start, or whose sync did not stop in time): new work is accepted again
/// and every account whose saved state is "syncing" is restarted.
pub fn resume_sync_after_cancelled_shutdown(state: &tauri::State<'_, AppState>) {
    set_sync_shutdown_phase(SyncShutdownPhase::Running);
    let profiles = match load_profiles() {
        Ok(profiles) => profiles,
        Err(error) => {
            log::error!("SYNC_SHUTDOWN_RESUME_FAILED error={}", error);
            return;
        }
    };
    for profile in profiles {
        if profile.agent_state != "syncing" || !profile.auth_configured {
            continue;
        }
        if let Err(error) = on_agent_state_changed(state, &profile.id, "syncing") {
            log::error!(
                "{} SYNC_SHUTDOWN_RESUME_FAILED error={}",
                log_context::account_prefix(&profile.id),
                error
            );
        }
    }
    log::info!("SYNC_SHUTDOWN_CANCELLED sync_resumed=true");
}

#[cfg(test)]
mod sync_shutdown_tests {
    use super::*;

    // One test owns the process-wide phase, so parallel tests cannot race on it.
    #[test]
    fn sync_shutdown_stops_new_work_and_refuses_a_second_request() {
        assert!(sync_accepts_new_work());
        assert!(ensure_accepting_new_transfers().is_ok());

        begin_sync_shutdown(SyncShutdownReason::Update).expect("first shutdown is accepted");
        assert!(!sync_accepts_new_work());
        assert_eq!(
            ensure_accepting_new_transfers(),
            Err(SYNC_CANCELLED_ERROR.to_string())
        );
        assert!(begin_sync_shutdown(SyncShutdownReason::Exit).is_err());

        let before = ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst);
        let guard = DownloadWorkerTaskGuard::register();
        assert_eq!(
            ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst),
            before + 1
        );
        drop(guard);
        assert_eq!(ACTIVE_DOWNLOAD_WORKER_TASKS.load(Ordering::SeqCst), before);

        set_sync_shutdown_phase(SyncShutdownPhase::Stopped);
        assert!(tauri::async_runtime::block_on(wait_for_sync_stopped(
            Duration::from_millis(10)
        )));

        set_sync_shutdown_phase(SyncShutdownPhase::Running);
        assert!(sync_accepts_new_work());
    }

    #[test]
    fn exit_shutdown_fits_inside_the_installer_sigterm_grace() {
        assert!(SyncShutdownReason::Exit.total_timeout() < Duration::from_secs(25));
    }
}
