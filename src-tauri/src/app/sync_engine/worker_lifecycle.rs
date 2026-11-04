const SIGN_IN_REQUIRED_MESSAGE: &str =
    "Your Microsoft sign-in has expired. Sign in again to resume syncing.";

fn start_sync_worker(state: &tauri::State<'_, AppState>, profile_id: &str) -> Result<(), String> {
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
                        if cancel_flag.load(Ordering::Relaxed) || !sync_accepts_new_work() {
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
                                log::info!(
                                    "{} [cycle:{}] SYNC_CYCLE_COMPLETE downloaded={} uploaded={} upload_failures={} upload_cooldown_skips={} local_deleted={} remote_deleted={} remote_folders={} remote_pages={} remote_items={} remote_missing_skips={} local_items={}",
                                    stats.account_prefix,
                                    stats.cycle_id,
                                    stats.downloaded_files,
                                    stats.uploaded_files,
                                    stats.upload_failures,
                                    stats.upload_cooldown_skips,
                                    stats.deleted_local,
                                    stats.deleted_remote,
                                    stats.created_remote_folders,
                                    stats.remote_pages,
                                    stats.remote_items_received,
                                    stats.remote_items_skipped_missing,
                                    stats.local_items_seen,
                                );
                            }
                            Err(error) => {
                                if is_sync_cancelled_error(&error) {
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
                                let sign_in_required =
                                    !runtime_profile_auth_ready(&sync_runtime, &profile_id_owned);
                                let (issue_code, issue_actions, issue_message, phase_message) =
                                    if sign_in_required {
                                        (
                                            "auth_required",
                                            &["reauthenticate"][..],
                                            SIGN_IN_REQUIRED_MESSAGE.to_string(),
                                            "Sign-in required".to_string(),
                                        )
                                    } else {
                                        let (issue_code, issue_actions) = classify_sync_issue(&error);
                                        (issue_code, issue_actions, error.clone(), format!("Sync error: {}", error))
                                    };
                                log::error!(
                                    "{} SYNC_CYCLE_FAILED {}",
                                    log_context::account_prefix(&profile_id_owned),
                                    error
                                );
                                cancel_flag.store(true, Ordering::Relaxed);
                                runtime_reset_transfer_activity(&sync_runtime, &profile_id_owned);
                                runtime_set_engine_state(&sync_runtime, &profile_id_owned, "blocked");
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

fn stop_sync_worker(state: &tauri::State<'_, AppState>, profile_id: &str) -> Result<(), String> {
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
