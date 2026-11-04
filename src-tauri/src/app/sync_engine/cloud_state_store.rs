use super::*;

// Records the cloud side of `sync_files` from delta pages: path resolution
// from parent item IDs, renames and moves, deletes, and cloud items that are
// never synced. Part of the `sync_files` store.

pub(crate) const ISSUE_ONENOTE_NOTEBOOK: &str = "onenote_notebook";
pub(crate) const ISSUE_MALWARE_DETECTED: &str = "malware_detected";

/// What applying one delta page did.
#[derive(Debug, Default)]
pub(crate) struct DeltaPageSummary {
    pub(crate) recorded: usize,
    pub(crate) deleted: usize,
    pub(crate) skipped_unresolved: usize,
}

/// Applies one delta page and records its continuation link in the same
/// transaction, so an interrupted cycle resumes exactly after the last page
/// whose items are stored (nothing is lost on pause or quit).
pub(crate) fn apply_delta_page(
    profile_id: &str,
    account_prefix: &str,
    items: &[GraphDriveItem],
    next_link: Option<&str>,
) -> Result<DeltaPageSummary, String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting delta page transaction: {error}"))?;
    let now = current_unix_seconds();
    let mut summary = DeltaPageSummary::default();
    for item in items {
        if item.deleted.is_some() {
            mark_cloud_item_deleted(&transaction, profile_id, &item.id, now)?;
            summary.deleted += 1;
            continue;
        }
        let Some(path) = resolve_delta_item_path(&transaction, profile_id, item)? else {
            log::warn!(
                "{} DELTA_ITEM_SKIPPED id={} name={:?} reason=parent_unresolved",
                account_prefix,
                item.id,
                item.name
            );
            summary.skipped_unresolved += 1;
            continue;
        };
        let exclusion = cloud_item_exclusion(&transaction, profile_id, item)?;
        upsert_cloud_item(
            &transaction,
            profile_id,
            &item.to_cloud_item(path.clone()),
            now,
        )?;
        if let Some((issue_code, issue_detail)) = exclusion {
            write_sync_file_issue(&transaction, profile_id, &path, issue_code, issue_detail)?;
        }
        summary.recorded += 1;
    }
    if let Some(link) = next_link {
        transaction
            .execute(
                "INSERT INTO sync_lifecycle_state (profile_id, active_delta_next_link, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(profile_id) DO UPDATE SET
                    active_delta_next_link = excluded.active_delta_next_link,
                    updated_at = excluded.updated_at",
                params![profile_id, link, now],
            )
            .map_err(|error| format!("Failed recording delta page link: {error}"))?;
    }
    delete_untracked_rows(&transaction, profile_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing delta page: {error}"))?;
    Ok(summary)
}

/// Cloud items SomeDrive never syncs: OneNote notebooks and everything in
/// them (Graph cannot round-trip them as files) and items OneDrive flagged
/// as malware (downloads are refused).
pub(crate) fn cloud_item_exclusion(
    connection: &Connection,
    profile_id: &str,
    item: &GraphDriveItem,
) -> Result<Option<(&'static str, &'static str)>, String> {
    if item.package.is_some() {
        return Ok(Some((
            ISSUE_ONENOTE_NOTEBOOK,
            "OneNote notebooks are not synced; open them in OneNote.",
        )));
    }
    if item.malware.is_some() {
        return Ok(Some((
            ISSUE_MALWARE_DETECTED,
            "OneDrive flagged this file as malware, so it is not downloaded.",
        )));
    }
    let Some(parent_id) = item.parent_id() else {
        return Ok(None);
    };
    let parent_issue: Option<String> = connection
        .query_row(
            "SELECT issue_code FROM sync_files
             WHERE profile_id = ?1 AND remote_item_id = ?2 AND cloud_present = 1",
            params![profile_id, parent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed reading parent exclusion: {error}"))?
        .flatten();
    Ok(
        (parent_issue.as_deref() == Some(ISSUE_ONENOTE_NOTEBOOK)).then_some((
            ISSUE_ONENOTE_NOTEBOOK,
            "OneNote notebooks are not synced; open them in OneNote.",
        )),
    )
}

/// The item's path, from its parent's stored path (the drive root is stored
/// as `""`). Falls back to `parentReference.path` when Graph supplies it.
/// `None` when neither is known: the item is skipped, never guessed.
pub(crate) fn resolve_delta_item_path(
    connection: &Connection,
    profile_id: &str,
    item: &GraphDriveItem,
) -> Result<Option<String>, String> {
    if item.root.is_some() {
        return Ok(Some(String::new()));
    }
    let Some(name) = item
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Ok(None);
    };
    let parent_path = match item.parent_id() {
        Some(parent_id) => resolve_cloud_parent_path(connection, profile_id, &parent_id)?,
        None => None,
    };
    let parent_path = parent_path.or_else(|| {
        item.parent_reference
            .as_ref()
            .and_then(|reference| reference.path.as_deref())
            .map(extract_root_relative)
    });
    Ok(parent_path.map(|parent| {
        normalize_relative_path(&if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        })
    }))
}

/// The stored path of the cloud folder `parent_id` (`""` for the drive root).
pub(crate) fn resolve_cloud_parent_path(
    connection: &Connection,
    profile_id: &str,
    parent_id: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT path FROM sync_files
             WHERE profile_id = ?1 AND remote_item_id = ?2 AND cloud_present = 1 AND is_dir = 1",
            params![profile_id, parent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed resolving parent folder {parent_id}: {error}"))
}

pub(crate) fn upsert_cloud_item(
    connection: &Connection,
    profile_id: &str,
    item: &CloudItem,
    now: i64,
) -> Result<(), String> {
    let previous_path: Option<String> = connection
        .query_row(
            "SELECT path FROM sync_files
             WHERE profile_id = ?1 AND remote_item_id = ?2 AND cloud_present = 1",
            params![profile_id, item.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed reading previous cloud path: {error}"))?;
    if let Some(old_path) = previous_path.filter(|old_path| *old_path != item.path) {
        if !follow_cloud_move(connection, profile_id, &old_path, &item.path, now)? {
            move_cloud_state(connection, profile_id, &old_path, &item.path, now)?;
        }
    }

    connection
        .execute(
            "INSERT INTO sync_files (
                profile_id, path, is_dir, is_shared_reference, shared_drive_id, shared_item_id,
                shared_kind, remote_item_id, remote_parent_id, cloud_present, cloud_size,
                cloud_mtime, cloud_etag, cloud_ctag, cloud_hash, cloud_seen_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
             ON CONFLICT(profile_id, path) DO UPDATE SET
                is_dir = excluded.is_dir,
                is_shared_reference = excluded.is_shared_reference,
                shared_drive_id = excluded.shared_drive_id,
                shared_item_id = excluded.shared_item_id,
                shared_kind = excluded.shared_kind,
                remote_item_id = excluded.remote_item_id,
                remote_parent_id = excluded.remote_parent_id,
                cloud_present = 1,
                cloud_size = excluded.cloud_size,
                cloud_mtime = excluded.cloud_mtime,
                cloud_etag = excluded.cloud_etag,
                cloud_ctag = excluded.cloud_ctag,
                cloud_hash = excluded.cloud_hash,
                cloud_seen_at = excluded.cloud_seen_at,
                updated_at = excluded.updated_at",
            params![
                profile_id,
                item.path,
                bool_to_sql(item.is_dir),
                bool_to_sql(item.is_shared_reference),
                item.shared_drive_id,
                item.shared_item_id,
                item.shared_kind,
                item.id,
                item.parent_id,
                item.size as i64,
                item.mtime,
                item.etag,
                item.ctag,
                item.quick_xor_hash,
                now,
            ],
        )
        .map_err(|error| format!("Failed recording cloud item '{}': {error}", item.path))?;
    Ok(())
}

/// A cloud item (and, for a folder, everything under it) moved from
/// `old_path` to `new_path`: its cloud state follows it, while the local
/// state and baseline stay at the old path. The planner then removes the old
/// copy (to Trash, through the delete guard) and downloads the new one.
pub(crate) fn move_cloud_state(
    connection: &Connection,
    profile_id: &str,
    old_path: &str,
    new_path: &str,
    now: i64,
) -> Result<(), String> {
    struct MovedRow {
        path: String,
        is_dir: bool,
        remote_item_id: Option<String>,
        remote_parent_id: Option<String>,
        cloud_size: i64,
        cloud_mtime: i64,
        cloud_etag: Option<String>,
        cloud_ctag: Option<String>,
        cloud_hash: Option<String>,
    }
    let prefix = format!("{old_path}/");
    let moved: Vec<MovedRow> = {
        let mut statement = connection
            .prepare(
                "SELECT path, is_dir, remote_item_id, remote_parent_id, cloud_size, cloud_mtime,
                        cloud_etag, cloud_ctag, cloud_hash
                 FROM sync_files
                 WHERE profile_id = ?1 AND cloud_present = 1
                   AND (path = ?2 OR substr(path, 1, length(?3)) = ?3)",
            )
            .map_err(|error| format!("Failed preparing cloud move query: {error}"))?;
        let rows = statement
            .query_map(params![profile_id, old_path, prefix], |row| {
                Ok(MovedRow {
                    path: row.get(0)?,
                    is_dir: sql_to_bool(row.get(1)?),
                    remote_item_id: row.get(2)?,
                    remote_parent_id: row.get(3)?,
                    cloud_size: row.get(4)?,
                    cloud_mtime: row.get(5)?,
                    cloud_etag: row.get(6)?,
                    cloud_ctag: row.get(7)?,
                    cloud_hash: row.get(8)?,
                })
            })
            .map_err(|error| format!("Failed querying cloud move rows: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed reading cloud move rows: {error}"))?
    };

    for row in &moved {
        clear_cloud_state(connection, profile_id, &row.path, now)?;
    }
    for row in moved {
        let moved_path = format!("{new_path}{}", &row.path[old_path.len()..]);
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, remote_item_id, remote_parent_id, cloud_present,
                    cloud_size, cloud_mtime, cloud_etag, cloud_ctag, cloud_hash, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(profile_id, path) DO UPDATE SET
                    is_dir = excluded.is_dir,
                    remote_item_id = excluded.remote_item_id,
                    remote_parent_id = excluded.remote_parent_id,
                    cloud_present = 1,
                    cloud_size = excluded.cloud_size,
                    cloud_mtime = excluded.cloud_mtime,
                    cloud_etag = excluded.cloud_etag,
                    cloud_ctag = excluded.cloud_ctag,
                    cloud_hash = excluded.cloud_hash,
                    updated_at = excluded.updated_at",
                params![
                    profile_id,
                    moved_path,
                    bool_to_sql(row.is_dir),
                    row.remote_item_id,
                    row.remote_parent_id,
                    row.cloud_size,
                    row.cloud_mtime,
                    row.cloud_etag,
                    row.cloud_ctag,
                    row.cloud_hash,
                    now,
                ],
            )
            .map_err(|error| format!("Failed moving cloud state to '{moved_path}': {error}"))?;
    }
    Ok(())
}

/// A deleted cloud item: it and, for a folder, everything under it are
/// absent in the cloud (Graph reports only the folder itself).
pub(crate) fn mark_cloud_item_deleted(
    connection: &Connection,
    profile_id: &str,
    remote_item_id: &str,
    now: i64,
) -> Result<(), String> {
    let path: Option<String> = connection
        .query_row(
            "SELECT path FROM sync_files
             WHERE profile_id = ?1 AND remote_item_id = ?2 AND cloud_present = 1",
            params![profile_id, remote_item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed reading deleted cloud item: {error}"))?;
    let Some(path) = path else {
        return Ok(());
    };
    let prefix = format!("{path}/");
    connection
        .execute(
            "UPDATE sync_files
             SET cloud_present = 0, cloud_size = 0, cloud_mtime = 0, cloud_etag = NULL,
                 cloud_ctag = NULL, cloud_hash = NULL, remote_item_id = NULL,
                 remote_parent_id = NULL, updated_at = ?4
             WHERE profile_id = ?1 AND (path = ?2 OR substr(path, 1, length(?3)) = ?3)",
            params![profile_id, path, prefix, now],
        )
        .map_err(|error| format!("Failed marking cloud delete under '{path}': {error}"))?;
    Ok(())
}

pub(crate) fn clear_cloud_state(
    connection: &Connection,
    profile_id: &str,
    path: &str,
    now: i64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE sync_files
             SET cloud_present = 0, cloud_size = 0, cloud_mtime = 0, cloud_etag = NULL,
                 cloud_ctag = NULL, cloud_hash = NULL, remote_item_id = NULL,
                 remote_parent_id = NULL, updated_at = ?3
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, now],
        )
        .map_err(|error| format!("Failed clearing cloud state of '{path}': {error}"))?;
    Ok(())
}
