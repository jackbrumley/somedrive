use super::*;

// Applies a cycle's plan (docs/sync-engine.md sections 4 and 6). Every action
// re-checks the file it is about to touch, so a change made since the plan
// is never overwritten or deleted: the action is skipped and the next cycle
// plans again from the new state.

pub(crate) const DELETE_EVENT_DIRECTION: &str = "delete";

pub(crate) async fn hash_local_file(path: &Path) -> SyncResult<String> {
    let owned = path.to_path_buf();
    let hash = tauri::async_runtime::spawn_blocking(move || quick_xor_hash_file(&owned))
        .await
        .map_err(|error| format!("Hashing task failed: {error}"))??;
    Ok(hash)
}

pub(crate) fn local_path_for(sync_root: &Path, relative_path: &str) -> PathBuf {
    sync_root.join(path_to_local(relative_path))
}

pub(crate) fn forget_rows(profile_id: &str, rows: &[SyncFileRow]) -> SyncResult<()> {
    for row in rows {
        forget_sync_file(profile_id, &row.path)?;
    }
    Ok(())
}

pub(crate) fn create_local_folders(
    graph: &GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    stats: &mut SyncCycleStats,
) -> SyncResult<()> {
    for row in rows {
        let local_path = local_path_for(sync_root, &row.path);
        std::fs::create_dir_all(&local_path).map_err(|error| {
            SyncError::from_io(
                &error,
                format!("Failed creating folder '{}': {error}", local_path.display()),
            )
        })?;
        if let Some(entry) = read_local_entry(&local_path)? {
            commit_baseline(&graph.profile_id, &row.path, &row.cloud_version(), &entry)?;
            stats.created_local_folders += 1;
        }
    }
    Ok(())
}

pub(crate) async fn create_remote_folders(
    graph: &mut GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    for row in rows {
        ensure_accepting_new_transfers()?;
        let Some(entry) = read_local_entry(&local_path_for(sync_root, &row.path))? else {
            continue;
        };
        let created = create_remote_folder(graph, &row.path, cancel_flag).await?;
        commit_baseline_from_upload(
            &graph.profile_id,
            &created.to_cloud_item(row.path.clone()),
            &entry,
        )?;
        stats.created_remote_folders += 1;
    }
    Ok(())
}

pub(crate) fn conflict_detail(conflict_code: &str) -> &'static str {
    if conflict_code == ISSUE_CONFLICT_BOTH_NEW {
        "This file is on this device and in OneDrive with different content."
    } else {
        "This file changed on this device and in OneDrive since the last sync."
    }
}

/// Holds `path` as a conflict: it stops syncing until it is resolved. With
/// the "keep the newest" setting the newer side is chosen straight away (and
/// applied next cycle, through the same checks as a user's choice).
pub(crate) fn hold_conflict(
    graph: &GraphContext,
    path: &str,
    conflict_code: &str,
    local_mtime_ns: i64,
    cloud_mtime: i64,
) -> SyncResult<()> {
    record_sync_file_issue(
        &graph.profile_id,
        path,
        conflict_code,
        conflict_detail(conflict_code),
    )?;
    log::warn!(
        "{} [cycle:{}] CONFLICT_HELD path={} code={}",
        graph.account_prefix,
        graph.cycle_id,
        path,
        conflict_code
    );
    if crate::app::app_settings::load_conflict_policy()?
        == crate::app::app_settings::ConflictPolicy::KeepNewest
    {
        let resolution = if local_mtime_ns / 1_000_000_000 > cloud_mtime {
            RESOLUTION_KEEP_LOCAL
        } else {
            RESOLUTION_KEEP_CLOUD
        };
        set_automatic_conflict_resolution(&graph.profile_id, path, resolution, local_mtime_ns)?;
        log::info!(
            "{} [cycle:{}] CONFLICT_AUTO_RESOLVED path={} resolution={} policy=keep_newest",
            graph.account_prefix,
            graph.cycle_id,
            path,
            resolution
        );
    }
    Ok(())
}

/// Keeps the local version as a conflict copy and records that the cloud
/// version should now be fetched into the original name (the user chose
/// "Keep both").
pub(crate) fn keep_both_versions(
    graph: &GraphContext,
    row: &SyncFileRow,
    local_path: &Path,
    entry: &LocalEntry,
    stats: &mut SyncCycleStats,
) -> SyncResult<()> {
    let copy = preserve_conflict_copy(local_path)?;
    mark_conflict_preserved(&graph.profile_id, &row.path, entry)?;
    stats.conflict_copies += 1;
    log::warn!(
        "{} [cycle:{}] CONFLICT_KEPT_BOTH path={} local_copy={}",
        graph.account_prefix,
        graph.cycle_id,
        row.path,
        copy.display()
    );
    Ok(())
}

/// Items on both sides with no baseline (first sync, or a rebuilt database)
/// and items changed on both sides: identical content is adopted; anything
/// else is held as a conflict for the user to resolve. Nothing is copied or
/// overwritten.
/// Settles rows present on both sides: a file with the same content as the
/// cloud version gets its baseline; one with different content is held as
/// a conflict (`conflict_code`). Files are hashed on every core at once and
/// the progress is shown as "Checking files against OneDrive".
pub(crate) async fn reconcile_both_sides(
    graph: &GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    conflict_code: &str,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let total = rows.len();
    let started_at = std::time::Instant::now();
    log::info!(
        "{} [cycle:{}] CONTENT_CHECK_START files={} threads={}",
        graph.account_prefix,
        graph.cycle_id,
        total,
        content_check_threads()
    );
    let report_progress = |done: usize| {
        runtime_set_current_activity(
            &graph.sync_runtime,
            &graph.profile_id,
            "applying_local",
            "determinate",
            Some(done),
            Some(total),
            Some("files"),
            Some("Checking files against OneDrive"),
            Some(&graph.cycle_id),
        );
    };
    report_progress(0);

    let mut results = spawn_content_checks(
        rows.to_vec(),
        sync_root.to_path_buf(),
        Arc::clone(cancel_flag),
    );
    let mut done = 0;
    let mut last_report = std::time::Instant::now();
    while let Some((index, result)) = results.recv().await {
        ensure_not_cancelled(cancel_flag)?;
        let row = &rows[index];
        match result? {
            ContentCheck::Matches(entry) => {
                commit_baseline(&graph.profile_id, &row.path, &row.cloud_version(), &entry)?;
                if !row.is_dir {
                    stats.matched_files += 1;
                }
            }
            ContentCheck::Differs(entry) => {
                hold_conflict(
                    graph,
                    &row.path,
                    conflict_code,
                    entry.mtime_ns,
                    row.cloud_mtime,
                )?;
                stats.conflicts_held += 1;
            }
            ContentCheck::KindMismatch => {
                record_sync_file_issue(
                    &graph.profile_id,
                    &row.path,
                    "file_folder_mismatch",
                    "This name is a file on one side and a folder on the other.",
                )?;
            }
            ContentCheck::Skip => {}
        }
        done += 1;
        if last_report.elapsed() >= Duration::from_millis(250) {
            last_report = std::time::Instant::now();
            report_progress(done);
        }
    }
    ensure_not_cancelled(cancel_flag)?;
    // The check is over: the rest of the cycle shows its own activity.
    runtime_set_current_activity(
        &graph.sync_runtime,
        &graph.profile_id,
        "applying_local",
        "indeterminate",
        None,
        None,
        None,
        Some("Applying planned changes"),
        Some(&graph.cycle_id),
    );
    log::info!(
        "{} [cycle:{}] CONTENT_CHECK_DONE files={} elapsed_ms={}",
        graph.account_prefix,
        graph.cycle_id,
        done,
        started_at.elapsed().as_millis()
    );
    Ok(())
}

/// What one planned upload came to.
pub(crate) enum UploadOutcome {
    /// Gone from disk, now a folder, or claimed by another worker.
    Skipped,
    /// Same content as the baseline (a touch, a time change): only the
    /// baseline is refreshed.
    BaselineRefresh(LocalEntry),
    Failed(String),
    /// The file changed while it was uploading; the next cycle plans again.
    SourceChanged,
    Uploaded {
        item: Box<GraphDriveItem>,
        entry: LocalEntry,
        local_hash: String,
    },
}

/// Uploads planned files, many at a time under the account's transfer
/// limit (big files within its large-file share). The baseline is recorded
/// only if the file did not change while it was uploading.
pub(crate) async fn upload_planned_files(
    graph: &GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let planned: Vec<(SyncFileRow, u64)> =
        list_sync_file_rows_by_action(&graph.profile_id, PLANNER_ACTION_UPLOAD)?
            .into_iter()
            .filter(|row| !row.is_dir)
            .map(|row| {
                let size = std::fs::metadata(local_path_for(sync_root, &row.path))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                (row, size)
            })
            .collect();
    let task_graph = graph.clone();
    let task_root = sync_root.to_path_buf();
    let task_cancel = Arc::clone(cancel_flag);
    run_in_transfer_pool(
        graph,
        planned,
        |(_, size)| *size >= LARGE_DOWNLOAD_BYTES,
        cancel_flag,
        move |(row, _)| {
            let mut graph = task_graph.clone();
            let sync_root = task_root.clone();
            let cancel_flag = Arc::clone(&task_cancel);
            async move {
                let outcome = upload_planned_file(&mut graph, &sync_root, &row, &cancel_flag).await;
                (row, outcome)
            }
        },
        |(row, outcome)| record_upload_outcome(graph, &row, outcome?, stats),
    )
    .await
}

/// Hashes, claims and uploads one planned file.
async fn upload_planned_file(
    graph: &mut GraphContext,
    sync_root: &Path,
    row: &SyncFileRow,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<UploadOutcome> {
    ensure_not_cancelled(cancel_flag)?;
    let local_path = local_path_for(sync_root, &row.path);
    let Some(entry) = read_local_entry(&local_path)?.filter(|entry| !entry.is_dir) else {
        return Ok(UploadOutcome::Skipped);
    };
    let local_hash = hash_local_file(&local_path).await?;
    if row.base_present && row.base_hash.as_deref() == Some(local_hash.as_str()) {
        return Ok(UploadOutcome::BaselineRefresh(entry));
    }

    ensure_accepting_new_transfers()?;
    if !claim_upload_job_path(&graph.profile_id, &row.path, &graph.cycle_id)? {
        return Ok(UploadOutcome::Skipped);
    }
    let target = if row.base_present && row.cloud_present {
        UploadTarget::Replace {
            base_etag: row.base_etag.clone(),
        }
    } else {
        UploadTarget::New
    };
    let item = match upload_file_by_path(graph, sync_root, &row.path, &target, cancel_flag).await {
        Ok(item) => item,
        Err(error) if is_sync_cancelled_error(&error) => return Err(SyncError::from(error)),
        Err(error) => return Ok(UploadOutcome::Failed(error)),
    };
    if read_local_entry(&local_path)?.as_ref() != Some(&entry) {
        return Ok(UploadOutcome::SourceChanged);
    }
    Ok(UploadOutcome::Uploaded {
        item: Box::new(item),
        entry,
        local_hash,
    })
}

/// Records what one upload did.
fn record_upload_outcome(
    graph: &GraphContext,
    row: &SyncFileRow,
    outcome: UploadOutcome,
    stats: &mut SyncCycleStats,
) -> SyncResult<()> {
    match outcome {
        UploadOutcome::Skipped => {}
        UploadOutcome::BaselineRefresh(entry) => {
            refresh_baseline_local(&graph.profile_id, &row.path, &entry)?;
            stats.baseline_refreshes += 1;
        }
        UploadOutcome::Failed(error) => {
            stats.upload_failures += 1;
            log::warn!(
                "{} [cycle:{}] UPLOAD_FAILED path={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                row.path,
                error
            );
        }
        UploadOutcome::SourceChanged => log::info!(
            "{} [cycle:{}] UPLOAD_SOURCE_CHANGED path={} action=replan",
            graph.account_prefix,
            graph.cycle_id,
            row.path
        ),
        UploadOutcome::Uploaded {
            item,
            entry,
            local_hash,
        } => {
            if item
                .quick_xor_hash()
                .as_deref()
                .is_some_and(|cloud_hash| cloud_hash != local_hash)
            {
                // SharePoint libraries rewrite some file types after upload;
                // the baseline records what OneDrive now holds.
                log::warn!(
                    "{} [cycle:{}] UPLOAD_HASH_DIFFERS path={} local={} cloud={:?}",
                    graph.account_prefix,
                    graph.cycle_id,
                    row.path,
                    local_hash,
                    item.quick_xor_hash()
                );
            }
            commit_baseline_from_upload(
                &graph.profile_id,
                &item.to_cloud_item(row.path.clone()),
                &entry,
            )?;
            stats.uploaded_files += 1;
            graph.record_transfer_success();
        }
    }
    Ok(())
}

/// Moves local files and empty folders the plan deletes to the Trash, each
/// only if it still matches the baseline. These deletes were made in the
/// cloud, so they do not count towards the delete guard.
pub(crate) fn delete_local_items(
    graph: &GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    stats: &mut SyncCycleStats,
) -> SyncResult<()> {
    let mut progress = DeleteProgress::start(graph, "Moving to the Trash", count_files(rows));
    for row in rows {
        progress.advance(usize::from(!row.is_dir));
        let local_path = local_path_for(sync_root, &row.path);
        let Some(entry) = read_local_entry(&local_path)? else {
            forget_sync_file(&graph.profile_id, &row.path)?;
            continue;
        };
        let still_safe = if entry.is_dir {
            std::fs::read_dir(&local_path)
                .map(|mut children| children.next().is_none())
                .unwrap_or(false)
        } else {
            row.local_matches_baseline(&entry)
        };
        if !still_safe {
            log::info!(
                "{} [cycle:{}] LOCAL_DELETE_SKIPPED path={} reason=changed_since_plan",
                graph.account_prefix,
                graph.cycle_id,
                row.path
            );
            continue;
        }
        trash::delete(&local_path).map_err(|error| {
            format!(
                "Failed moving '{}' to the Trash: {error}",
                local_path.display()
            )
        })?;
        record_local_absent(&graph.profile_id, &row.path)?;
        forget_sync_file(&graph.profile_id, &row.path)?;
        if !entry.is_dir {
            record_finished_delete(
                &graph.profile_id,
                HISTORY_TRASHED_LOCALLY,
                &row.path,
                entry.size,
            )?;
        }
        stats.deleted_local += 1;
        log::info!(
            "{} [cycle:{}] LOCAL_DELETE_TO_TRASH path={}",
            graph.account_prefix,
            graph.cycle_id,
            row.path
        );
    }
    progress.finish();
    Ok(())
}

/// Carries out the user's choices for held conflicts. Each choice applies
/// only if the local file is as it was when they chose; otherwise the choice
/// is withdrawn and the file stays held for a fresh decision.
pub(crate) async fn apply_conflict_resolutions(
    graph: &mut GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    for choice in list_conflict_resolutions(&graph.profile_id)? {
        ensure_not_cancelled(cancel_flag)?;
        let row = &choice.row;
        let local_path = local_path_for(sync_root, &row.path);
        let entry = read_local_entry(&local_path)?.filter(|entry| !entry.is_dir);
        if entry.as_ref().map(|entry| entry.mtime_ns) != choice.chosen_local_mtime_ns {
            withdraw_conflict_resolution(&graph.profile_id, &row.path)?;
            log::info!(
                "{} [cycle:{}] CONFLICT_CHOICE_WITHDRAWN path={} reason=changed_since_choice",
                graph.account_prefix,
                graph.cycle_id,
                row.path
            );
            continue;
        }
        match (choice.resolution.as_str(), entry) {
            (RESOLUTION_KEEP_LOCAL, Some(entry)) => {
                ensure_accepting_new_transfers()?;
                let local_size = entry.size;
                upsert_upload_job_queued(&graph.profile_id, &row.path, local_size, entry.mtime_ns)?;
                if !claim_upload_job_path(&graph.profile_id, &row.path, &graph.cycle_id)? {
                    continue;
                }
                let target = if row.cloud_present {
                    UploadTarget::Replace { base_etag: None }
                } else {
                    UploadTarget::New
                };
                let uploaded =
                    upload_file_by_path(graph, sync_root, &row.path, &target, cancel_flag).await?;
                if read_local_entry(&local_path)?.as_ref() == Some(&entry) {
                    commit_baseline_from_upload(
                        &graph.profile_id,
                        &uploaded.to_cloud_item(row.path.clone()),
                        &entry,
                    )?;
                    clear_sync_file_issue(&graph.profile_id, &row.path)?;
                    stats.uploaded_files += 1;
                }
            }
            (RESOLUTION_KEEP_CLOUD, local_entry) => {
                if local_entry.is_some() {
                    trash::delete(&local_path).map_err(|error| {
                        format!(
                            "Failed moving '{}' to the Trash: {error}",
                            local_path.display()
                        )
                    })?;
                    record_local_absent(&graph.profile_id, &row.path)?;
                }
                // With no baseline the next plan downloads OneDrive's version
                // (or, if OneDrive deleted it, there is nothing left to do).
                forget_sync_file(&graph.profile_id, &row.path)?;
                clear_sync_file_issue(&graph.profile_id, &row.path)?;
            }
            (RESOLUTION_KEEP_BOTH, Some(entry)) => {
                if row.cloud_present {
                    keep_both_versions(graph, row, &local_path, &entry, stats)?;
                } else {
                    forget_sync_file(&graph.profile_id, &row.path)?;
                }
                clear_sync_file_issue(&graph.profile_id, &row.path)?;
            }
            (_, None) => {
                // The local file is gone and the choice needed it: OneDrive's
                // version (if any) comes back through the normal plan.
                forget_sync_file(&graph.profile_id, &row.path)?;
                clear_sync_file_issue(&graph.profile_id, &row.path)?;
            }
            (resolution, Some(_)) => {
                return Err(SyncError::from(format!(
                    "Unknown conflict resolution '{resolution}' for '{}'",
                    row.path
                )));
            }
        }
        log::info!(
            "{} [cycle:{}] CONFLICT_RESOLVED path={} resolution={}",
            graph.account_prefix,
            graph.cycle_id,
            row.path,
            choice.resolution
        );
    }
    Ok(())
}
