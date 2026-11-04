use super::*;

/// When each account's next automatic retry after a failed cycle may run.
/// Cleared when the user resumes or retries sync.
pub(crate) static SYNC_RETRY_NOT_BEFORE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub(crate) const SYNC_RETRY_FIRST_DELAY: Duration = Duration::from_secs(30);
pub(crate) const SYNC_RETRY_MAX_DELAY: Duration = Duration::from_secs(15 * 60);

/// 30 s, 1 min, 2 min, ... up to 15 min for consecutive failed cycles.
pub(crate) fn sync_retry_delay(consecutive_failures: u32) -> Duration {
    let doublings = consecutive_failures.saturating_sub(1).min(10);
    (SYNC_RETRY_FIRST_DELAY * 2_u32.pow(doublings)).min(SYNC_RETRY_MAX_DELAY)
}

pub(crate) fn schedule_sync_retry(profile_id: &str, delay: Duration) {
    if let Ok(mut retries) = SYNC_RETRY_NOT_BEFORE.lock() {
        retries.insert(profile_id.to_string(), std::time::Instant::now() + delay);
    }
}

pub(crate) fn sync_retry_pending(profile_id: &str) -> bool {
    SYNC_RETRY_NOT_BEFORE
        .lock()
        .ok()
        .and_then(|retries| retries.get(profile_id).copied())
        .is_some_and(|not_before| std::time::Instant::now() < not_before)
}

pub(crate) fn clear_sync_retry(profile_id: &str) {
    if let Ok(mut retries) = SYNC_RETRY_NOT_BEFORE.lock() {
        retries.remove(profile_id);
    }
}

pub(crate) const SIGN_IN_REQUIRED_MESSAGE: &str =
    "Your Microsoft sign-in has expired. Sign in again to resume syncing.";

pub(crate) fn start_sync_worker(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let account_prefix = log_context::account_prefix(profile_id);
    if !sync_accepts_new_work() {
        // The saved agent state already says "syncing", so the updated
        // version starts this worker when it launches.
        log::info!(
            "{} SYNC_WORKER_START_DEFERRED reason=update_in_progress",
            account_prefix
        );
        return Ok(());
    }
    let cancel_flag = set_cancel_flag(state, profile_id, false)?;
    let cycle_lock = get_or_create_cycle_lock(state, profile_id)?;
    let initial_delay = remaining_until_next_cycle(profile_id, Duration::from_secs(15));
    {
        let mut stops = state
            .sync_worker_stops
            .lock()
            .map_err(|_| "Sync worker lock is poisoned".to_string())?;
        if stops.contains_key(profile_id) {
            log::info!("{} SYNC_WORKER_ALREADY_RUNNING", account_prefix);
            return Ok(());
        }
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        stops.insert(profile_id.to_string(), tx);
        log::info!(
            "{} SYNC_WORKER_STARTING interval_seconds=15",
            account_prefix
        );

        let profile_id_owned = profile_id.to_string();
        let profiles_lock = Arc::clone(&state.profiles_lock);
        let sync_runtime = Arc::clone(&state.sync_runtime);
        runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
        runtime_set_phase(
            &sync_runtime,
            &profile_id_owned,
            "syncing",
            "Preparing next sync cycle",
        );
        runtime_clear_issue(&sync_runtime, &profile_id_owned);
        let worker_task = tauri::async_runtime::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
            let mut consecutive_failures: u32 = 0;
            if let Some(delay) = initial_delay {
                log::info!(
                    "{} SYNC_WORKER_RESUME_DELAY wait_ms={}",
                    log_context::account_prefix(&profile_id_owned),
                    delay.as_millis()
                );
                if sleep_with_cancellation(&cancel_flag, delay).await.is_err() {
                    runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
                    runtime_set_phase(
                        &sync_runtime,
                        &profile_id_owned,
                        "paused",
                        "Synchronization paused",
                    );
                    return;
                }
            }
            loop {
                tokio::select! {
                    _ = &mut rx => {
                        log::info!("{} SYNC_WORKER_STOP_SIGNAL", log_context::account_prefix(&profile_id_owned));
                        if let Err(error) = reset_running_sync_jobs_for_pause(&profile_id_owned) {
                            log::warn!(
                                "{} SYNC_WORKER_STOP_DRAIN_FAILED error={}",
                                log_context::account_prefix(&profile_id_owned),
                                error
                            );
                        }
                        runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
                        runtime_set_phase(
                            &sync_runtime,
                            &profile_id_owned,
                            "paused",
                            "Synchronization paused",
                        );
                        break;
                    }
                    _ = ticker.tick() => {
                        if cancel_flag.load(Ordering::Relaxed)
                            || !sync_accepts_new_work()
                            || sync_retry_pending(&profile_id_owned)
                        {
                            continue;
                        }
                        let _cycle_guard = match cycle_lock.try_lock() {
                            Ok(guard) => guard,
                            Err(_) => {
                                let (active_phase, active_phase_message) = sync_runtime
                                    .lock()
                                    .ok()
                                    .and_then(|runtime_map| {
                                        runtime_map.get(&profile_id_owned).map(|status| {
                                            (status.phase.clone(), status.phase_message.clone())
                                        })
                                    })
                                    .unwrap_or_else(|| {
                                        ("unknown".to_string(), "unknown".to_string())
                                    });
                                log::warn!(
                                    "{} SYNC_TICK_SKIPPED cycle already running phase={} phase_message={}",
                                    log_context::account_prefix(&profile_id_owned),
                                    active_phase,
                                    active_phase_message
                                );
                                continue;
                            }
                        };
                        log::info!("{} SYNC_TICK", log_context::account_prefix(&profile_id_owned));
                        match tick_sync_cycle(&profiles_lock, &sync_runtime, &profile_id_owned, &cancel_flag).await {
                            Ok(stats) => {
                                if consecutive_failures > 0 {
                                    consecutive_failures = 0;
                                    clear_sync_retry(&profile_id_owned);
                                    runtime_clear_issue(&sync_runtime, &profile_id_owned);
                                }
                                log::info!(
                                    "{} [cycle:{}] SYNC_CYCLE_COMPLETE downloaded={} uploaded={} upload_failures={} matched={} baseline_refreshes={} conflict_copies={} local_deleted={} remote_deleted={} local_folders={} remote_folders={} remote_pages={} remote_items={} remote_unresolved={} local_items={} name_issues={}",
                                    stats.account_prefix,
                                    stats.cycle_id,
                                    stats.downloaded_files,
                                    stats.uploaded_files,
                                    stats.upload_failures,
                                    stats.matched_files,
                                    stats.baseline_refreshes,
                                    stats.conflict_copies,
                                    stats.deleted_local,
                                    stats.deleted_remote,
                                    stats.created_local_folders,
                                    stats.created_remote_folders,
                                    stats.remote_pages,
                                    stats.remote_items_received,
                                    stats.remote_items_skipped_unresolved,
                                    stats.local_items_seen,
                                    stats.name_issues,
                                );
                            }
                            Err(error) => {
                                if error == SyncError::Cancelled {
                                    log::info!(
                                        "{} SYNC_CYCLE_CANCELLED",
                                        log_context::account_prefix(&profile_id_owned)
                                    );
                                    if let Err(error) = reset_running_sync_jobs_for_pause(&profile_id_owned) {
                                        log::warn!(
                                            "{} SYNC_CANCEL_DRAIN_FAILED error={}",
                                            log_context::account_prefix(&profile_id_owned),
                                            error
                                        );
                                    }
                                    runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
                                    continue;
                                }
                                // Claimed jobs go back to the queue on every failure.
                                if let Err(drain_error) = reset_running_sync_jobs_for_pause(&profile_id_owned) {
                                    log::warn!(
                                        "{} SYNC_FAILURE_DRAIN_FAILED error={}",
                                        log_context::account_prefix(&profile_id_owned),
                                        drain_error
                                    );
                                }
                                let sign_in_required =
                                    !runtime_profile_auth_ready(&sync_runtime, &profile_id_owned);
                                log::error!(
                                    "{} SYNC_CYCLE_FAILED {}",
                                    log_context::account_prefix(&profile_id_owned),
                                    error
                                );
                                runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
                                let (issue_code, issue_actions, issue_message, phase_message) = if sign_in_required {
                                    // Only signing in again can fix this: stop until the user does.
                                    cancel_flag.store(true, Ordering::Relaxed);
                                    runtime_set_engine_state(&sync_runtime, &profile_id_owned, "blocked");
                                    (
                                        "auth_required",
                                        &["reauthenticate"][..],
                                        SIGN_IN_REQUIRED_MESSAGE.to_string(),
                                        "Sign-in required".to_string(),
                                    )
                                } else {
                                    // Anything else (network, OneDrive errors, an unmounted
                                    // folder) can clear by itself: retry with backoff.
                                    consecutive_failures = consecutive_failures.saturating_add(1);
                                    let delay = sync_retry_delay(consecutive_failures);
                                    schedule_sync_retry(&profile_id_owned, delay);
                                    let retry_at = chrono::Local::now()
                                        + chrono::Duration::from_std(delay).unwrap_or_default();
                                    log::warn!(
                                        "{} SYNC_CYCLE_RETRY_SCHEDULED attempt={} delay_s={}",
                                        log_context::account_prefix(&profile_id_owned),
                                        consecutive_failures,
                                        delay.as_secs()
                                    );
                                    let (issue_code, issue_actions) = error.issue();
                                    (
                                        issue_code,
                                        issue_actions,
                                        format!("{error} SomeDrive tries again at {}.", retry_at.format("%H:%M")),
                                        format!("Sync error - retrying at {}", retry_at.format("%H:%M")),
                                    )
                                };
                                runtime_set_phase(
                                    &sync_runtime,
                                    &profile_id_owned,
                                    "error",
                                    &phase_message,
                                );
                                runtime_set_issue(
                                    &sync_runtime,
                                    &profile_id_owned,
                                    issue_code,
                                    &issue_message,
                                    issue_actions,
                                    None,
                                    None,
                                );
                                let _ = activity_log::append_event(
                                    &profile_id_owned,
                                    &log_context::account_identity(&profile_id_owned),
                                    "error",
                                    &format!(
                                        "{} SYNC_CYCLE_FAILED {error}",
                                        log_context::account_prefix(&profile_id_owned)
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        });
        state
            .sync_worker_tasks
            .lock()
            .map_err(|_| "Sync worker task lock is poisoned".to_string())?
            .insert(profile_id.to_string(), worker_task);
    }

    let _ = activity_log::append_event(
        profile_id,
        &log_context::account_identity(profile_id),
        "info",
        &format!(
            "{} Sync agent started",
            log_context::account_prefix(profile_id)
        ),
    );
    Ok(())
}

pub(crate) fn stop_sync_worker(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(), String> {
    let _ = set_cancel_flag(state, profile_id, true)?;
    let maybe_sender = {
        let mut stops = state
            .sync_worker_stops
            .lock()
            .map_err(|_| "Sync worker lock is poisoned".to_string())?;
        stops.remove(profile_id)
    };

    if let Some(sender) = maybe_sender {
        let _ = sender.send(());
        let _ = activity_log::append_event(
            profile_id,
            &log_context::account_identity(profile_id),
            "info",
            &format!(
                "{} Sync agent stopped",
                log_context::account_prefix(profile_id)
            ),
        );
    }

    Ok(())
}
