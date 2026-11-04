use super::*;

// Held conflicts and the per-file problems the UI's Sync problems tab lists
// (docs/sync-engine.md section 6.2): recording the user's choice, and the
// problems themselves. Part of the `sync_files` store.

/// Issue codes for a file that differs on both sides. The file is held (not
/// synced, not copied) until the user picks a resolution.
pub(crate) const ISSUE_CONFLICT_BOTH_CHANGED: &str = "conflict_both_changed";
pub(crate) const ISSUE_CONFLICT_BOTH_NEW: &str = "conflict_both_new";
pub(crate) const CONFLICT_ISSUE_PREFIX: &str = "conflict_";

pub(crate) const RESOLUTION_KEEP_LOCAL: &str = "keep_local";
pub(crate) const RESOLUTION_KEEP_CLOUD: &str = "keep_cloud";
pub(crate) const RESOLUTION_KEEP_BOTH: &str = "keep_both";

/// A held conflict with the user's choice, and the local file's time when
/// they chose (a later edit cancels the choice and holds the file again).
pub(crate) struct ConflictResolution {
    pub(crate) row: SyncFileRow,
    pub(crate) resolution: String,
    pub(crate) chosen_local_mtime_ns: Option<i64>,
}

/// Records the user's choice for held conflicts. Returns how many paths took
/// it (paths that are no longer held are ignored).
pub(crate) fn set_conflict_resolution(
    profile_id: &str,
    paths: &[String],
    resolution: &str,
) -> Result<usize, String> {
    if ![
        RESOLUTION_KEEP_LOCAL,
        RESOLUTION_KEEP_CLOUD,
        RESOLUTION_KEEP_BOTH,
    ]
    .contains(&resolution)
    {
        return Err(format!("Unknown conflict resolution '{resolution}'"));
    }
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed starting resolution transaction: {error}"))?;
    let mut updated = 0;
    for path in paths {
        updated += transaction
            .execute(
                "UPDATE sync_files
                 SET resolution = ?3,
                     resolution_local_mtime_ns = CASE WHEN local_present = 1 THEN local_mtime_ns ELSE NULL END,
                     updated_at = ?4
                 WHERE profile_id = ?1 AND path = ?2
                   AND substr(issue_code, 1, length(?5)) = ?5",
                params![profile_id, path, resolution, current_unix_seconds(), CONFLICT_ISSUE_PREFIX],
            )
            .map_err(|error| format!("Failed recording resolution for '{path}': {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed committing resolutions: {error}"))?;
    Ok(updated)
}

/// The setting's automatic choice for one held conflict, recorded against
/// the local file's time as it is now.
pub(crate) fn set_automatic_conflict_resolution(
    profile_id: &str,
    path: &str,
    resolution: &str,
    local_mtime_ns: i64,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET resolution = ?3, resolution_local_mtime_ns = ?4, updated_at = ?5
             WHERE profile_id = ?1 AND path = ?2 AND substr(issue_code, 1, length(?6)) = ?6",
            params![
                profile_id,
                path,
                resolution,
                local_mtime_ns,
                current_unix_seconds(),
                CONFLICT_ISSUE_PREFIX
            ],
        )
        .map_err(|error| format!("Failed recording automatic resolution for '{path}': {error}"))?;
    Ok(())
}

pub(crate) fn list_conflict_resolutions(
    profile_id: &str,
) -> Result<Vec<ConflictResolution>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SYNC_FILE_ROW_COLUMNS}, resolution, resolution_local_mtime_ns FROM sync_files
             WHERE profile_id = ?1 AND resolution IS NOT NULL
               AND substr(issue_code, 1, length(?2)) = ?2
             ORDER BY path"
        ))
        .map_err(|error| format!("Failed preparing resolution query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id, CONFLICT_ISSUE_PREFIX], |row| {
            Ok(ConflictResolution {
                row: sync_file_row_from_sql(row)?,
                resolution: row.get(17)?,
                chosen_local_mtime_ns: row.get(18)?,
            })
        })
        .map_err(|error| format!("Failed querying resolutions: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading resolutions: {error}"))
}

/// The file was changed again after the user chose: drop the choice and keep
/// it held so they decide on what is there now.
pub(crate) fn withdraw_conflict_resolution(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files SET resolution = NULL, resolution_local_mtime_ns = NULL, updated_at = ?3
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed withdrawing resolution for '{path}': {error}"))?;
    Ok(())
}

/// The problem on `path` is resolved: it syncs normally again.
pub(crate) fn clear_sync_file_issue(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files
             SET issue_code = NULL, issue_detail = NULL, resolution = NULL,
                 resolution_local_mtime_ns = NULL, updated_at = ?3
             WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path, current_unix_seconds()],
        )
        .map_err(|error| format!("Failed clearing issue for '{path}': {error}"))?;
    Ok(())
}

/// One per-file problem shown in the UI's Sync problems tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProblem {
    pub path: String,
    pub code: String,
    pub detail: String,
    pub is_conflict: bool,
    pub resolution: Option<String>,
    pub local_size: Option<u64>,
    pub local_modified_at: Option<String>,
    pub cloud_size: Option<u64>,
    pub cloud_modified_at: Option<String>,
    /// For a failed download: the id `retry_failed_download` takes.
    pub retry_id: Option<String>,
}

pub(crate) fn nanos_to_rfc3339(value: i64) -> String {
    unix_seconds_to_rfc3339(value / 1_000_000_000)
}

pub(crate) fn list_file_problems(profile_id: &str) -> Result<Vec<SyncProblem>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path, issue_code, COALESCE(issue_detail, ''), resolution,
                    local_present, local_size, local_mtime_ns,
                    cloud_present, cloud_size, cloud_mtime
             FROM sync_files
             WHERE profile_id = ?1 AND issue_code IS NOT NULL AND issue_code != ?2
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing problems query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id, ISSUE_EXCLUDED_BY_RULE], |row| {
            let code: String = row.get(1)?;
            let local_present = sql_to_bool(row.get(4)?);
            let cloud_present = sql_to_bool(row.get(7)?);
            Ok(SyncProblem {
                path: row.get(0)?,
                is_conflict: code.starts_with(CONFLICT_ISSUE_PREFIX),
                code,
                detail: row.get(2)?,
                resolution: row.get(3)?,
                local_size: local_present
                    .then(|| row.get::<_, i64>(5).map(|value| value.max(0) as u64))
                    .transpose()?,
                local_modified_at: local_present
                    .then(|| row.get::<_, i64>(6).map(nanos_to_rfc3339))
                    .transpose()?,
                cloud_size: cloud_present
                    .then(|| row.get::<_, i64>(8).map(|value| value.max(0) as u64))
                    .transpose()?,
                cloud_modified_at: cloud_present
                    .then(|| row.get::<_, i64>(9).map(unix_seconds_to_rfc3339))
                    .transpose()?,
                retry_id: None,
            })
        })
        .map_err(|error| format!("Failed querying problems: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading problems: {error}"))
}

/// Downloads that failed for good, as problems the user can retry.
pub(crate) fn list_failed_download_problems(profile_id: &str) -> Result<Vec<SyncProblem>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT id, path, COALESCE(last_error, 'Download failed'), remote_size
             FROM sync_jobs
             WHERE profile_id = ?1 AND direction = ?2 AND state = ?3
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing failed downloads query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_FAILED_TERMINAL
            ],
            |row| {
                let job_id: i64 = row.get(0)?;
                Ok(SyncProblem {
                    path: row.get(1)?,
                    code: "download_failed".to_string(),
                    detail: row.get(2)?,
                    is_conflict: false,
                    resolution: None,
                    local_size: None,
                    local_modified_at: None,
                    cloud_size: Some(row.get::<_, i64>(3)?.max(0) as u64),
                    cloud_modified_at: None,
                    retry_id: Some(format!("job-{job_id}")),
                })
            },
        )
        .map_err(|error| format!("Failed querying failed downloads: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading failed downloads: {error}"))
}
