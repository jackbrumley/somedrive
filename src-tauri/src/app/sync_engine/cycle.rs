async fn run_local_scan_with_runtime_updates(
    account_prefix: &str,
    cycle_id: &str,
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    sync_root: &Path,
    estimated_total: Option<usize>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<HashMap<String, LocalEntry>, String> {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Mutex;

    let scanned_count = Arc::new(AtomicUsize::new(0));
    let last_progress_unix = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp()));
    let current_path = Arc::new(Mutex::new(None::<String>));

    let thread_scanned_count = Arc::clone(&scanned_count);
    let thread_last_progress_unix = Arc::clone(&last_progress_unix);
    let thread_current_path = Arc::clone(&current_path);
    let sync_root_owned = sync_root.to_path_buf();

    let mut scan_handle = tauri::async_runtime::spawn_blocking(move || {
        collect_local_snapshot_with_progress(&sync_root_owned, |relative_path| {
            thread_scanned_count.fetch_add(1, AtomicOrdering::Relaxed);
            thread_last_progress_unix.store(chrono::Utc::now().timestamp(), AtomicOrdering::Relaxed);
            if let Ok(mut value) = thread_current_path.lock() {
                *value = Some(relative_path.to_string());
            }
        })
    });

    let mut last_emit_at = std::time::Instant::now();
    let mut last_progress_log_at = std::time::Instant::now();
    let scan_started_at = std::time::Instant::now();
    let mut last_stall_warn_at: Option<std::time::Instant> = None;
    log::info!(
        "{} [cycle:{}] LOCAL_SCAN_START sync_root={}",
        account_prefix,
        cycle_id,
        sync_root.display()
    );
    runtime_set_local_scan_progress(sync_runtime, profile_id, 0, estimated_total, None, Some(cycle_id));

    loop {
        ensure_not_cancelled(cancel_flag)?;
        tokio::select! {
            scan_result = &mut scan_handle => {
                let local_snapshot = scan_result
                    .map_err(|error| format!("Local scan task failed: {error}"))??;
                log::info!(
                    "{} [cycle:{}] LOCAL_SCAN_COMPLETE scanned={} duration_ms={}",
                    account_prefix,
                    cycle_id,
                    local_snapshot.len(),
                    scan_started_at.elapsed().as_millis()
                );
                runtime_set_local_scan_progress(
                    sync_runtime,
                    profile_id,
                    local_snapshot.len(),
                    Some(local_snapshot.len()),
                    None,
                    Some(cycle_id),
                );
                return Ok(local_snapshot);
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let now_unix = chrono::Utc::now().timestamp();
                let last_progress = last_progress_unix.load(AtomicOrdering::Relaxed);
                let stalled_seconds = now_unix.saturating_sub(last_progress);
                let scanned = scanned_count.load(AtomicOrdering::Relaxed);
                let path = current_path.lock().ok().and_then(|value| value.clone());

                if last_emit_at.elapsed() >= Duration::from_millis(350) {
                    runtime_set_local_scan_progress(
                        sync_runtime,
                        profile_id,
                        scanned,
                        estimated_total,
                        path.as_deref(),
                        Some(cycle_id),
                    );
                    last_emit_at = std::time::Instant::now();
                }

                if last_progress_log_at.elapsed() >= Duration::from_secs(10) {
                    log::info!(
                        "{} [cycle:{}] LOCAL_SCAN_PROGRESS scanned={} elapsed_s={} current_path={}",
                        account_prefix,
                        cycle_id,
                        scanned,
                        scan_started_at.elapsed().as_secs(),
                        path.as_deref().unwrap_or("(none)")
                    );
                    last_progress_log_at = std::time::Instant::now();
                }

                if stalled_seconds >= 60 {
                    let should_warn = last_stall_warn_at
                        .as_ref()
                        .is_none_or(|value| value.elapsed() >= Duration::from_secs(30));
                    if should_warn {
                        log::warn!(
                            "{} [cycle:{}] LOCAL_SCAN_STALLED stalled_for={}s scanned={} current_path={}",
                            account_prefix,
                            cycle_id,
                            stalled_seconds,
                            scanned,
                            path.as_deref().unwrap_or("(none)")
                        );
                        runtime_set_phase(
                            sync_runtime,
                            profile_id,
                            "scanning_local",
                            &format!(
                                "Local scan is taking longer than expected (scanned {} items)",
                                scanned
                            ),
                        );
                        last_stall_warn_at = Some(std::time::Instant::now());
                    }
                }
            }
        }
    }
}

fn new_cycle_id() -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let pid = std::process::id();
    format!("{}-{}", nanos, pid)
}

fn remaining_until_next_cycle(profile_id: &str, interval: Duration) -> Option<Duration> {
    let lifecycle_state = read_sync_lifecycle_operational_state(profile_id).ok()?;
    let last_cycle_at = lifecycle_state.last_cycle_at?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&last_cycle_at).ok()?;
    let elapsed = chrono::Utc::now().signed_duration_since(timestamp.with_timezone(&chrono::Utc));
    if elapsed.num_milliseconds() <= 0 {
        return Some(interval);
    }
    let elapsed_duration = Duration::from_millis(elapsed.num_milliseconds() as u64);
    if elapsed_duration >= interval {
        return None;
    }
    Some(interval - elapsed_duration)
}

fn load_syncable_profile(
    profiles_lock: &Arc<std::sync::Mutex<()>>,
    profile_id: &str,
) -> Result<AccountProfile, String> {
    let _guard = profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let profiles = load_profiles()?;
    let profile = profiles
        .into_iter()
        .find(|entry| entry.id == profile_id)
        .ok_or_else(|| "Account profile not found".to_string())?;
    if profile.agent_state != "syncing" {
        return Err("Account is not in syncing state".to_string());
    }
    if !profile.auth_configured {
        return Err("Account is not authenticated".to_string());
    }
    Ok(profile)
}

fn update_profile_last_sync(
    profiles_lock: &Arc<std::sync::Mutex<()>>,
    profile_id: &str,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();
    persist_sync_lifecycle_last_sync_at(profile_id, Some(&now))?;

    let _guard = profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let mut profiles = load_profiles()?;
    let profile = profiles
        .iter_mut()
        .find(|entry| entry.id == profile_id)
        .ok_or_else(|| "Account profile not found".to_string())?;
    profile.last_sync_at = Some(now);
    save_profiles(&profiles)
}
