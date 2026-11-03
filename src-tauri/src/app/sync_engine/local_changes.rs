// each planned action list is passed explicitly; a cycle context struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
async fn apply_local_changes(
    graph: &mut GraphContext,
    sync_root: &Path,
    current_local_snapshot: &HashMap<String, LocalSnapshotEntry>,
    remote_applied_paths: &std::collections::HashSet<String>,
    planned_upload_paths: &std::collections::HashSet<String>,
    planned_delete_remote_paths: &std::collections::HashSet<String>,
    planned_delete_local_paths: &std::collections::HashSet<String>,
    planned_conflict_paths: &[String],
    sync_state: &mut PersistedSyncState,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
    const ACTIVITY_EMIT_INTERVAL: Duration = Duration::from_millis(500);

    runtime_set_phase(
        &graph.sync_runtime,
        &graph.profile_id,
        "applying_local",
        "Applying local changes",
    );
    let mut local_paths: Vec<String> = current_local_snapshot.keys().cloned().collect();
    local_paths.sort_by_key(|path| path.matches('/').count());
    let total_local_paths = local_paths.len();
    let mut local_paths_seen: usize = 0;
    let mut last_heartbeat_at = std::time::Instant::now();
    let mut last_activity_emit_at = std::time::Instant::now();
    runtime_set_current_activity(
        &graph.sync_runtime,
        &graph.profile_id,
        "applying_local",
        "determinate",
        Some(0),
        Some(total_local_paths),
        Some("paths"),
        Some("Evaluating local changes"),
        Some(&graph.cycle_id),
    );
    for path in local_paths {
        ensure_not_cancelled(cancel_flag)?;
        local_paths_seen += 1;

        if last_activity_emit_at.elapsed() >= ACTIVITY_EMIT_INTERVAL || local_paths_seen.is_multiple_of(200) {
            runtime_set_current_activity(
                &graph.sync_runtime,
                &graph.profile_id,
                "applying_local",
                "determinate",
                Some(local_paths_seen),
                Some(total_local_paths),
                Some("paths"),
                Some("Evaluating local changes"),
                Some(&graph.cycle_id),
            );
            last_activity_emit_at = std::time::Instant::now();
        }

        if last_heartbeat_at.elapsed() >= HEARTBEAT_INTERVAL {
            log::info!(
                "{} [cycle:{}] SYNC_HEARTBEAT phase=applying_local paths_seen={} uploaded={} upload_failures={} upload_cooldown_skips={}",
                graph.account_prefix,
                graph.cycle_id,
                local_paths_seen,
                stats.uploaded_files,
                stats.upload_failures,
                stats.upload_cooldown_skips
            );
            last_heartbeat_at = std::time::Instant::now();
        }

        if is_safe_backup_artifact(&path) {
            continue;
        }
        let Some(local_entry) = current_local_snapshot.get(&path) else {
            continue;
        };

        if local_entry.is_dir {
            if read_remote_item_id_for_path(&graph.profile_id, &path)?.is_none() {
                log::info!(
                    "{} [cycle:{}] REMOTE_DIR_CREATE_START path={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    path
                );
                let created = create_remote_folder(graph, &path, cancel_flag).await?;
                let known = remote_known_item_from_drive_item(created, &path)?;
                upsert_remote_known_item(sync_state, known);
                stats.created_remote_folders += 1;
            }
            continue;
        }

        if !planned_upload_paths.contains(&path) {
            continue;
        }

        if remote_applied_paths.contains(&path) {
            log::info!(
                "{} [cycle:{}] LOCAL_SKIP_REMOTE_APPLIED path={}",
                graph.account_prefix,
                graph.cycle_id,
                path
            );
            continue;
        }

        log::info!(
            "{} [cycle:{}] LOCAL_CHANGE path={} is_dir={} size={} modified_ts={}",
            graph.account_prefix,
            graph.cycle_id,
            path,
            local_entry.is_dir,
            local_entry.size,
            local_entry.modified_ts
        );

        log::info!(
            "{} [cycle:{}] LOCAL_UPLOAD_PLANNED path={} local_size={} local_modified_ts={}",
            graph.account_prefix,
            graph.cycle_id,
            path,
            local_entry.size,
            local_entry.modified_ts
        );
        // An upload already under way finishes; none start once an update
        // is being prepared.
        ensure_accepting_new_transfers()?;
        if !claim_upload_job_path(&graph.profile_id, &path, &graph.cycle_id)? {
            log::info!(
                "{} [cycle:{}] LOCAL_UPLOAD_SKIPPED_NOT_CLAIMED path={}",
                graph.account_prefix,
                graph.cycle_id,
                path
            );
            continue;
        }
        match upload_file_by_path(graph, sync_root, &path, cancel_flag).await {
            Ok(uploaded) => {
                let known = remote_known_item_from_drive_item(uploaded, &path)?;
                upsert_remote_known_item(sync_state, known);
                stats.uploaded_files += 1;
            }
            Err(error) => {
                stats.upload_failures += 1;
                log::warn!(
                    "{} [cycle:{}] LOCAL_UPLOAD_FAILED path={} reason={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    path,
                    error
                );
            }
        }
    }

    let claimed_delete_remote_paths = claim_action_job_paths(
        &graph.profile_id,
        DELETE_REMOTE_JOB_DIRECTION,
        &graph.cycle_id,
    )?;
    let mut deleted_paths: Vec<String> = planned_delete_remote_paths
        .intersection(&claimed_delete_remote_paths)
        .cloned()
        .collect();

    let mut guard_state = read_large_delete_guard_state(&graph.profile_id)?;
    let remote_deleted_paths = collect_remote_delete_candidate_paths(&graph.profile_id, &deleted_paths)?;
    let large_delete_guard_threshold = resolve_large_delete_guard_threshold();
    let large_delete_guard = resolve_large_delete_guard(
        &mut guard_state,
        deleted_paths,
        remote_deleted_paths,
        large_delete_guard_threshold,
    );
    persist_large_delete_guard_state(&graph.profile_id, &guard_state)?;
    if large_delete_guard.clear_issue {
        runtime_clear_issue(&graph.sync_runtime, &graph.profile_id);
    }
    if let Some(pending_count) = large_delete_guard.blocked_pending_count {
        let issue_message = large_delete_guard_issue_message(pending_count);
        runtime_set_issue(
            &graph.sync_runtime,
            &graph.profile_id,
            "large_delete_guard",
            &issue_message,
            &[
                "confirm_large_delete",
                "keep_cloud_files",
                "open_sync_root",
                "retry_sync",
            ],
            None,
            None,
        );
        runtime_set_phase(
            &graph.sync_runtime,
            &graph.profile_id,
            "paused",
            "Large deletion detected - review required",
        );
        if large_delete_guard.triggered_by_threshold {
            log::warn!(
                "{} [cycle:{}] LARGE_DELETE_GUARD_TRIGGERED count={} threshold={}",
                graph.account_prefix,
                graph.cycle_id,
                pending_count,
                large_delete_guard_threshold
            );
        } else {
            log::warn!(
                "{} [cycle:{}] LARGE_DELETE_GUARD_BLOCKING pending_count={}",
                graph.account_prefix,
                graph.cycle_id,
                pending_count
            );
        }
        return Ok(());
    }

    if large_delete_guard.confirmed_by_threshold {
        log::warn!(
            "{} [cycle:{}] LARGE_DELETE_GUARD_CONFIRMED count={} threshold={}",
            graph.account_prefix,
            graph.cycle_id,
            large_delete_guard.remote_deleted_count,
            large_delete_guard_threshold
        );
    }

    deleted_paths = large_delete_guard.deleted_paths;

    deleted_paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    let total_deleted_paths = deleted_paths.len();
    let mut deleted_paths_seen: usize = 0;

    runtime_set_current_activity(
        &graph.sync_runtime,
        &graph.profile_id,
        "applying_local",
        "determinate",
        Some(0),
        Some(total_deleted_paths),
        Some("paths"),
        Some("Reconciling remote deletions"),
        Some(&graph.cycle_id),
    );

    for deleted_path in deleted_paths {
        ensure_not_cancelled(cancel_flag)?;
        deleted_paths_seen += 1;
        mark_action_job_running(&graph.profile_id, DELETE_REMOTE_JOB_DIRECTION, &deleted_path)?;

        if last_activity_emit_at.elapsed() >= ACTIVITY_EMIT_INTERVAL || deleted_paths_seen.is_multiple_of(100) {
            runtime_set_current_activity(
                &graph.sync_runtime,
                &graph.profile_id,
                "applying_local",
                "determinate",
                Some(deleted_paths_seen),
                Some(total_deleted_paths),
                Some("paths"),
                Some("Reconciling remote deletions"),
                Some(&graph.cycle_id),
            );
            last_activity_emit_at = std::time::Instant::now();
        }
        if last_heartbeat_at.elapsed() >= HEARTBEAT_INTERVAL {
            log::info!(
                "{} [cycle:{}] SYNC_HEARTBEAT phase=applying_local delete_paths_seen={} remote_deleted={} local_deleted={}",
                graph.account_prefix,
                graph.cycle_id,
                deleted_paths_seen,
                stats.deleted_remote,
                stats.deleted_local
            );
            last_heartbeat_at = std::time::Instant::now();
        }
        if let Some(remote_id) = read_remote_item_id_for_path(&graph.profile_id, &deleted_path)? {
            log::info!(
                "{} [cycle:{}] REMOTE_DELETE_START path={} remote_id={}",
                graph.account_prefix,
                graph.cycle_id,
                deleted_path,
                remote_id
            );
            if let Err(error) = delete_remote_item(graph, &remote_id, cancel_flag).await {
                mark_action_job_failed(
                    &graph.profile_id,
                    DELETE_REMOTE_JOB_DIRECTION,
                    &deleted_path,
                    &error,
                )?;
                return Err(error);
            }
            sync_state.remote_path_to_id.remove(&deleted_path);
            sync_state.remote_by_id.remove(&remote_id);
            log::info!(
                "{} [cycle:{}] REMOTE_DELETE_OK path={} remote_id={}",
                graph.account_prefix,
                graph.cycle_id,
                deleted_path,
                remote_id
            );
            stats.deleted_remote += 1;
        }
        mark_action_job_done(
            &graph.profile_id,
            DELETE_REMOTE_JOB_DIRECTION,
            &deleted_path,
        )?;
    }

    let claimed_delete_local_paths = claim_action_job_paths(
        &graph.profile_id,
        DELETE_LOCAL_JOB_DIRECTION,
        &graph.cycle_id,
    )?;
    let mut local_delete_paths: Vec<String> = planned_delete_local_paths
        .intersection(&claimed_delete_local_paths)
        .cloned()
        .collect();
    local_delete_paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    let total_local_delete_paths = local_delete_paths.len();
    let mut local_delete_paths_seen: usize = 0;

    runtime_set_current_activity(
        &graph.sync_runtime,
        &graph.profile_id,
        "applying_local",
        "determinate",
        Some(0),
        Some(total_local_delete_paths),
        Some("paths"),
        Some("Reconciling local deletions"),
        Some(&graph.cycle_id),
    );

    for local_delete_path in local_delete_paths {
        ensure_not_cancelled(cancel_flag)?;
        local_delete_paths_seen = local_delete_paths_seen.saturating_add(1);
        mark_action_job_running(&graph.profile_id, DELETE_LOCAL_JOB_DIRECTION, &local_delete_path)?;
        if last_activity_emit_at.elapsed() >= ACTIVITY_EMIT_INTERVAL || local_delete_paths_seen.is_multiple_of(100) {
            runtime_set_current_activity(
                &graph.sync_runtime,
                &graph.profile_id,
                "applying_local",
                "determinate",
                Some(local_delete_paths_seen),
                Some(total_local_delete_paths),
                Some("paths"),
                Some("Reconciling local deletions"),
                Some(&graph.cycle_id),
            );
            last_activity_emit_at = std::time::Instant::now();
        }

        if last_heartbeat_at.elapsed() >= HEARTBEAT_INTERVAL {
            log::info!(
                "{} [cycle:{}] SYNC_HEARTBEAT phase=applying_local local_delete_paths_seen={} remote_deleted={} local_deleted={}",
                graph.account_prefix,
                graph.cycle_id,
                local_delete_paths_seen,
                stats.deleted_remote,
                stats.deleted_local
            );
            last_heartbeat_at = std::time::Instant::now();
        }

        log::info!(
            "{} [cycle:{}] LOCAL_DELETE_START path={}",
            graph.account_prefix,
            graph.cycle_id,
            local_delete_path
        );
        if let Err(error) = remove_local_path(sync_root, &local_delete_path) {
            mark_action_job_failed(
                &graph.profile_id,
                DELETE_LOCAL_JOB_DIRECTION,
                &local_delete_path,
                &error,
            )?;
            return Err(error);
        }
        sync_state.local_snapshot.remove(&local_delete_path);
        log::info!(
            "{} [cycle:{}] LOCAL_DELETE_OK path={}",
            graph.account_prefix,
            graph.cycle_id,
            local_delete_path
        );
        stats.deleted_local += 1;
        mark_action_job_done(
            &graph.profile_id,
            DELETE_LOCAL_JOB_DIRECTION,
            &local_delete_path,
        )?;
    }

    if !planned_conflict_paths.is_empty() {
        let claimed_conflict_paths = claim_action_job_paths(
            &graph.profile_id,
            CONFLICT_JOB_DIRECTION,
            &graph.cycle_id,
        )?;
        let mut conflict_paths: Vec<String> = planned_conflict_paths
            .iter()
            .filter(|path| claimed_conflict_paths.contains(path.as_str()))
            .cloned()
            .collect();
        conflict_paths.sort();
        let total_conflict_paths = conflict_paths.len();
        let mut conflict_paths_seen: usize = 0;
        runtime_set_current_activity(
            &graph.sync_runtime,
            &graph.profile_id,
            "applying_local",
            "determinate",
            Some(0),
            Some(total_conflict_paths),
            Some("paths"),
            Some("Resolving conflict backups"),
            Some(&graph.cycle_id),
        );

        for conflict_path in conflict_paths {
            ensure_not_cancelled(cancel_flag)?;
            conflict_paths_seen = conflict_paths_seen.saturating_add(1);
            mark_action_job_running(&graph.profile_id, CONFLICT_JOB_DIRECTION, &conflict_path)?;
            if last_activity_emit_at.elapsed() >= ACTIVITY_EMIT_INTERVAL || conflict_paths_seen.is_multiple_of(100) {
                runtime_set_current_activity(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    "applying_local",
                    "determinate",
                    Some(conflict_paths_seen),
                    Some(total_conflict_paths),
                    Some("paths"),
                    Some("Resolving conflict backups"),
                    Some(&graph.cycle_id),
                );
                last_activity_emit_at = std::time::Instant::now();
            }

            let local_abs = sync_root.join(path_to_local(&conflict_path));
            let backup_result = create_safe_backup(&local_abs);
            let Some(backup_path) = (match backup_result {
                Ok(value) => value,
                Err(error) => {
                    mark_action_job_failed(
                        &graph.profile_id,
                        CONFLICT_JOB_DIRECTION,
                        &conflict_path,
                        &error,
                    )?;
                    return Err(error);
                }
            }) else {
                mark_action_job_done(&graph.profile_id, CONFLICT_JOB_DIRECTION, &conflict_path)?;
                continue;
            };
            log::warn!(
                "{} [cycle:{}] CONFLICT_SAFE_BACKUP_CREATED path={} source={} backup={}",
                graph.account_prefix,
                graph.cycle_id,
                conflict_path,
                local_abs.display(),
                backup_path.display(),
            );
            let conflict_backup_relative = relative_path_for_issue(sync_root, &backup_path);
            runtime_set_issue(
                &graph.sync_runtime,
                &graph.profile_id,
                "conflict_detected",
                "Conflict detected. A safe backup was created.",
                &["open_conflict", "open_sync_root", "retry_sync"],
                Some(&conflict_path),
                conflict_backup_relative.as_deref(),
            );
            mark_action_job_done(&graph.profile_id, CONFLICT_JOB_DIRECTION, &conflict_path)?;
        }
    }

    Ok(())
}

async fn reconcile_bootstrap_local_snapshot(
    graph: &mut GraphContext,
    sync_root: &Path,
    sync_state: &mut PersistedSyncState,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut remote_items: Vec<RemoteKnownItem> = sync_state.remote_by_id.values().cloned().collect();
    remote_items.sort_by_key(|item| item.path.matches('/').count());
    let total_remote_items = remote_items.len();
    let mut remote_items_seen: usize = 0;
    let mut last_activity_emit_at = std::time::Instant::now();

    runtime_set_current_activity(
        &graph.sync_runtime,
        &graph.profile_id,
        "preparing_two_way_baseline",
        "determinate",
        Some(0),
        Some(total_remote_items),
        Some("items"),
        Some("Preparing two-way sync baseline"),
        Some(&graph.cycle_id),
    );

    for remote_item in remote_items {
        ensure_not_cancelled(cancel_flag)?;
        remote_items_seen = remote_items_seen.saturating_add(1);
        if last_activity_emit_at.elapsed() >= Duration::from_millis(500) || remote_items_seen.is_multiple_of(100) {
            runtime_set_current_activity(
                &graph.sync_runtime,
                &graph.profile_id,
                "preparing_two_way_baseline",
                "determinate",
                Some(remote_items_seen),
                Some(total_remote_items),
                Some("items"),
                Some("Preparing two-way sync baseline"),
                Some(&graph.cycle_id),
            );
            last_activity_emit_at = std::time::Instant::now();
        }
        if sync_state.local_snapshot.contains_key(&remote_item.path) {
            continue;
        }

        let local_abs = sync_root.join(path_to_local(&remote_item.path));
        if remote_item.is_dir {
            std::fs::create_dir_all(&local_abs).map_err(|error| {
                format!(
                    "Failed creating local directory '{}' during initial sync: {}",
                    local_abs.display(),
                    error
                )
            })?;
            continue;
        }

        log::info!(
            "{} [cycle:{}] INITIAL_SYNC_RESTORE path={} id={}",
            graph.account_prefix,
            graph.cycle_id,
            remote_item.path,
            remote_item.id
        );
        let outcome = download_remote_item_content(
            graph,
            None,
            &remote_item.id,
            &remote_item.path,
            Some(remote_item.size),
            &local_abs,
            cancel_flag,
        )
        .await?;

        match outcome {
            RemoteDownloadOutcome::Downloaded => {
                if let Some(local_entry) = read_local_entry(&local_abs)? {
                    sync_state.local_snapshot.insert(remote_item.path.clone(), local_entry);
                }
                stats.downloaded_files += 1;
            }
            RemoteDownloadOutcome::SkippedMissingRemote => {
                sync_state.remote_by_id.remove(&remote_item.id);
                sync_state.remote_path_to_id.remove(&remote_item.path);
                sync_state.local_snapshot.remove(&remote_item.path);
                stats.remote_items_skipped_missing += 1;
                log::warn!(
                    "{} [cycle:{}] INITIAL_SYNC_RESTORE_SKIPPED_MISSING path={} id={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    remote_item.path,
                    remote_item.id
                );
            }
        }
    }

    Ok(())
}

fn upsert_remote_known_item(sync_state: &mut PersistedSyncState, item: RemoteKnownItem) {
    sync_state
        .remote_path_to_id
        .insert(item.path.clone(), item.id.clone());
    sync_state.remote_by_id.insert(item.id.clone(), item);
}

fn is_safe_backup_artifact(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| name.contains("-safeBackup-"))
}

fn create_safe_backup(local_path: &Path) -> Result<Option<PathBuf>, String> {
    if !local_path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::metadata(local_path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Ok(None);
    }

    let parent = local_path
        .parent()
        .ok_or_else(|| "Local backup path has no parent".to_string())?;
    let file_name = local_path
        .file_name()
        .ok_or_else(|| "Local backup path has no filename".to_string())?
        .to_string_lossy()
        .to_string();

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut index = 1_u32;
    loop {
        let backup_name = format!("{}-safeBackup-{}-{:04}", file_name, stamp, index);
        let backup_path = parent.join(backup_name);
        if !backup_path.exists() {
            std::fs::copy(local_path, &backup_path).map_err(|error| {
                format!(
                    "Failed creating safe backup '{}' from '{}': {}",
                    backup_path.display(),
                    local_path.display(),
                    error
                )
            })?;
            return Ok(Some(backup_path));
        }
        index += 1;
    }
}

fn large_delete_guard_issue_message(pending_count: usize) -> String {
    format!(
        "Large deletion detected: {} items. Review before deleting from cloud.",
        pending_count
    )
}

struct LargeDeleteGuardResolution {
    deleted_paths: Vec<String>,
    blocked_pending_count: Option<usize>,
    triggered_by_threshold: bool,
    confirmed_by_threshold: bool,
    clear_issue: bool,
    remote_deleted_count: usize,
}

fn resolve_large_delete_guard(
    guard_state: &mut LargeDeleteGuardState,
    mut deleted_paths: Vec<String>,
    remote_deleted_paths: Vec<String>,
    threshold: usize,
) -> LargeDeleteGuardResolution {
    if guard_state.approved && !guard_state.pending_paths.is_empty() {
        deleted_paths = guard_state.pending_paths.clone();
    }

    if !guard_state.approved && !guard_state.pending_paths.is_empty() {
        return LargeDeleteGuardResolution {
            deleted_paths,
            blocked_pending_count: Some(guard_state.pending_paths.len()),
            triggered_by_threshold: false,
            confirmed_by_threshold: false,
            clear_issue: false,
            remote_deleted_count: 0,
        };
    }

    if remote_deleted_paths.len() >= threshold {
        if !guard_state.approved {
            guard_state.pending_paths = remote_deleted_paths;
            return LargeDeleteGuardResolution {
                deleted_paths,
                blocked_pending_count: Some(guard_state.pending_paths.len()),
                triggered_by_threshold: true,
                confirmed_by_threshold: false,
                clear_issue: false,
                remote_deleted_count: 0,
            };
        }

        guard_state.approved = false;
        guard_state.pending_paths.clear();
        return LargeDeleteGuardResolution {
            deleted_paths,
            blocked_pending_count: None,
            triggered_by_threshold: false,
            confirmed_by_threshold: true,
            clear_issue: true,
            remote_deleted_count: remote_deleted_paths.len(),
        };
    }

    if guard_state.approved {
        guard_state.approved = false;
        guard_state.pending_paths.clear();
        return LargeDeleteGuardResolution {
            deleted_paths,
            blocked_pending_count: None,
            triggered_by_threshold: false,
            confirmed_by_threshold: false,
            clear_issue: true,
            remote_deleted_count: remote_deleted_paths.len(),
        };
    }

    LargeDeleteGuardResolution {
        deleted_paths,
        blocked_pending_count: None,
        triggered_by_threshold: false,
        confirmed_by_threshold: false,
        clear_issue: false,
        remote_deleted_count: remote_deleted_paths.len(),
    }
}

fn collect_remote_delete_candidate_paths(
    profile_id: &str,
    deleted_paths: &[String],
) -> Result<Vec<String>, String> {
    let mut remote_deleted_paths: Vec<String> = Vec::new();
    for path in deleted_paths {
        if read_remote_item_id_for_path(profile_id, path)?.is_some() {
            remote_deleted_paths.push(path.clone());
        }
    }
    Ok(remote_deleted_paths)
}

fn remove_local_path(sync_root: &Path, relative_path: &str) -> Result<(), String> {
    let full_path = sync_root.join(path_to_local(relative_path));
    if !full_path.exists() {
        return Ok(());
    }
    let metadata = std::fs::metadata(&full_path).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&full_path).map_err(|error| {
            format!(
                "Failed removing directory '{}': {}",
                full_path.display(),
                error
            )
        })
    } else {
        std::fs::remove_file(&full_path)
            .map_err(|error| format!("Failed removing file '{}': {}", full_path.display(), error))
    }
}

fn relative_path_for_issue(sync_root: &Path, candidate: &Path) -> Option<String> {
    let relative = candidate.strip_prefix(sync_root).ok()?;
    let mut output = String::new();
    for component in relative.components() {
        if let std::path::Component::Normal(segment) = component {
            if !output.is_empty() {
                output.push('/');
            }
            output.push_str(&segment.to_string_lossy());
        }
    }
    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

fn resolve_delta_item_path(item: &DeltaItem) -> Option<String> {
    let name = item.name.as_ref()?.trim();
    if name.is_empty() {
        return None;
    }
    let base = item
        .parent_reference
        .as_ref()
        .and_then(|reference| reference.path.as_deref())
        .map(extract_root_relative)
        .unwrap_or_default();
    let combined = if base.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", base, name)
    };
    Some(normalize_relative_path(&combined))
}

fn extract_root_relative(parent_path: &str) -> String {
    let mut value = parent_path.trim().to_string();
    if let Some(rest) = value.strip_prefix("/drive/root:") {
        value = rest.to_string();
    }
    value.trim_start_matches('/').to_string()
}

fn normalize_relative_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn path_to_local(relative_path: &str) -> PathBuf {
    let mut output = PathBuf::new();
    for segment in relative_path.split('/') {
        if !segment.is_empty() {
            output.push(segment);
        }
    }
    output
}

fn parse_rfc3339_seconds(value: Option<&str>) -> i64 {
    value
        .and_then(|input| chrono::DateTime::parse_from_rfc3339(input).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or(0)
}

fn shared_drive_id_from_delta_item(item: &DeltaItem) -> Option<String> {
    item.remote_item
        .as_ref()
        .and_then(|remote_item| remote_item.get("parentReference"))
        .and_then(|parent_reference| parent_reference.get("driveId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn shared_item_id_from_delta_item(item: &DeltaItem) -> Option<String> {
    item.remote_item
        .as_ref()
        .and_then(|remote_item| remote_item.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn shared_kind_from_delta_item(item: &DeltaItem) -> Option<String> {
    let remote_item = item.remote_item.as_ref()?;
    if remote_item.get("folder").is_some() {
        return Some("folder".to_string());
    }
    if remote_item.get("file").is_some() {
        return Some("file".to_string());
    }
    if remote_item.get("package").is_some() {
        return Some("package".to_string());
    }
    Some("unknown".to_string())
}
