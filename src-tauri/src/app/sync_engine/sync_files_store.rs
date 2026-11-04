// The single owner of `sync_files`: the cloud, local and baseline state of
// every path (docs/sync-engine.md sections 3 and 8). The planner reads it;
// everything that observes or changes a file writes through here.

/// What applying one delta page did.
#[derive(Debug, Default)]
struct DeltaPageSummary {
    recorded: usize,
    deleted: usize,
    skipped_unresolved: usize,
}

/// The planner's view of one row, read when an action is executed.
#[derive(Debug, Clone)]
struct SyncFileRow {
    path: String,
    is_dir: bool,
    remote_item_id: Option<String>,
    cloud_present: bool,
    cloud_size: u64,
    cloud_mtime: i64,
    cloud_etag: Option<String>,
    cloud_ctag: Option<String>,
    cloud_hash: Option<String>,
    base_present: bool,
    base_size: u64,
    base_local_mtime_ns: i64,
    base_etag: Option<String>,
    base_hash: Option<String>,
    base_cloud_size: u64,
}

impl SyncFileRow {
    /// Whether `entry` (the file on disk now) still matches the baseline.
    fn local_matches_baseline(&self, entry: &LocalEntry) -> bool {
        self.base_present
            && entry.size == self.base_size
            && entry.mtime_ns == self.base_local_mtime_ns
    }

    /// The cloud version currently recorded for this path.
    fn cloud_version(&self) -> CloudVersion {
        CloudVersion {
            etag: self.cloud_etag.clone(),
            ctag: self.cloud_ctag.clone(),
            quick_xor_hash: self.cloud_hash.clone(),
            size: self.cloud_size,
        }
    }
}

/// One version of a cloud file, captured when work on it was planned, so the
/// baseline records the version actually transferred even if the cloud
/// changes meanwhile (the next plan then sees that newer change).
#[derive(Debug, Clone, PartialEq)]
struct CloudVersion {
    etag: Option<String>,
    ctag: Option<String>,
    quick_xor_hash: Option<String>,
    size: u64,
}

const SYNC_FILE_ROW_COLUMNS: &str = "path, is_dir, remote_item_id,
    cloud_present, cloud_size, cloud_mtime, cloud_etag, cloud_ctag, cloud_hash,
    base_present, base_size, base_local_mtime_ns, base_etag, base_hash, base_cloud_size";

fn sync_file_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncFileRow> {
    Ok(SyncFileRow {
        path: row.get(0)?,
        is_dir: sql_to_bool(row.get(1)?),
        remote_item_id: row.get(2)?,
        cloud_present: sql_to_bool(row.get(3)?),
        cloud_size: row.get::<_, i64>(4)?.max(0) as u64,
        cloud_mtime: row.get(5)?,
        cloud_etag: row.get(6)?,
        cloud_ctag: row.get(7)?,
        cloud_hash: row.get(8)?,
        base_present: sql_to_bool(row.get(9)?),
        base_size: row.get::<_, i64>(10)?.max(0) as u64,
        base_local_mtime_ns: row.get(11)?,
        base_etag: row.get(12)?,
        base_hash: row.get(13)?,
        base_cloud_size: row.get::<_, i64>(14)?.max(0) as u64,
    })
}

fn read_sync_file_row(profile_id: &str, path: &str) -> Result<Option<SyncFileRow>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            &format!(
                "SELECT {SYNC_FILE_ROW_COLUMNS} FROM sync_files WHERE profile_id = ?1 AND path = ?2"
            ),
            params![profile_id, path],
            sync_file_row_from_sql,
        )
        .optional()
        .map_err(|error| format!("Failed reading sync file row '{path}': {error}"))
}

fn list_sync_file_rows_by_action(profile_id: &str, action: &str) -> Result<Vec<SyncFileRow>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SYNC_FILE_ROW_COLUMNS} FROM sync_files
             WHERE profile_id = ?1 AND desired_action = ?2
             ORDER BY path"
        ))
        .map_err(|error| format!("Failed preparing sync file action query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id, action], sync_file_row_from_sql)
        .map_err(|error| format!("Failed querying sync files by action: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading sync file rows: {error}"))
}

/// Records the local scan: every scanned path is present with its current
/// size and time; every other path is absent locally.
fn apply_local_scan(
    profile_id: &str,
    entries: &HashMap<String, LocalEntry>,
) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting local scan transaction: {error}"))?;
    let now = current_unix_seconds();
    transaction
        .execute(
            "UPDATE sync_files
             SET local_present = 0, local_size = 0, local_mtime_ns = 0
             WHERE profile_id = ?1",
            params![profile_id],
        )
        .map_err(|error| format!("Failed clearing local state: {error}"))?;
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, local_present, local_size, local_mtime_ns, updated_at
                 ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)
                 ON CONFLICT(profile_id, path) DO UPDATE SET
                    is_dir = CASE WHEN cloud_present = 1 THEN is_dir ELSE excluded.is_dir END,
                    local_present = 1,
                    local_size = excluded.local_size,
                    local_mtime_ns = excluded.local_mtime_ns,
                    updated_at = excluded.updated_at",
            )
            .map_err(|error| format!("Failed preparing local scan upsert: {error}"))?;
        for (path, entry) in entries {
            statement
                .execute(params![
                    profile_id,
                    path,
                    bool_to_sql(entry.is_dir),
                    entry.size as i64,
                    entry.mtime_ns,
                    now
                ])
                .map_err(|error| format!("Failed recording local file '{path}': {error}"))?;
        }
    }
    delete_untracked_rows(&transaction, profile_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing local scan: {error}"))
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

const ISSUE_ONENOTE_NOTEBOOK: &str = "onenote_notebook";
const ISSUE_MALWARE_DETECTED: &str = "malware_detected";

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
        move_cloud_state(connection, profile_id, &old_path, &item.path, now)?;
    }

    connection
        .execute(
            "INSERT INTO sync_files (
                profile_id, path, is_dir, is_shared_reference, shared_drive_id, shared_item_id,
                shared_kind, remote_item_id, remote_parent_id, cloud_present, cloud_size,
                cloud_mtime, cloud_etag, cloud_ctag, cloud_hash, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11, ?12, ?13, ?14, ?15)
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

/// Rows with nothing on either side and no baseline carry no information.
fn delete_untracked_rows(connection: &Connection, profile_id: &str) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM sync_files
             WHERE profile_id = ?1 AND cloud_present = 0 AND local_present = 0 AND base_present = 0",
            params![profile_id],
        )
        .map_err(|error| format!("Failed removing untracked rows: {error}"))?;
    Ok(())
}

/// Both sides now hold the same content: cloud `version` and the local file
/// `entry`. Records that as the baseline.
fn commit_baseline(
    profile_id: &str,
    path: &str,
    version: &CloudVersion,
    entry: &LocalEntry,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    write_baseline(&connection, profile_id, path, version, entry)
}

fn write_baseline(
    connection: &Connection,
    profile_id: &str,
    path: &str,
    version: &CloudVersion,
    entry: &LocalEntry,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE sync_files
             SET local_present = 1,
                 local_size = ?3,
                 local_mtime_ns = ?4,
                 base_present = 1,
                 base_size = ?3,
                 base_local_mtime_ns = ?4,
                 base_etag = ?5,
                 base_ctag = ?6,
                 base_hash = ?7,
                 base_cloud_size = ?8,
                 desired_action = 'none',
                 issue_code = NULL,
                 issue_detail = NULL,
                 updated_at = ?9
             WHERE profile_id = ?1 AND path = ?2",
            params![
                profile_id,
                path,
                entry.size as i64,
                entry.mtime_ns,
                version.etag,
                version.ctag,
                version.quick_xor_hash,
                version.size as i64,
                current_unix_seconds()
            ],
        )
        .map_err(|error| format!("Failed recording baseline for '{path}': {error}"))?;
    Ok(())
}

/// An upload finished: OneDrive now holds `uploaded`, made from the local
/// file `entry`. Records the new cloud state and the baseline together.
fn commit_baseline_from_upload(
    profile_id: &str,
    uploaded: &CloudItem,
    entry: &LocalEntry,
) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting upload baseline transaction: {error}"))?;
    upsert_cloud_item(&transaction, profile_id, uploaded, current_unix_seconds())?;
    let version = CloudVersion {
        etag: uploaded.etag.clone(),
        ctag: uploaded.ctag.clone(),
        quick_xor_hash: uploaded.quick_xor_hash.clone(),
        size: uploaded.size,
    };
    write_baseline(&transaction, profile_id, &uploaded.path, &version, entry)?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing upload baseline: {error}"))
}

/// The local file `entry` has just been preserved as a conflict copy. The
/// baseline now matches it locally but holds no cloud version, so the next
/// plan downloads the cloud version over it (decision table row 2).
fn mark_conflict_preserved(profile_id: &str, path: &str, entry: &LocalEntry) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET base_present = 1,
                 base_size = ?3,
                 base_local_mtime_ns = ?4,
                 base_etag = NULL,
                 base_ctag = NULL,
                 base_hash = NULL,
                 base_cloud_size = 0,
                 desired_action = 'none',
                 updated_at = ?5
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, entry.size as i64, entry.mtime_ns, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed recording conflict copy for '{path}': {error}"))?;
    Ok(())
}

/// The item is gone from both sides (or was never there): forget it.
fn forget_sync_file(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET base_present = 0, base_size = 0, base_cloud_size = 0, base_local_mtime_ns = 0,
                 base_etag = NULL, base_ctag = NULL, base_hash = NULL, desired_action = 'none',
                 updated_at = ?3
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed forgetting '{path}': {error}"))?;
    delete_untracked_rows(&connection, profile_id)
}

/// The local file at `path` is gone (deleted to Trash or renamed aside).
fn record_local_absent(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET local_present = 0, local_size = 0, local_mtime_ns = 0, updated_at = ?3
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed recording local removal of '{path}': {error}"))?;
    Ok(())
}

/// A cloud delete of `path` succeeded.
fn record_cloud_absent(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    clear_cloud_state(&connection, profile_id, path, current_unix_seconds())
}

/// A per-file problem that stops this item syncing until it changes (an
/// invalid name, a case collision) without stopping the whole cycle.
fn record_sync_file_issue(
    profile_id: &str,
    path: &str,
    issue_code: &str,
    issue_detail: &str,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    write_sync_file_issue(&connection, profile_id, path, issue_code, issue_detail)
}

fn write_sync_file_issue(
    connection: &Connection,
    profile_id: &str,
    path: &str,
    issue_code: &str,
    issue_detail: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE sync_files
             SET issue_code = ?3, issue_detail = ?4, desired_action = 'none', updated_at = ?5
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, issue_code, issue_detail, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed recording issue for '{path}': {error}"))?;
    Ok(())
}

/// The local file was touched but its content still equals the baseline:
/// only the recorded local size and time move on (no upload).
fn refresh_baseline_local(profile_id: &str, path: &str, entry: &LocalEntry) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET local_present = 1, local_size = ?3, local_mtime_ns = ?4,
                 base_size = ?3, base_local_mtime_ns = ?4, desired_action = 'none', updated_at = ?5
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, entry.size as i64, entry.mtime_ns, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed refreshing baseline of '{path}': {error}"))?;
    Ok(())
}

/// How many files have a baseline (were synced at least once).
fn count_tracked_files(profile_id: &str) -> Result<usize, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT COUNT(1) FROM sync_files
             WHERE profile_id = ?1 AND base_present = 1 AND is_dir = 0",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("Failed counting tracked files: {error}"))
}
