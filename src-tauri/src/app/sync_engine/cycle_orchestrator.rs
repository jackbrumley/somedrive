use super::*;

// One sync cycle (docs/sync-engine.md section 5): scan this device, record
// cloud changes (downloading as pages arrive), plan from the baseline, then
// apply the plan. There is one mode from the first sync onwards.

/// Below this many synced files a missing sync folder cannot be told apart
/// from a small drive the user emptied, so the check does not apply.
pub(crate) const MISSING_ROOT_MIN_TRACKED_FILES: usize = 20;

pub(crate) async fn tick_sync_cycle(
    profiles_lock: &Arc<std::sync::Mutex<()>>,
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    local_watch: &mut LocalChangeWatch,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<SyncCycleStats> {
    ensure_not_cancelled(cancel_flag)?;
    let profile = load_syncable_profile(profiles_lock, profile_id)?;
    let account_prefix = log_context::account_prefix_from_parts(profile_id, &profile.email);
    let cycle_id = new_cycle_id();
    let sync_root = PathBuf::from(profile.sync_root.clone());
    ensure_sync_root_available(profile_id, &sync_root)?;

    let session = load_auth_session(profile_id).map_err(|error| {
        record_sign_in_required(profiles_lock, sync_runtime, profile_id, &error)
    })?;
    if session.access_token.trim().is_empty() {
        return Err(SyncError::from(record_sign_in_required(
            profiles_lock,
            sync_runtime,
            profile_id,
            "Saved access token is empty",
        )));
    }

    let mut graph = GraphContext {
        profile_id: profile_id.to_string(),
        account_prefix: account_prefix.clone(),
        cycle_id: cycle_id.clone(),
        access_token: Arc::new(tokio::sync::RwLock::new(session.access_token)),
        token_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        profiles_lock: Arc::clone(profiles_lock),
        sync_runtime: Arc::clone(sync_runtime),
        throttled_until: Arc::new(std::sync::Mutex::new(None)),
        exclusions: Arc::new(ExclusionRules::load()?),
        http: graph_http_client()?,
        http_no_redirect: graph_http_client_no_redirect()?,
        content_http: content_http_client()?,
        pending_dir_syncs: Arc::new(std::sync::Mutex::new(HashSet::new())),
        download_limit: Arc::new(AdaptiveDownloadLimit::new(
            resolve_download_concurrency(),
            read_learned_download_rate(profile_id)?,
        )),
        throttle_releases: Arc::new(AtomicUsize::new(0)),
        throttle_probe_until: Arc::new(std::sync::Mutex::new(None)),
    };
    let mut stats = SyncCycleStats {
        account_prefix: account_prefix.clone(),
        cycle_id: cycle_id.clone(),
        ..SyncCycleStats::default()
    };
    log::info!(
        "{} [cycle:{}] SYNC_CYCLE_START sync_root={}",
        account_prefix,
        cycle_id,
        sync_root.display()
    );
    let _ = activity_log::append_event(
        profile_id,
        &profile.email,
        "info",
        &format!("{} [cycle:{}] SYNC_CYCLE_START", account_prefix, cycle_id),
    );

    // Cloud renames recorded by an earlier, interrupted cycle.
    apply_pending_local_moves(profile_id, &account_prefix, &sync_root)?;

    // 1. This device first, so every download decision knows what is on
    // disk. The folder watcher says whether anything changed since the last
    // scan; with nothing changed, the database already holds this device's
    // state (the engine records its own transfers and deletes as it makes
    // them), so the scan is skipped.
    let scan_reason = local_watch.scan_decision(&sync_root, &graph.exclusions.patterns);
    if let Some(watching) = local_watch.unreported_watching_state() {
        runtime_set_local_watch(profile_id, watching);
    }
    match scan_reason {
        Some(reason) => {
            log::info!(
                "{} [cycle:{}] LOCAL_SCAN_REASON reason={}",
                account_prefix,
                cycle_id,
                reason.as_str()
            );
            scan_this_device(&graph, &sync_root, &mut stats, cancel_flag).await?;
        }
        None => log::info!(
            "{} [cycle:{}] LOCAL_SCAN_SKIPPED reason=no_changes_seen",
            account_prefix,
            cycle_id
        ),
    }

    // 2. Cloud changes; planned downloads start while pages arrive.
    fetch_and_apply_delta_changes(&mut graph, &sync_root, &mut stats, cancel_flag).await?;

    // 3. Plan everything from the baseline.
    runtime_set_phase(
        &graph.sync_runtime,
        profile_id,
        "planning_actions",
        "Planning sync actions",
    );
    let counters = recompute_sync_file_actions(profile_id)?;
    log::info!(
        "{} [cycle:{}] SYNC_PLANNER_SUMMARY cloud={} local={} shared_excluded={} download={} upload={} delete_remote={} delete_local={} conflicts={}",
        account_prefix,
        cycle_id,
        counters.cloud_discovered_total,
        counters.local_discovered_total,
        counters.shared_reference_total,
        counters.need_download_total,
        counters.need_upload_total,
        counters.need_delete_remote_total,
        counters.need_delete_local_total,
        counters.conflict_total
    );
    runtime_set_upload_planned_total(&graph.sync_runtime, profile_id, counters.need_upload_total);
    let planned = collect_planned_actions(profile_id)?;

    // 4. Apply it, starting with the user's conflict choices.
    runtime_set_phase(
        &graph.sync_runtime,
        profile_id,
        "applying_local",
        "Applying changes",
    );
    apply_conflict_resolutions(&mut graph, &sync_root, &mut stats, cancel_flag).await?;
    let planned_action_count = planned.verify.len()
        + planned.conflicts.len()
        + planned.create_local_folders.len()
        + planned.create_remote_folders.len()
        + planned.delete_local.len()
        + planned.delete_remote.len()
        + counters.need_upload_total;
    runtime_set_current_activity(
        &graph.sync_runtime,
        profile_id,
        "applying_local",
        "indeterminate",
        None,
        Some(planned_action_count),
        Some("items"),
        Some("Applying planned changes"),
        Some(&cycle_id),
    );
    forget_rows(profile_id, &planned.forget)?;
    create_local_folders(
        &graph,
        &sync_root,
        &planned.create_local_folders,
        &mut stats,
    )?;
    create_remote_folders(
        &mut graph,
        &sync_root,
        &planned.create_remote_folders,
        &mut stats,
        cancel_flag,
    )
    .await?;
    reconcile_both_sides(
        &graph,
        &sync_root,
        &planned.verify,
        ISSUE_CONFLICT_BOTH_NEW,
        &mut stats,
        cancel_flag,
    )
    .await?;
    reconcile_both_sides(
        &graph,
        &sync_root,
        &planned.conflicts,
        ISSUE_CONFLICT_BOTH_CHANGED,
        &mut stats,
        cancel_flag,
    )
    .await?;
    materialize_planner_upload_jobs(profile_id)?;
    upload_planned_files(&graph, &sync_root, &mut stats, cancel_flag).await?;
    apply_deletes_through_guard(&mut graph, &sync_root, &planned, &mut stats, cancel_flag).await?;

    let mut lifecycle_state = read_sync_lifecycle_operational_state(profile_id)?;
    lifecycle_state.last_cycle_at = Some(chrono::Local::now().to_rfc3339());
    persist_lifecycle_operational_state(profile_id, &lifecycle_state)?;
    update_profile_last_sync(profiles_lock, profile_id)?;

    let summary = format!(
        "Sync cycle complete (downloaded {}, uploaded {}, matched {}, refreshed {}, conflicts held {}, conflict copies {}, local moves {}, cloud deletes {}, local deletes {}, upload failures {}, name issues {})",
        stats.downloaded_files,
        stats.uploaded_files,
        stats.matched_files,
        stats.baseline_refreshes,
        stats.conflicts_held,
        stats.conflict_copies,
        stats.local_moves,
        stats.deleted_remote,
        stats.deleted_local,
        stats.upload_failures,
        stats.name_issues
    );
    let _ = activity_log::append_event(
        profile_id,
        &profile.email,
        "success",
        &format!("{} [cycle:{}] {}", account_prefix, cycle_id, summary),
    );
    runtime_set_phase(
        &graph.sync_runtime,
        profile_id,
        "idle",
        "Up to date - waiting for the next check",
    );
    Ok(stats)
}

/// Scans this device and records what is on disk.
async fn scan_this_device(
    graph: &GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let profile_id = graph.profile_id.as_str();
    runtime_set_phase(
        &graph.sync_runtime,
        profile_id,
        "scanning_local",
        "Checking this device",
    );
    let local_snapshot = run_local_scan_with_runtime_updates(
        &graph.account_prefix,
        &graph.cycle_id,
        &graph.sync_runtime,
        profile_id,
        sync_root,
        Arc::clone(&graph.exclusions),
        cancel_flag,
    )
    .await?;
    apply_local_scan(profile_id, &local_snapshot)?;
    apply_exclusion_rules(profile_id, &graph.exclusions)?;
    stats.local_items_seen = local_snapshot.len();
    stats.name_issues = record_local_name_issues(profile_id, &local_snapshot)?;
    log::info!(
        "{} [cycle:{}] LOCAL_SCAN_COMPLETE items={} name_issues={}",
        graph.account_prefix,
        graph.cycle_id,
        stats.local_items_seen,
        stats.name_issues
    );
    Ok(())
}

/// The sync folder must be there before anything is planned. Before the
/// first file is synced a missing folder is simply created; afterwards a
/// missing or empty folder (an unmounted drive, an empty mount point) stops
/// the cycle instead of being read as "every file deleted".
pub(crate) fn ensure_sync_root_available(profile_id: &str, sync_root: &Path) -> SyncResult<()> {
    let tracked_files = count_tracked_files(profile_id)?;
    let root_exists = sync_root.is_dir();
    let root_empty = root_exists
        && std::fs::read_dir(sync_root)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
    if tracked_files < MISSING_ROOT_MIN_TRACKED_FILES {
        if !root_exists {
            std::fs::create_dir_all(sync_root).map_err(|error| {
                SyncError::from_io(
                    &error,
                    format!(
                        "Failed to create sync root '{}': {}",
                        sync_root.display(),
                        error
                    ),
                )
            })?;
        }
        return Ok(());
    }
    if !root_exists || root_empty {
        return Err(SyncError::SyncRootUnavailable(format!(
            "Sync root unavailable: '{}' is {}, but {} files were synced there. Check the drive is mounted; nothing is deleted while the folder is unavailable.",
            sync_root.display(),
            if root_exists { "empty" } else { "missing" },
            tracked_files
        )));
    }
    Ok(())
}
