use super::*;

// The decision table (docs/sync-engine.md section 4). Each side is compared
// with the baseline, never with the other side:
//   cloud changed = cTag differs from the baseline cTag
//   local changed = size or time differs from the baseline local size/time

pub(crate) const PLANNER_ACTION_NONE: &str = "none";
pub(crate) const PLANNER_ACTION_DOWNLOAD: &str = "download";
pub(crate) const PLANNER_ACTION_UPLOAD: &str = "upload";
pub(crate) const PLANNER_ACTION_DELETE_REMOTE: &str = "delete_remote";
pub(crate) const PLANNER_ACTION_DELETE_LOCAL: &str = "delete_local";
pub(crate) const PLANNER_ACTION_CONFLICT: &str = "conflict";
/// Both sides have the item and there is no baseline: compare content.
pub(crate) const PLANNER_ACTION_VERIFY: &str = "verify";
/// Gone from both sides: drop the baseline.
pub(crate) const PLANNER_ACTION_FORGET: &str = "forget";
pub(crate) const PLANNER_ACTION_CREATE_LOCAL_FOLDER: &str = "create_local_folder";
pub(crate) const PLANNER_ACTION_CREATE_REMOTE_FOLDER: &str = "create_remote_folder";

/// Recomputes `desired_action` for every row. The drive root (path `""`),
/// shared references and rows with a per-file issue are never acted on.
pub(crate) fn recompute_sync_file_actions(
    profile_id: &str,
) -> Result<SyncFilePlannerCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let action_expression = format!(
        "CASE
            WHEN path = '' OR is_shared_reference = 1 OR issue_code IS NOT NULL THEN '{none}'
            WHEN is_dir = 0 THEN {file_rule}
            ELSE {folder_rule}
         END",
        none = PLANNER_ACTION_NONE,
        file_rule = PLANNER_FILE_RULE.as_str(),
        folder_rule = PLANNER_FOLDER_RULE,
    );
    // Only rows whose action changes are written, so re-planning after every
    // delta page stays cheap on large drives.
    connection
        .execute(
            &format!(
                "UPDATE sync_files SET desired_action = {action_expression}
                 WHERE profile_id = ?1 AND desired_action != {action_expression}"
            ),
            params![profile_id],
        )
        .map_err(|error| format!("Failed computing sync actions: {error}"))?;
    read_sync_file_planner_counters(profile_id)
}

/// Local changed: size or time differs from the baseline local size/time
/// (a touch also matches; the upload step compares content first).
pub(crate) const LOCAL_CHANGED: &str =
    "(local_size != base_size OR local_mtime_ns != base_local_mtime_ns)";

/// Cloud changed: the content hash (or, without hashes on both sides, the
/// cTag) or the size differs from the baseline cloud version. A baseline
/// with no cloud version recorded (after a conflict copy) counts as changed,
/// so the cloud version is fetched.
pub(crate) const CLOUD_CHANGED: &str = "(CASE
    WHEN cloud_hash IS NOT NULL AND base_hash IS NOT NULL
        THEN cloud_hash != base_hash OR cloud_size != base_cloud_size
    WHEN cloud_ctag IS NOT NULL AND base_ctag IS NOT NULL
        THEN cloud_ctag != base_ctag OR cloud_size != base_cloud_size
    WHEN base_hash IS NULL AND base_ctag IS NULL THEN 1
    ELSE cloud_size != base_cloud_size
END)";

/// Files: the decision table, rows 1 to 12.
pub(crate) static PLANNER_FILE_RULE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "CASE
            WHEN base_present = 0 THEN CASE
                WHEN local_present = 1 AND cloud_present = 1 THEN 'verify'
                WHEN local_present = 1 THEN 'upload'
                WHEN cloud_present = 1 THEN 'download'
                ELSE 'forget'
            END
            ELSE CASE
                WHEN local_present = 1 AND cloud_present = 1 THEN CASE
                    WHEN {lc} AND {cc} THEN 'conflict'
                    WHEN {lc} THEN 'upload'
                    WHEN {cc} THEN 'download'
                    ELSE 'none'
                END
                WHEN local_present = 0 AND cloud_present = 1 THEN CASE
                    WHEN {cc} THEN 'download'
                    ELSE 'delete_remote'
                END
                WHEN local_present = 1 AND cloud_present = 0 THEN CASE
                    WHEN {lc} THEN 'upload'
                    ELSE 'delete_local'
                END
                ELSE 'forget'
            END
        END",
        lc = LOCAL_CHANGED,
        cc = CLOUD_CHANGED,
    )
});

/// Folders: presence only. A folder is deleted on one side only once nothing
/// beneath it is still present on that side, so a delete never takes
/// unsynced content with it. Descendants are found with a range on the path
/// index ('/' sorts just before '0').
pub(crate) const PLANNER_FOLDER_RULE: &str = "CASE
    WHEN base_present = 0 THEN CASE
        WHEN local_present = 1 AND cloud_present = 1 THEN 'verify'
        WHEN local_present = 1 THEN 'create_remote_folder'
        WHEN cloud_present = 1 THEN 'create_local_folder'
        ELSE 'forget'
    END
    ELSE CASE
        WHEN local_present = 1 AND cloud_present = 1 THEN 'none'
        WHEN local_present = 0 AND cloud_present = 1 THEN CASE
            WHEN EXISTS (
                SELECT 1 FROM sync_files AS child
                WHERE child.profile_id = sync_files.profile_id
                  AND child.path > sync_files.path || '/'
                  AND child.path < sync_files.path || '0'
                  AND child.cloud_present = 1
            ) THEN 'none'
            ELSE 'delete_remote'
        END
        WHEN local_present = 1 AND cloud_present = 0 THEN CASE
            WHEN EXISTS (
                SELECT 1 FROM sync_files AS child
                WHERE child.profile_id = sync_files.profile_id
                  AND child.path > sync_files.path || '/'
                  AND child.path < sync_files.path || '0'
                  AND child.local_present = 1
            ) THEN 'none'
            ELSE 'delete_local'
        END
        ELSE 'forget'
    END
END";
