// Records the cloud side of `sync_files` from delta pages: path resolution
// from parent item IDs, renames and moves, deletes, and cloud items that are
// never synced. Part of the `sync_files` store.

const ISSUE_ONENOTE_NOTEBOOK: &str = "onenote_notebook";
const ISSUE_MALWARE_DETECTED: &str = "malware_detected";

/// What applying one delta page did.
#[derive(Debug, Default)]
struct DeltaPageSummary {
    recorded: usize,
    deleted: usize,
    skipped_unresolved: usize,
}

/// Applies one delta page and records its continuation link in the same
/// transaction, so an interrupted cycle resumes exactly after the last page
/// whose items are stored (nothing is lost on pause or quit).
fn apply_delta_page(
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
        upsert_cloud_item(&transaction, profile_id, &item.to_cloud_item(path.clone()), now)?;
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
fn cloud_item_exclusion(
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
    Ok((parent_issue.as_deref() == Some(ISSUE_ONENOTE_NOTEBOOK)).then_some((
        ISSUE_ONENOTE_NOTEBOOK,
        "OneNote notebooks are not synced; open them in OneNote.",
    )))
}

/// The item's path, from its parent's stored path (the drive root is stored
/// as `""`). Falls back to `parentReference.path` when Graph supplies it.
/// `None` when neither is known: the item is skipped, never guessed.
fn resolve_delta_item_path(
    connection: &Connection,
    profile_id: &str,
    item: &GraphDriveItem,
) -> Result<Option<String>, String> {
    if item.root.is_some() {
        return Ok(Some(String::new()));
    }
    let Some(name) = item.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) else {
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
fn resolve_cloud_parent_path(
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

fn upsert_cloud_item(
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
fn move_cloud_state(
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
fn mark_cloud_item_deleted(
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

fn clear_cloud_state(
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

/// OneDrive expired the delta link: forget it and remember when the fresh
/// enumeration started, so items it does not return can be checked one by
/// one afterwards instead of being taken as deleted.
fn begin_delta_resync(profile_id: &str) -> Result<i64, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "INSERT INTO sync_lifecycle_state (profile_id, resync_started_at, updated_at)
             VALUES (?1, ?2, ?2)
             ON CONFLICT(profile_id) DO UPDATE SET
                delta_link = NULL,
                active_delta_next_link = NULL,
                resync_started_at = excluded.resync_started_at,
                updated_at = excluded.updated_at",
            params![profile_id, now],
        )
        .map_err(|error| format!("Failed starting delta resync: {error}"))?;
    Ok(now)
}

fn read_delta_resync_started_at(profile_id: &str) -> Result<Option<i64>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT resync_started_at FROM sync_lifecycle_state WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|error| format!("Failed reading delta resync marker: {error}"))
}

fn finish_delta_resync(profile_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_lifecycle_state SET resync_started_at = NULL WHERE profile_id = ?1",
            params![profile_id],
        )
        .map_err(|error| format!("Failed clearing delta resync marker: {error}"))?;
    Ok(())
}

/// Cloud items recorded before `since` that a fresh enumeration did not
/// return: `(path, remote item id)`.
fn list_cloud_items_unseen_since(profile_id: &str, since: i64) -> Result<Vec<(String, String)>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path, remote_item_id FROM sync_files
             WHERE profile_id = ?1 AND cloud_present = 1 AND remote_item_id IS NOT NULL
               AND cloud_seen_at < ?2 AND path != ''
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing unseen items query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id, since], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| format!("Failed querying unseen items: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading unseen items: {error}"))
}

/// A cloud item confirmed gone (it and anything under it).
fn record_cloud_item_deleted(profile_id: &str, remote_item_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    mark_cloud_item_deleted(&connection, profile_id, remote_item_id, current_unix_seconds())
}

/// Rows at `path` or anywhere under it.
const PATH_OR_DESCENDANT: &str = "(path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')";

/// A cloud item moved from `old_path` to `new_path` and nothing is tracked at
/// the new path yet: the whole record (cloud, local and baseline) moves with
/// it, and the same rename is queued for this device. Returns false when the
/// new path is already in use, so the caller falls back to moving only the
/// cloud state (re-download plus a guarded delete of the old copy).
fn follow_cloud_move(
    connection: &Connection,
    profile_id: &str,
    old_path: &str,
    new_path: &str,
    now: i64,
) -> Result<bool, String> {
    let destination_in_use: bool = connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM sync_files WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT})"),
            params![profile_id, new_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed checking move destination '{new_path}': {error}"))?;
    if destination_in_use {
        return Ok(false);
    }
    let has_local_copy: bool = connection
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM sync_files WHERE profile_id = ?1 AND local_present = 1 AND {PATH_OR_DESCENDANT})"
            ),
            params![profile_id, old_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed checking local copy of '{old_path}': {error}"))?;
    connection
        .execute(
            &format!(
                "UPDATE sync_files SET path = ?3 || substr(path, length(?2) + 1), updated_at = ?4
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}"
            ),
            params![profile_id, old_path, new_path, now],
        )
        .map_err(|error| format!("Failed moving '{old_path}' to '{new_path}': {error}"))?;
    if has_local_copy {
        connection
            .execute(
                "INSERT INTO sync_local_moves (profile_id, old_path, new_path, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(profile_id, old_path) DO UPDATE SET new_path = excluded.new_path",
                params![profile_id, old_path, new_path, now],
            )
            .map_err(|error| format!("Failed queuing local move of '{old_path}': {error}"))?;
    }
    Ok(true)
}

/// A cloud rename waiting to be repeated on this device.
#[derive(Debug, Clone, PartialEq)]
struct PendingLocalMove {
    old_path: String,
    new_path: String,
}

fn list_pending_local_moves(profile_id: &str) -> Result<Vec<PendingLocalMove>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT old_path, new_path FROM sync_local_moves
             WHERE profile_id = ?1 ORDER BY created_at, old_path",
        )
        .map_err(|error| format!("Failed preparing local moves query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| {
            Ok(PendingLocalMove {
                old_path: row.get(0)?,
                new_path: row.get(1)?,
            })
        })
        .map_err(|error| format!("Failed querying local moves: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading local moves: {error}"))
}

fn finish_local_move(profile_id: &str, old_path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_local_moves WHERE profile_id = ?1 AND old_path = ?2",
            params![profile_id, old_path],
        )
        .map_err(|error| format!("Failed finishing local move of '{old_path}': {error}"))?;
    Ok(())
}

/// The rename could not be repeated on disk: the local state and baseline go
/// back to the old path, while the cloud state stays at the new one. The
/// planner then downloads the new path and (through the delete guard)
/// removes the old copy.
fn undo_local_follow(profile_id: &str, old_path: &str, new_path: &str) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting move undo: {error}"))?;
    let now = current_unix_seconds();
    transaction
        .execute(
            &format!(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, local_present, local_size, local_mtime_ns,
                    base_present, base_size, base_cloud_size, base_local_mtime_ns, base_etag,
                    base_ctag, base_hash, updated_at
                 )
                 SELECT profile_id, ?3 || substr(path, length(?2) + 1), is_dir, local_present,
                        local_size, local_mtime_ns, base_present, base_size, base_cloud_size,
                        base_local_mtime_ns, base_etag, base_ctag, base_hash, ?4
                 FROM sync_files
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}
                   AND (local_present = 1 OR base_present = 1)"
            ),
            params![profile_id, new_path, old_path, now],
        )
        .map_err(|error| format!("Failed restoring local state at '{old_path}': {error}"))?;
    transaction
        .execute(
            &format!(
                "UPDATE sync_files
                 SET local_present = 0, local_size = 0, local_mtime_ns = 0,
                     base_present = 0, base_size = 0, base_cloud_size = 0, base_local_mtime_ns = 0,
                     base_etag = NULL, base_ctag = NULL, base_hash = NULL, updated_at = ?3
                 WHERE profile_id = ?1 AND {PATH_OR_DESCENDANT}"
            ),
            params![profile_id, new_path, now],
        )
        .map_err(|error| format!("Failed clearing local state at '{new_path}': {error}"))?;
    transaction
        .execute(
            "DELETE FROM sync_local_moves WHERE profile_id = ?1 AND old_path = ?2",
            params![profile_id, old_path],
        )
        .map_err(|error| format!("Failed dropping local move of '{old_path}': {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing move undo: {error}"))
}
