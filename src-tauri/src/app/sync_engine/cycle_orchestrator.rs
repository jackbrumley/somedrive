async fn tick_sync_cycle(
    profiles_lock: &Arc<std::sync::Mutex<()>>,
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<SyncCycleStats, String> {
    ensure_not_cancelled(cancel_flag)?;
    let profile = load_syncable_profile(profiles_lock, profile_id)?;
    let account_prefix = log_context::account_prefix_from_parts(profile_id, &profile.email);
    let cycle_id = new_cycle_id();
    let sync_root = PathBuf::from(profile.sync_root.clone());
    std::fs::create_dir_all(&sync_root).map_err(|error| {
        format!(
            "Failed to create sync root '{}': {}",
            sync_root.display(),
            error
        )
    })?;

    let session = load_auth_session(profile_id)
        .map_err(|error| record_sign_in_required(profiles_lock, sync_runtime, profile_id, &error))?;
    if session.access_token.trim().is_empty() {
        return Err(record_sign_in_required(
            profiles_lock,
            sync_runtime,
            profile_id,
            "Saved access token is empty",
        ));
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
    };

    runtime_set_phase(
        &graph.sync_runtime,
        profile_id,
        "syncing",
        "Preparing synchronization cycle",
    );

    let mut sync_state = load_sync_state(profile_id)?;
    let mut lifecycle_state = read_sync_lifecycle_operational_state(profile_id)?;
    let started_two_way_ready = lifecycle_state.two_way_ready;
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

    let remote_applied_paths = fetch_and_apply_delta_changes(
        &mut graph,
        &sync_root,
        &mut sync_state,
        &mut stats,
        cancel_flag,
    )
    .await?;

    lifecycle_state = read_sync_lifecycle_operational_state(profile_id)?;

    if lifecycle_state.two_way_ready {
        runtime_set_phase(
            &graph.sync_runtime,
            profile_id,
            "scanning_local",
            "Scanning local files",
        );
        log::info!(
            "{} [cycle:{}] STAGE_START stage=scanning_local sync_root={}",
            account_prefix,
            cycle_id,
            sync_root.display()
        );
        let local_scan_estimated_total = if sync_state.local_snapshot.is_empty() {
            None
        } else {
            Some(sync_state.local_snapshot.len())
        };
        let local_snapshot = run_local_scan_with_runtime_updates(
            &graph.account_prefix,
            &graph.cycle_id,
            &graph.sync_runtime,
            profile_id,
            &sync_root,
            local_scan_estimated_total,
            cancel_flag,
        )
        .await?;
        log::info!(
            "{} [cycle:{}] STAGE_COMPLETE stage=scanning_local local_items={}",
            account_prefix,
            cycle_id,
            local_snapshot.len()
        );
        let index_started_at = std::time::Instant::now();
        runtime_set_phase(
            &graph.sync_runtime,
            profile_id,
            "building_index",
            "Building sync index",
        );
        log::info!(
            "{} [cycle:{}] STAGE_START stage=index_rebuild remote_items={} local_items={}",
            account_prefix,
            cycle_id,
            sync_state.remote_by_id.len(),
            local_snapshot.len()
        );
        rebuild_sync_file_index(
            &graph.sync_runtime,
            profile_id,
            &sync_state,
            &local_snapshot,
            &graph.account_prefix,
            &graph.cycle_id,
        )?;
        log::info!(
            "{} [cycle:{}] STAGE_COMPLETE stage=index_rebuild duration_ms={}",
            account_prefix,
            cycle_id,
            index_started_at.elapsed().as_millis()
        );
        let planner_started_at = std::time::Instant::now();
        runtime_set_phase(
            &graph.sync_runtime,
            profile_id,
            "planning_actions",
            "Planning sync actions",
        );
        log::info!(
            "{} [cycle:{}] STAGE_START stage=planner two_way_ready={}",
            account_prefix,
            cycle_id,
            lifecycle_state.two_way_ready
        );
        let planner_counters =
            recompute_sync_file_actions(profile_id, lifecycle_state.two_way_ready)?;
        let materialized = materialize_planner_actions(profile_id, &account_prefix, &cycle_id)?;
        log::info!(
            "{} [cycle:{}] STAGE_COMPLETE stage=planner duration_ms={} need_download={} need_upload={} conflicts={}",
            account_prefix,
            cycle_id,
            planner_started_at.elapsed().as_millis(),
            planner_counters.need_download_total,
            planner_counters.need_upload_total,
            planner_counters.conflict_total
        );
        runtime_set_upload_planned_total(
            &graph.sync_runtime,
            profile_id,
            planner_counters.need_upload_total,
        );
        log::info!(
            "{} [cycle:{}] SYNC_PLANNER_SUMMARY cloud_discovered={} local_discovered={} need_download={} need_upload={} need_delete_remote={} need_delete_local={} conflicts={} shared_references_excluded={} desired_download_jobs={} desired_upload_jobs={} desired_delete_remote_paths={} desired_delete_local_paths={} desired_conflict_paths={} active_download_jobs={} active_upload_jobs={}",
            account_prefix,
            cycle_id,
            planner_counters.cloud_discovered_total,
            planner_counters.local_discovered_total,
            planner_counters.need_download_total,
            planner_counters.need_upload_total,
            planner_counters.need_delete_remote_total,
            planner_counters.need_delete_local_total,
            planner_counters.conflict_total,
            planner_counters.shared_reference_total,
            materialized.desired_download_paths,
            materialized.desired_upload_paths,
            materialized.desired_delete_remote_paths,
            materialized.desired_delete_local_paths,
            materialized.desired_conflict_paths,
            materialized.active_download_jobs,
            materialized.active_upload_jobs,
        );
        if planner_counters.need_download_total > 0 && materialized.active_download_jobs == 0 {
            log::warn!(
                "{} [cycle:{}] PLANNER_DOWNLOAD_MATERIALIZATION_GAP need_download={} active_download_jobs={}",
                account_prefix,
                cycle_id,
                planner_counters.need_download_total,
                materialized.active_download_jobs,
            );
        }
        if planner_counters.need_upload_total != materialized.upload_paths.len() {
            log::warn!(
                "{} [cycle:{}] PLANNER_UPLOAD_MATERIALIZATION_GAP need_upload={} upload_paths={}",
                account_prefix,
                cycle_id,
                planner_counters.need_upload_total,
                materialized.upload_paths.len(),
            );
        }
        if !materialized.conflict_paths.is_empty() {
            let conflict_sample = materialized
                .conflict_paths
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ");
            log::warn!(
                "{} [cycle:{}] PLANNER_CONFLICT_PATHS count={} sample_paths={}",
                account_prefix,
                cycle_id,
                materialized.conflict_paths.len(),
                if conflict_sample.is_empty() {
                    "(none)".to_string()
                } else {
                    conflict_sample
                }
            );
        }
        stats.local_items_seen = local_snapshot.len();
        log::info!(
            "{} [cycle:{}] LOCAL_SCAN_SUMMARY items={}",
            account_prefix,
            cycle_id,
            stats.local_items_seen
        );
        runtime_set_phase(
            &graph.sync_runtime,
            profile_id,
            "applying_local",
            "Applying local changes",
        );
        ensure_not_cancelled(cancel_flag)?;
        apply_local_changes(
            &mut graph,
            &sync_root,
            &local_snapshot,
            &remote_applied_paths,
            &materialized.upload_paths,
            &materialized.delete_remote_paths,
            &materialized.delete_local_paths,
            &materialized.conflict_paths,
            &mut sync_state,
            &mut stats,
            cancel_flag,
        )
        .await?;
    } else {
        let download_counters = read_download_job_counters(profile_id)?;
        runtime_set_upload_planned_total(&graph.sync_runtime, profile_id, 0);
        let bootstrap_ready_for_two_way =
            bootstrap_ready_for_two_way(&lifecycle_state, &download_counters);

        if bootstrap_ready_for_two_way {
            runtime_set_phase(
                &graph.sync_runtime,
                profile_id,
                "preparing_two_way_baseline",
                "Preparing two-way sync - building your local baseline",
            );
            ensure_not_cancelled(cancel_flag)?;
            reconcile_bootstrap_local_snapshot(
                &mut graph,
                &sync_root,
                &mut sync_state,
                &mut stats,
                cancel_flag,
            )
            .await?;
            lifecycle_state.two_way_ready = true;
            persist_lifecycle_operational_state(profile_id, &lifecycle_state)?;
        } else {
            let blocked_message = if download_counters.failed_terminal > 0 {
                format!(
                    "Initial sync blocked: {} failed download{} need retry before two-way sync",
                    download_counters.failed_terminal,
                    if download_counters.failed_terminal == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            } else {
                "Initial sync in progress - downloading cloud files only".to_string()
            };
            runtime_set_phase(&graph.sync_runtime, profile_id, "syncing", &blocked_message);
            if download_counters.failed_terminal > 0 {
                let failed_samples = list_terminal_failed_download_paths(profile_id, 5)
                    .unwrap_or_else(|_| Vec::new());
                log::warn!(
                    "{} [cycle:{}] TWO_WAY_BLOCK_REASON reason=failed_terminal_downloads count={} sample_paths={}",
                    account_prefix,
                    cycle_id,
                    download_counters.failed_terminal,
                    if failed_samples.is_empty() {
                        "(none)".to_string()
                    } else {
                        failed_samples.join(" | ")
                    }
                );
            }
            log::warn!(
                "{} [cycle:{}] BOOTSTRAP_TWO_WAY_BLOCKED cursor_active={} bootstrap_full_scan_completed={} queue_remaining={} failed_terminal={}",
                account_prefix,
                cycle_id,
                lifecycle_state.active_delta_next_link.is_some(),
                lifecycle_state.bootstrap_full_scan_completed,
                download_counters.remaining,
                download_counters.failed_terminal
            );
        }
    }

    if started_two_way_ready {
        sync_state.local_snapshot = collect_local_snapshot(&sync_root)?;
    }
    lifecycle_state.last_cycle_at = Some(chrono::Local::now().to_rfc3339());
    persist_lifecycle_operational_state(profile_id, &lifecycle_state)?;
    save_sync_state(profile_id, &sync_state)?;

    update_profile_last_sync(profiles_lock, profile_id)?;

    let summary = format!(
        "Sync cycle complete (downloaded {}, uploaded {}, upload failures {}, upload cooldown skips {}, remote deletes {}, local deletes {}, remote pages {}, remote items {}, remote missing skips {}, local items {})",
        stats.downloaded_files,
        stats.uploaded_files,
        stats.upload_failures,
        stats.upload_cooldown_skips,
        stats.deleted_remote,
        stats.deleted_local,
        stats.remote_pages,
        stats.remote_items_received,
        stats.remote_items_skipped_missing,
        stats.local_items_seen
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
        "Idle - waiting for next sync cycle",
    );
    Ok(stats)
}

fn bootstrap_ready_for_two_way(
    lifecycle_state: &SyncLifecycleOperationalState,
    download_counters: &DownloadJobCounters,
) -> bool {
    lifecycle_state.active_delta_next_link.is_none()
        && lifecycle_state.bootstrap_full_scan_completed
        && download_counters.remaining == 0
        && download_counters.failed_terminal == 0
}

#[cfg(test)]
mod cycle_orchestrator_tests {
    use super::*;

    #[test]
    fn bootstrap_gate_blocks_when_cursor_or_queue_or_failures_exist() {
        let base_lifecycle = SyncLifecycleOperationalState {
            bootstrap_full_scan_completed: true,
            ..SyncLifecycleOperationalState::default()
        };
        let base_downloads = DownloadJobCounters::default();

        let mut cursor_active = base_lifecycle.clone();
        cursor_active.active_delta_next_link = Some("https://example.test/next".to_string());
        assert!(!bootstrap_ready_for_two_way(&cursor_active, &base_downloads));

        let mut queue_remaining = base_downloads.clone();
        queue_remaining.remaining = 1;
        assert!(!bootstrap_ready_for_two_way(&base_lifecycle, &queue_remaining));

        let mut failed_terminal = base_downloads;
        failed_terminal.failed_terminal = 1;
        assert!(!bootstrap_ready_for_two_way(&base_lifecycle, &failed_terminal));
    }

    #[test]
    fn bootstrap_gate_allows_transition_when_requirements_clear() {
        let lifecycle = SyncLifecycleOperationalState {
            bootstrap_full_scan_completed: true,
            active_delta_next_link: None,
            ..SyncLifecycleOperationalState::default()
        };
        let downloads = DownloadJobCounters {
            remaining: 0,
            failed_terminal: 0,
            ..DownloadJobCounters::default()
        };
        assert!(bootstrap_ready_for_two_way(&lifecycle, &downloads));
    }
}
