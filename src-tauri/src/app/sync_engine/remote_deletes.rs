use super::*;

// Deletes in OneDrive (its recycle bin keeps them). A folder whose whole
// content is being deleted goes with one request instead of one check and
// one delete per file, which would run into Microsoft's request limits for
// a big folder; anything that does not line up falls back to per-item
// deletes.

/// A cloud folder whose every item is planned for deletion.
#[derive(Debug, Clone)]
pub(crate) struct FolderDelete {
    pub(crate) folder: SyncFileRow,
    /// The planned file deletes inside it, used if the folder falls back to
    /// per-item deletes.
    pub(crate) rows: Vec<SyncFileRow>,
    pub(crate) file_count: usize,
    /// The total size of the files inside, as the cloud last reported it.
    pub(crate) file_bytes: u64,
}

/// Applies the planned (and allowed) cloud deletes.
pub(crate) async fn apply_remote_deletes(
    graph: &mut GraphContext,
    rows: &[SyncFileRow],
    user_approved: &HashSet<String>,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let (folders, mut per_item) = plan_folder_deletes(&graph.profile_id, rows)?;
    let mut progress = DeleteProgress::start(graph, "Deleting from OneDrive", count_files(rows));
    for folder_delete in folders {
        let deleted =
            delete_whole_folder(graph, &folder_delete, user_approved, stats, cancel_flag).await?;
        if deleted {
            progress.advance(folder_delete.file_count);
        } else {
            per_item.extend(folder_delete.rows);
        }
    }
    // Deepest first, so a folder is only reached after its contents.
    per_item.sort_by_key(|row| std::cmp::Reverse(row.path.matches('/').count()));
    delete_remote_items(
        graph,
        &per_item,
        user_approved,
        stats,
        &mut progress,
        cancel_flag,
    )
    .await?;
    progress.finish();
    Ok(())
}

/// Splits the planned cloud deletes into whole folders (the top-most
/// folders deleted on this device whose every cloud item is a planned file
/// delete or a folder also deleted here) and the remaining per-item deletes.
pub(crate) fn plan_folder_deletes(
    profile_id: &str,
    rows: &[SyncFileRow],
) -> Result<(Vec<FolderDelete>, Vec<SyncFileRow>), String> {
    let planned: HashSet<&str> = rows
        .iter()
        .filter(|row| !row.is_dir)
        .map(|row| row.path.as_str())
        .collect();
    if planned.is_empty() {
        return Ok((Vec::new(), rows.to_vec()));
    }
    let connection = open_sync_jobs_connection(profile_id)?;
    let candidates = {
        let mut statement = connection
            .prepare(&format!(
                "SELECT {SYNC_FILE_ROW_COLUMNS} FROM sync_files
                 WHERE profile_id = ?1 AND is_dir = 1 AND is_shared_reference = 0
                   AND local_present = 0 AND base_present = 1 AND cloud_present = 1
                   AND remote_item_id IS NOT NULL AND cloud_etag IS NOT NULL
                 ORDER BY path"
            ))
            .map_err(|error| format!("Failed preparing folder delete query: {error}"))?;
        let rows = statement
            .query_map(params![profile_id], sync_file_row_from_sql)
            .map_err(|error| format!("Failed querying folder deletes: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed reading folder deletes: {error}"))?
    };

    let mut folders: Vec<FolderDelete> = Vec::new();
    for folder in candidates {
        if folders
            .iter()
            .any(|accepted| is_inside(&folder.path, &accepted.folder.path))
        {
            continue;
        }
        let mut statement = connection
            .prepare_cached(
                "SELECT path, is_dir, local_present, base_present, is_shared_reference, cloud_size
                 FROM sync_files
                 WHERE profile_id = ?1 AND cloud_present = 1
                   AND path > ?2 || '/' AND path < ?2 || '0'",
            )
            .map_err(|error| format!("Failed preparing folder content query: {error}"))?;
        let mut inside = statement
            .query(params![profile_id, folder.path])
            .map_err(|error| format!("Failed querying folder content: {error}"))?;
        let mut whole = true;
        let mut file_count = 0;
        let mut file_bytes = 0u64;
        while let Some(item) = inside
            .next()
            .map_err(|error| format!("Failed reading folder content: {error}"))?
        {
            let path: String = item.get(0).map_err(|error| error.to_string())?;
            let is_dir = sql_to_bool(item.get(1).map_err(|error| error.to_string())?);
            let local_present = sql_to_bool(item.get(2).map_err(|error| error.to_string())?);
            let base_present = sql_to_bool(item.get(3).map_err(|error| error.to_string())?);
            let shared = sql_to_bool(item.get(4).map_err(|error| error.to_string())?);
            let size: i64 = item.get(5).map_err(|error| error.to_string())?;
            let goes = if shared {
                false
            } else if is_dir {
                !local_present && base_present
            } else {
                planned.contains(path.as_str())
            };
            if !goes {
                whole = false;
                break;
            }
            if !is_dir {
                file_count += 1;
                file_bytes += size.max(0) as u64;
            }
        }
        if whole && file_count > 0 {
            folders.push(FolderDelete {
                folder,
                rows: Vec::new(),
                file_count,
                file_bytes,
            });
        }
    }

    let mut remaining = Vec::new();
    'rows: for row in rows {
        for folder_delete in folders.iter_mut() {
            if row.path == folder_delete.folder.path {
                continue 'rows;
            }
            if is_inside(&row.path, &folder_delete.folder.path) {
                folder_delete.rows.push(row.clone());
                continue 'rows;
            }
        }
        remaining.push(row.clone());
    }
    Ok((folders, remaining))
}

/// Whether `path` is strictly inside the folder `folder`.
pub(crate) fn is_inside(path: &str, folder: &str) -> bool {
    path.len() > folder.len() + 1
        && path.starts_with(folder)
        && path.as_bytes()[folder.len()] == b'/'
}

/// Deletes the folder with one request after reading it again: it must hold
/// exactly the bytes the plan expects and still carry the eTag the cloud
/// changes last reported, and the delete is conditional on that eTag.
/// Returns false (nothing deleted) when it does not line up.
async fn delete_whole_folder(
    graph: &mut GraphContext,
    folder_delete: &FolderDelete,
    user_approved: &HashSet<String>,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<bool> {
    let folder = &folder_delete.folder;
    let (Some(item_id), Some(expected_etag)) =
        (folder.remote_item_id.clone(), folder.cloud_etag.clone())
    else {
        return Ok(false);
    };
    let current = fetch_remote_item(graph, &item_id, cancel_flag).await?;
    let lines_up = current.as_ref().is_some_and(|item| {
        item.e_tag.as_deref() == Some(expected_etag.as_str())
            && item.size == Some(folder_delete.file_bytes)
    });
    if current.is_some() && !lines_up {
        log::info!(
            "{} [cycle:{}] REMOTE_FOLDER_DELETE_FALLBACK path={} reason=changed_since_plan expected_bytes={} cloud_bytes={:?}",
            graph.account_prefix,
            graph.cycle_id,
            folder.path,
            folder_delete.file_bytes,
            current.as_ref().and_then(|item| item.size)
        );
        return Ok(false);
    }
    if current.is_some() {
        log::info!(
            "{} [cycle:{}] REMOTE_FOLDER_DELETE_REQUEST path={} files={} bytes={}",
            graph.account_prefix,
            graph.cycle_id,
            folder.path,
            folder_delete.file_count,
            folder_delete.file_bytes
        );
        let outcome =
            delete_remote_folder_if_unchanged(graph, &item_id, &expected_etag, cancel_flag).await?;
        if outcome == GraphDeleteOutcome::ChangedSinceRead {
            return Ok(false);
        }
    }
    forget_folder_tree(&graph.profile_id, &folder.path)?;
    if !user_approved.contains(&folder.path) {
        for _ in 0..folder_delete.file_count {
            record_throttle_event(&graph.profile_id, DELETE_EVENT_DIRECTION)?;
        }
    }
    record_finished_delete(
        &graph.profile_id,
        HISTORY_DELETED_REMOTELY,
        &folder.path,
        folder_delete.file_bytes,
    )?;
    stats.deleted_remote += folder_delete.file_count;
    Ok(true)
}

/// Deletes cloud items the plan deletes (OneDrive keeps them in its recycle
/// bin). Each item is read again first: a file whose content changed since
/// the baseline, or a folder that is not empty, is left alone. Deletes the
/// user approved in SomeDrive do not count towards the delete guard.
pub(crate) async fn delete_remote_items(
    graph: &mut GraphContext,
    rows: &[SyncFileRow],
    user_approved: &HashSet<String>,
    stats: &mut SyncCycleStats,
    progress: &mut DeleteProgress,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    for row in rows {
        ensure_not_cancelled(cancel_flag)?;
        progress.advance(usize::from(!row.is_dir));
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
        if !user_approved.contains(&row.path) {
            record_throttle_event(&graph.profile_id, DELETE_EVENT_DIRECTION)?;
        }
        if !row.is_dir {
            record_finished_delete(
                &graph.profile_id,
                HISTORY_DELETED_REMOTELY,
                &row.path,
                row.base_cloud_size,
            )?;
        }
        stats.deleted_remote += 1;
    }
    Ok(())
}
