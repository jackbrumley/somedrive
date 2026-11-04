// Applies a cycle's plan (docs/sync-engine.md sections 4 and 6). Every action
// re-checks the file it is about to touch, so a change made since the plan
// is never overwritten or deleted: the action is skipped and the next cycle
// plans again from the new state.

const DELETE_EVENT_DIRECTION: &str = "delete";

async fn hash_local_file(path: &Path) -> Result<String, String> {
    let owned = path.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || quick_xor_hash_file(&owned))
        .await
        .map_err(|error| format!("Hashing task failed: {error}"))?
}

fn local_path_for(sync_root: &Path, relative_path: &str) -> PathBuf {
    sync_root.join(path_to_local(relative_path))
}

/// Whether the local file's content equals the cloud version in `row`.
/// Empty files match without a hash; a non-empty file is never assumed equal
/// without one.
async fn local_matches_cloud(row: &SyncFileRow, local_path: &Path, entry: &LocalEntry) -> Result<bool, String> {
    if entry.size == 0 && row.cloud_size == 0 {
        return Ok(true);
    }
    if entry.size != row.cloud_size {
        return Ok(false);
    }
    match row.cloud_hash.as_deref() {
        Some(cloud_hash) => Ok(hash_local_file(local_path).await? == cloud_hash),
        None => Ok(false),
    }
}

fn forget_rows(profile_id: &str, rows: &[SyncFileRow]) -> Result<(), String> {
    for row in rows {
        forget_sync_file(profile_id, &row.path)?;
    }
    Ok(())
}

fn create_local_folders(graph: &GraphContext, sync_root: &Path, rows: &[SyncFileRow], stats: &mut SyncCycleStats) -> Result<(), String> {
    for row in rows {
        let local_path = local_path_for(sync_root, &row.path);
        std::fs::create_dir_all(&local_path)
            .map_err(|error| format!("Failed creating folder '{}': {error}", local_path.display()))?;
        if let Some(entry) = read_local_entry(&local_path)? {
            commit_baseline(&graph.profile_id, &row.path, &row.cloud_version(), &entry)?;
            stats.created_local_folders += 1;
        }
    }
    Ok(())
}

async fn create_remote_folders(
    graph: &mut GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    for row in rows {
        ensure_accepting_new_transfers()?;
        let Some(entry) = read_local_entry(&local_path_for(sync_root, &row.path))? else {
            continue;
        };
        let created = create_remote_folder(graph, &row.path, cancel_flag).await?;
        commit_baseline_from_upload(&graph.profile_id, &created.to_cloud_item(row.path.clone()), &entry)?;
        stats.created_remote_folders += 1;
    }
    Ok(())
}

fn conflict_detail(conflict_code: &str) -> &'static str {
    if conflict_code == ISSUE_CONFLICT_BOTH_NEW {
        "This file is on this device and in OneDrive with different content."
    } else {
        "This file changed on this device and in OneDrive since the last sync."
    }
}

/// Holds `path` as a conflict: it stops syncing until it is resolved. With
/// the "keep the newest" setting the newer side is chosen straight away (and
/// applied next cycle, through the same checks as a user's choice).
fn hold_conflict(
    graph: &GraphContext,
    path: &str,
    conflict_code: &str,
    local_mtime_ns: i64,
    cloud_mtime: i64,
) -> Result<(), String> {
    record_sync_file_issue(&graph.profile_id, path, conflict_code, conflict_detail(conflict_code))?;
    log::warn!(
        "{} [cycle:{}] CONFLICT_HELD path={} code={}",
        graph.account_prefix,
        graph.cycle_id,
        path,
        conflict_code
    );
    if crate::app::app_settings::load_conflict_policy()? == crate::app::app_settings::ConflictPolicy::KeepNewest {
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
fn keep_both_versions(graph: &GraphContext, row: &SyncFileRow, local_path: &Path, entry: &LocalEntry, stats: &mut SyncCycleStats) -> Result<(), String> {
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
async fn reconcile_both_sides(
    graph: &GraphContext,
    sync_root: &Path,
    rows: &[SyncFileRow],
    conflict_code: &str,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    for row in rows {
        ensure_not_cancelled(cancel_flag)?;
        let local_path = local_path_for(sync_root, &row.path);
        let Some(entry) = read_local_entry(&local_path)? else {
            continue;
        };
        if row.is_dir || entry.is_dir {
            if row.is_dir && entry.is_dir {
                commit_baseline(&graph.profile_id, &row.path, &row.cloud_version(), &entry)?;
            } else {
                record_sync_file_issue(
                    &graph.profile_id,
                    &row.path,
                    "file_folder_mismatch",
                    "This name is a file on one side and a folder on the other.",
                )?;
            }
            continue;
        }
        if local_matches_cloud(row, &local_path, &entry).await? {
            commit_baseline(&graph.profile_id, &row.path, &row.cloud_version(), &entry)?;
            stats.matched_files += 1;
        } else {
            hold_conflict(graph, &row.path, conflict_code, entry.mtime_ns, row.cloud_mtime)?;
            stats.conflicts_held += 1;
        }
    }
    Ok(())
}

/// Uploads planned files. A file whose content still equals the baseline
/// (a touch, a time change) only refreshes the baseline. The baseline is
/// recorded only if the file did not change while it was uploading.
async fn upload_planned_files(
    graph: &mut GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    for row in list_sync_file_rows_by_action(&graph.profile_id, PLANNER_ACTION_UPLOAD)? {
        ensure_not_cancelled(cancel_flag)?;
        if row.is_dir {
            continue;
        }
        let local_path = local_path_for(sync_root, &row.path);
        let Some(entry) = read_local_entry(&local_path)?.filter(|entry| !entry.is_dir) else {
            continue;
        };
        let local_hash = hash_local_file(&local_path).await?;
        if row.base_present && row.base_hash.as_deref() == Some(local_hash.as_str()) {
            refresh_baseline_local(&graph.profile_id, &row.path, &entry)?;
            stats.baseline_refreshes += 1;
            continue;
        }

        ensure_accepting_new_transfers()?;
        if !claim_upload_job_path(&graph.profile_id, &row.path, &graph.cycle_id)? {
            continue;
        }
        let target = if row.base_present && row.cloud_present {
            UploadTarget::Replace { base_etag: row.base_etag.clone() }
        } else {
            UploadTarget::New
        };
        let uploaded = match upload_file_by_path(graph, sync_root, &row.path, &target, cancel_flag).await {
            Ok(uploaded) => uploaded,
            Err(error) => {
                if is_sync_cancelled_error(&error) {
                    return Err(error);
                }
                stats.upload_failures += 1;
                log::warn!(
                    "{} [cycle:{}] UPLOAD_FAILED path={} error={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    row.path,
                    error
                );
                continue;
            }
        };
        if read_local_entry(&local_path)?.as_ref() != Some(&entry) {
            log::info!(
                "{} [cycle:{}] UPLOAD_SOURCE_CHANGED path={} action=replan",
                graph.account_prefix,
                graph.cycle_id,
                row.path
            );
            continue;
        }
        if uploaded.quick_xor_hash().as_deref().is_some_and(|cloud_hash| cloud_hash != local_hash) {
            // SharePoint libraries rewrite some file types after upload; the
            // baseline records what OneDrive now holds.
            log::warn!(
                "{} [cycle:{}] UPLOAD_HASH_DIFFERS path={} local={} cloud={:?}",
                graph.account_prefix,
                graph.cycle_id,
                row.path,
                local_hash,
                uploaded.quick_xor_hash()
            );
        }
        commit_baseline_from_upload(&graph.profile_id, &uploaded.to_cloud_item(row.path.clone()), &entry)?;
        stats.uploaded_files += 1;
    }
    Ok(())
}

/// Moves local files and empty folders the plan deletes to the Trash, each
/// only if it still matches the baseline.
fn delete_local_items(graph: &GraphContext, sync_root: &Path, rows: &[SyncFileRow], stats: &mut SyncCycleStats) -> Result<(), String> {
    for row in rows {
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
        trash::delete(&local_path)
            .map_err(|error| format!("Failed moving '{}' to the Trash: {error}", local_path.display()))?;
        record_local_absent(&graph.profile_id, &row.path)?;
        forget_sync_file(&graph.profile_id, &row.path)?;
        record_throttle_event(&graph.profile_id, DELETE_EVENT_DIRECTION)?;
        stats.deleted_local += 1;
        log::info!(
            "{} [cycle:{}] LOCAL_DELETE_TO_TRASH path={}",
            graph.account_prefix,
            graph.cycle_id,
            row.path
        );
    }
    Ok(())
}

/// Deletes cloud items the plan deletes (OneDrive keeps them in its recycle
/// bin). Each item is read again first: a file whose content changed since
/// the baseline, or a folder that is not empty, is left alone.
async fn delete_remote_items(
    graph: &mut GraphContext,
    rows: &[SyncFileRow],
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    for row in rows {
        ensure_not_cancelled(cancel_flag)?;
        let Some(item_id) = row.remote_item_id.clone() else {
            continue;
        };
        let Some(current) = fetch_remote_item(graph, &item_id, cancel_flag).await? else {
            record_cloud_absent(&graph.profile_id, &row.path)?;
            forget_sync_file(&graph.profile_id, &row.path)?;
            continue;
        };
        let unchanged = if row.is_dir {
            !remote_folder_has_children(graph, &item_id, cancel_flag).await?
        } else {
            current.size.unwrap_or(0) == row.base_cloud_size
                && match (current.quick_xor_hash(), row.base_hash.as_deref()) {
                    (Some(current_hash), Some(base_hash)) => current_hash == base_hash,
                    _ => current.c_tag.is_some() && current.c_tag == row.cloud_ctag,
                }
        };
        if !unchanged {
            log::info!(
                "{} [cycle:{}] REMOTE_DELETE_SKIPPED path={} reason=changed_since_plan",
                graph.account_prefix,
                graph.cycle_id,
                row.path
            );
            continue;
        }
        delete_remote_item(graph, &item_id, cancel_flag).await?;
        record_cloud_absent(&graph.profile_id, &row.path)?;
        forget_sync_file(&graph.profile_id, &row.path)?;
        record_throttle_event(&graph.profile_id, DELETE_EVENT_DIRECTION)?;
        stats.deleted_remote += 1;
    }
    Ok(())
}

/// Carries out the user's choices for held conflicts. Each choice applies
/// only if the local file is as it was when they chose; otherwise the choice
/// is withdrawn and the file stays held for a fresh decision.
async fn apply_conflict_resolutions(
    graph: &mut GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
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
                let uploaded = upload_file_by_path(graph, sync_root, &row.path, &target, cancel_flag).await?;
                if read_local_entry(&local_path)?.as_ref() == Some(&entry) {
                    commit_baseline_from_upload(&graph.profile_id, &uploaded.to_cloud_item(row.path.clone()), &entry)?;
                    clear_sync_file_issue(&graph.profile_id, &row.path)?;
                    stats.uploaded_files += 1;
                }
            }
            (RESOLUTION_KEEP_CLOUD, local_entry) => {
                if local_entry.is_some() {
                    trash::delete(&local_path).map_err(|error| {
                        format!("Failed moving '{}' to the Trash: {error}", local_path.display())
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
                return Err(format!("Unknown conflict resolution '{resolution}' for '{}'", row.path));
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
